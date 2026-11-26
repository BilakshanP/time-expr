//! Parser: source text -> [`Expr`] tree or [`Program`].
//!
//! Grammar (whitespace-insensitive around tokens):
//!
//! ```text
//! program = statement (";" statement)* ";"?
//! statement = assign | expr
//! assign  = "$" ident "=" expr
//!
//! expr    = sum
//! sum     = product (("+" | "-") product)*
//! product = unary   (("*" | "/") unary)*
//! unary   = atom | "-" unary
//! atom    = literal | var_ref | "(" expr ")"
//! var_ref = "$" ident
//! literal = duration | timepoint | scalar
//! ident   = [a-zA-Z_][a-zA-Z0-9_]*
//! ```
//!
//! Literals are disambiguated by shape:
//! - **duration** carries unit letters: `1d2h3m4s`, components omittable,
//!   optional leading `-` (e.g. `5h`, `-2h30m`, `0s`).
//! - **timepoint** uses colons, meaning set by field count:
//!   `HH:MM`, `HH:MM:SS`, or `DD:HH:MM:SS` (day may be signed / multi-digit).
//! - **scalar** is a bare number: `3`, `2.5`.

use chumsky::{error::Rich, prelude::*};

use crate::{
    ast::{Expr, Program, Statement},
    error::ParseError,
    helpers::{checked_dhms_to_seconds, HOURS_PER_DAY, MINUTES_PER_HOUR, SECONDS_PER_MINUTE},
    op::{BinOp, UnaryOp},
    value::Value,
};

type Extra<'src> = extra::Err<Rich<'src, char>>;

// =========================================================================
// Comment stripping
// =========================================================================

/// Strip `#` line comments and normalize newlines to `;` for the program
/// parser.
///
/// Each line's comment (from `#` to end of line) is removed, and blank /
/// comment-only lines are dropped. Remaining lines are joined with ` ; ` so
/// the program parser's existing `;`-separation handles both semicolons and
/// line breaks as statement separators.
pub fn strip_comments(src: &str) -> String {
    src.lines()
        .map(|line| {
            match line.find('#') {
                Some(pos) => &line[..pos],
                None => line,
            }
            .trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ; ")
}

// =========================================================================
// Public API — single expression
// =========================================================================

/// Parse a complete source string into an [`Expr`] tree (single expression).
///
/// For programs with variables and `;`, use [`parse_program`] instead.
/// Line comments (`# ...`) are stripped before parsing.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let clean = strip_comments(src);
    parse_rich(&clean).map_err(collapse_errors)
}

/// Parse a source string into an [`Expr`], returning span-rich errors.
pub fn parse_rich(src: &str) -> Result<Expr, Vec<Rich<'static, char>>> {
    let clean = strip_comments(src);
    expr_parser()
        .then_ignore(end())
        .padded()
        .parse(&clean)
        .into_result()
        .map_err(|errs| errs.into_iter().map(|e| e.into_owned()).collect())
}

// =========================================================================
// Public API — program (statements + variables)
// =========================================================================

/// Parse a source string into a [`Program`] (statements separated by `;`).
/// Line comments (`# ...`) are stripped before parsing.
pub fn parse_program(src: &str) -> Result<Program, ParseError> {
    let clean = strip_comments(src);
    parse_program_rich(&clean).map_err(collapse_errors)
}

/// Parse a source string into a [`Program`], returning span-rich errors.
pub fn parse_program_rich(src: &str) -> Result<Program, Vec<Rich<'static, char>>> {
    let clean = strip_comments(src);
    program_parser()
        .parse(&clean)
        .into_result()
        .map_err(|errs| errs.into_iter().map(|e| e.into_owned()).collect())
}

// =========================================================================
// Public API — reusable parser handle
// =========================================================================

/// A reusable, boxed expression parser.
pub type ExprParser<'src> = Boxed<'src, 'src, &'src str, Expr, Extra<'src>>;

/// Build the expression parser once, as a reusable boxed handle.
///
/// # Example (REPL-style reuse)
/// ```
/// use time_expr::parser::{parser, run};
///
/// let p = parser();
/// for line in ["2h + 3h", "6h / 2h"] {
///     let value = run(&p, line).unwrap().eval(&Default::default()).unwrap();
///     println!("{value}");
/// }
/// ```
pub fn parser<'src>() -> ExprParser<'src> {
    expr_parser().then_ignore(end()).padded().boxed()
}

/// Run a prebuilt [`parser`] over one input, collapsing errors into a
/// [`ParseError`] (same contract as [`parse`]).
pub fn run<'a>(parser: &ExprParser<'a>, src: &'a str) -> Result<Expr, ParseError> {
    parser
        .parse(src)
        .into_result()
        .map_err(|errs| collapse_errors(errs.into_iter().map(|e| e.into_owned()).collect()))
}

// =========================================================================
// Error collapsing helper
// =========================================================================

fn collapse_errors(errs: Vec<Rich<'static, char>>) -> ParseError {
    if errs.is_empty() {
        ParseError::unexpected_eof()
    } else {
        ParseError::new(errs.iter().map(|e| e.to_string()))
    }
}

// =========================================================================
// Expression parser (the core recursive grammar)
// =========================================================================

fn expr_parser<'src>() -> impl Parser<'src, &'src str, Expr, Extra<'src>> + Clone {
    recursive(|expr| {
        // --- primitive: a bounded, overflow-checked unsigned integer -----

        let uint = one_of('0'..='9')
            .repeated()
            .at_least(1)
            .to_slice()
            .try_map(|s: &str, span| {
                s.parse::<i64>()
                    .map_err(|_| Rich::custom(span, format!("number '{s}' is too large")))
            });

        // --- scalar ------------------------------------------------------

        let digits = one_of('0'..='9').repeated().at_least(1);
        let scalar = digits
            .clone()
            .then(just('.').then(digits).or_not())
            .to_slice()
            .try_map(|s: &str, span| {
                s.parse::<f64>()
                    .map(Value::Scalar)
                    .map_err(|_| Rich::custom(span, format!("invalid number '{s}'")))
            });

        // --- duration ----------------------------------------------------

        let unit_part = {
            let uint = uint.clone();
            move |unit: char| uint.clone().then_ignore(just(unit))
        };

        let duration = just('-')
            .or_not()
            .then(
                unit_part('d')
                    .or_not()
                    .then(unit_part('h').or_not())
                    .then(unit_part('m').or_not())
                    .then(unit_part('s').or_not()),
            )
            .try_map(|(sign, (((d, h), m), s)), span| {
                if d.is_none() && h.is_none() && m.is_none() && s.is_none() {
                    return Err(Rich::custom(span, "empty duration"));
                }
                let secs = checked_dhms_to_seconds(
                    d.unwrap_or(0),
                    h.unwrap_or(0),
                    m.unwrap_or(0),
                    s.unwrap_or(0),
                )
                .ok_or_else(|| Rich::custom(span, "duration is too large"))?;

                let secs = if sign.is_some() {
                    secs.checked_neg()
                        .ok_or_else(|| Rich::custom(span, "duration is too large"))?
                } else {
                    secs
                };

                Ok(Value::Duration(crate::value::Duration::from_secs(secs)))
            });

        // --- timepoint ---------------------------------------------------

        let field = just('-')
            .or_not()
            .then(uint.clone())
            .map(|(sign, n)| if sign.is_some() { -n } else { n });

        let timepoint = field
            .separated_by(just(':'))
            .at_least(2)
            .at_most(4)
            .collect::<Vec<i64>>()
            .validate(|parts, extra, emitter| {
                let (day, h, m, s) = match parts.as_slice() {
                    [h, m] => (0, *h, *m, 0),
                    [h, m, s] => (0, *h, *m, *s),
                    [d, h, m, s] => (*d, *h, *m, *s),
                    _ => unreachable!("timepoint field count is bounded to 2..=4"),
                };

                let hours_max = HOURS_PER_DAY as i64;
                let mins_max = MINUTES_PER_HOUR as i64;
                let secs_max = SECONDS_PER_MINUTE as i64;
                if !(0..hours_max).contains(&h) {
                    emitter.emit(Rich::custom(
                        extra.span(),
                        format!("hour {h} out of range (0..{hours_max})"),
                    ));
                } else if !(0..mins_max).contains(&m) {
                    emitter.emit(Rich::custom(
                        extra.span(),
                        format!("minute {m} out of range (0..{mins_max})"),
                    ));
                } else if !(0..secs_max).contains(&s) {
                    emitter.emit(Rich::custom(
                        extra.span(),
                        format!("second {s} out of range (0..{secs_max})"),
                    ));
                }

                Value::TimePoint(crate::value::TimePoint::from_dhms_normalized(
                    day,
                    h.rem_euclid(hours_max) as u32,
                    m.rem_euclid(mins_max) as u32,
                    s.rem_euclid(secs_max) as u32,
                ))
            });

        // --- variable reference ------------------------------------------

        let var_ref = just('$')
            .ignore_then(text::ident())
            .map(|name: &str| Expr::Var(name.to_string()));

        // Order matters: timepoint (colons) and duration (unit letters) are
        // tried before scalar (a bare number), the fallback shape.
        let literal = choice((timepoint, duration, scalar)).map(Expr::Literal);

        // --- atoms, grouping, precedence ---------------------------------

        let atom = literal
            .or(var_ref)
            .or(expr.delimited_by(just('('), just(')')))
            .padded();

        let op = |c: char, binop: BinOp| just(c).padded().to(binop);

        let unary = recursive(|unary| {
            atom.clone().or(just('-')
                .padded()
                .ignore_then(unary)
                .map(|operand| Expr::unary(UnaryOp::Neg, operand)))
        });

        let product = unary.clone().foldl(
            choice((op('*', BinOp::Mul), op('/', BinOp::Div)))
                .then(unary)
                .repeated(),
            |lhs, (binop, rhs)| Expr::binary(binop, lhs, rhs),
        );

        product.clone().foldl(
            choice((op('+', BinOp::Add), op('-', BinOp::Sub)))
                .then(product)
                .repeated(),
            |lhs, (binop, rhs)| Expr::binary(binop, lhs, rhs),
        )
    })
}

// =========================================================================
// Program parser (statements + variables + semicolons)
// =========================================================================

fn program_parser<'src>() -> impl Parser<'src, &'src str, Program, Extra<'src>> {
    let expr = expr_parser();

    // $ident = expr
    let assign = just('$')
        .ignore_then(text::ident())
        .then_ignore(just('=').padded())
        .then(expr.clone())
        .map(|(name, e): (&str, Expr)| Statement::Assign {
            name: name.to_string(),
            expr: e,
        });

    // A statement is either an assignment or a bare expression.
    // Try assign first since `$x = ...` starts with `$` which var_ref would
    // also match; chumsky backtracks if the `= expr` part fails.
    let stmt = assign.or(expr.map(Statement::Expr));

    stmt.separated_by(just(';').padded())
        .allow_trailing()
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|statements| Program { statements })
        .then_ignore(end())
        .padded()
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Env;
    use crate::value::Value;

    // Parse then evaluate, returning the resulting Value as a string via Display.
    fn eval_str(src: &str) -> String {
        parse(src)
            .unwrap()
            .eval(&Env::new())
            .unwrap()
            .to_string()
    }

    fn parsed_value(src: &str) -> Value {
        parse(src).unwrap().eval(&Env::new()).unwrap()
    }

    // --- literals --------------------------------------------------------

    #[test]
    fn parse_scalar() {
        assert_eq!(parsed_value("3"), Value::Scalar(3.0));
        assert_eq!(parsed_value("2.5"), Value::Scalar(2.5));
    }

    #[test]
    fn parse_duration_full_and_partial() {
        assert_eq!(eval_str("1d2h3m4s"), "1d2h3m4s");
        assert_eq!(eval_str("5h"), "5h");
        assert_eq!(eval_str("1d4s"), "1d4s");
        assert_eq!(eval_str("0s"), "0s");
    }

    #[test]
    fn parse_duration_negative() {
        assert_eq!(eval_str("-2h30m"), "-2h30m");
    }

    #[test]
    fn parse_timepoint_field_counts() {
        assert_eq!(eval_str("08:00"), "08:00");
        assert_eq!(eval_str("08:00:30"), "08:00:30");
        assert_eq!(eval_str("1:14:30:00"), "1:14:30:00");
    }

    // --- precedence & grouping ------------------------------------------

    #[test]
    fn unary_minus_negates_duration_and_scalar() {
        assert_eq!(eval_str("-2h30m"), "-2h30m");
        assert_eq!(eval_str("-(2h)"), "-2h");
        assert_eq!(eval_str("-2.5"), "-2.5");
    }

    #[test]
    fn double_unary_minus_cancels() {
        assert_eq!(eval_str("--2h"), "2h");
    }

    #[test]
    fn unary_binds_tighter_than_multiplication() {
        assert_eq!(eval_str("3 * -2h"), "-6h");
    }

    #[test]
    fn unary_minus_on_group() {
        assert_eq!(eval_str("-(5h - 2h)"), "-3h");
    }

    #[test]
    fn binary_minus_not_confused_with_unary() {
        assert_eq!(eval_str("8h - 24h"), "-16h");
        assert_eq!(eval_str("8h-24h"), "-16h");
        assert_eq!(eval_str("2h - -3h"), "5h");
    }

    #[test]
    fn unary_minus_on_timepoint_is_type_error() {
        let err = parse("-(08:00)")
            .unwrap()
            .eval(&Env::new())
            .unwrap_err();
        assert_eq!(
            err,
            crate::error::EvalError::InvalidUnaryOperation {
                op: crate::op::UnaryOp::Neg,
                operand: "TimePoint",
            }
        );
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        assert_eq!(eval_str("2h + 3h * 2"), "8h");
    }

    #[test]
    fn parens_override_precedence() {
        assert_eq!(eval_str("(2h + 3h) * 2"), "10h");
    }

    #[test]
    fn left_associative_subtraction() {
        assert_eq!(eval_str("5h - 2h - 1h"), "2h");
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(eval_str("  2h   +   3h  "), "5h");
    }

    // --- the motivating expression --------------------------------------

    #[test]
    fn nested_timepoint_expression() {
        assert_eq!(eval_str("(17:48 - 10:10) + (16:41 - 08:19)"), "16h");
    }

    // --- errors ----------------------------------------------------------

    #[test]
    fn empty_input_is_error() {
        assert!(parse("").is_err());
    }

    #[test]
    fn trailing_garbage_is_error() {
        assert!(parse("2h +").is_err());
        assert!(parse("2h 3h").is_err());
    }

    #[test]
    fn out_of_range_timepoint_reports_precise_field() {
        let err = parse("25:00").unwrap_err();
        assert!(
            err.messages
                .iter()
                .any(|m| m.contains("hour 25 out of range")),
            "got: {:?}",
            err.messages
        );

        let err = parse("14:70").unwrap_err();
        assert!(
            err.messages
                .iter()
                .any(|m| m.contains("minute 70 out of range")),
            "got: {:?}",
            err.messages
        );
    }

    #[test]
    fn parse_error_carries_messages() {
        let err = parse("2h +").unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(err.len(), err.messages.len());
        assert!(err.messages.iter().all(|m| !m.is_empty()));
    }

    // --- round-trip: Display output must parse back to the same value ----

    #[test]
    fn display_output_round_trips() {
        for src in [
            "16h", "-16h", "0s", "2h15m", "1d2h3m4s", "-2h30m", "-1d", "08:00", "08:00:30",
            "1:14:30:00", "-1:23:00:00", "3", "-2.5",
        ] {
            let v1 = parse(src).unwrap().eval(&Env::new()).unwrap();
            let shown = v1.to_string();
            let v2 = parse(&shown)
                .unwrap_or_else(|e| panic!("re-parsing Display output {shown:?} failed: {e}"))
                .eval(&Env::new())
                .unwrap();
            assert_eq!(v1, v2, "round-trip mismatch: {src:?} -> {shown:?}");
        }
    }

    // --- overflow robustness ---------------------------------------------

    #[test]
    fn oversized_number_errors_not_panics() {
        assert!(parse("99999999999999999999d").is_err());
        assert!(parse("99999999999999999999:00").is_err());
    }

    #[test]
    fn oversized_bare_scalar_is_allowed() {
        assert!(parse("99999999999999999999").is_ok());
    }

    #[test]
    fn duration_component_overflow_errors() {
        assert!(parse("9000000000000000d").is_err());
    }

    #[test]
    fn large_but_valid_duration_parses() {
        assert_eq!(eval_str("1000000d"), "1000000d");
    }

    // --- variables and programs ------------------------------------------

    fn prog_eval(src: &str) -> String {
        let prog = parse_program(src).unwrap();
        let mut env = Env::new();
        prog.eval(&mut env).unwrap().unwrap().to_string()
    }

    #[test]
    fn var_assign_and_ref() {
        assert_eq!(prog_eval("$x = 2h; $x"), "2h");
    }

    #[test]
    fn var_in_expression() {
        assert_eq!(prog_eval("$x = 2h; $x + 3h"), "5h");
    }

    #[test]
    fn chained_vars() {
        assert_eq!(prog_eval("$x = 2h + 2h; $y = $x / 2; $y"), "2h");
    }

    #[test]
    fn assignment_is_last_statement() {
        // No trailing bare expression — assignment value is the result.
        assert_eq!(prog_eval("$x = 2h + 2h; $y = $x / 2"), "2h");
    }

    #[test]
    fn single_assignment() {
        assert_eq!(prog_eval("$x = 5h"), "5h");
    }

    #[test]
    fn trailing_semicolon() {
        assert_eq!(prog_eval("$x = 5h;"), "5h");
    }

    #[test]
    fn bare_expr_in_program() {
        assert_eq!(prog_eval("2h + 3h"), "5h");
    }

    #[test]
    fn undefined_variable_errors() {
        let prog = parse_program("$x + 1h").unwrap();
        let mut env = Env::new();
        assert_eq!(
            prog.eval(&mut env),
            Err(crate::error::EvalError::UndefinedVariable {
                name: "x".to_string()
            })
        );
    }

    #[test]
    fn var_ref_in_single_expr_parse() {
        // The single-expression parser should also handle $var.
        let expr = parse("$x").unwrap();
        let mut env = Env::new();
        env.set("x".to_string(), Value::Scalar(42.0));
        assert_eq!(expr.eval(&env).unwrap(), Value::Scalar(42.0));
    }

    #[test]
    fn existing_parse_still_works() {
        assert_eq!(
            parse("2h + 3h")
                .unwrap()
                .eval(&Env::new())
                .unwrap()
                .to_string(),
            "5h"
        );
    }

    // --- comments --------------------------------------------------------

    #[test]
    fn line_comment_stripped() {
        assert_eq!(eval_str("2h + 3h # this is a comment"), "5h");
    }

    #[test]
    fn full_line_comment_in_program() {
        let src = "\
# Calculate total hours
$x = 2h + 3h
# Double it
$y = $x * 2
$y";
        assert_eq!(prog_eval(src), "10h");
    }

    #[test]
    fn inline_and_full_line_comments() {
        let src = "$x = 8h # workday\n$x - 1h # minus lunch";
        assert_eq!(prog_eval(src), "7h");
    }

    #[test]
    fn comment_only_lines_skipped() {
        let src = "# just a comment\n2h + 3h";
        assert_eq!(prog_eval(src), "5h");
    }

    // --- multiline scripts -----------------------------------------------

    #[test]
    fn multiline_script_newlines_as_separators() {
        let src = "\
$start = 09:00
$end = 17:30
$end - $start";
        assert_eq!(prog_eval(src), "8h30m");
    }
}
