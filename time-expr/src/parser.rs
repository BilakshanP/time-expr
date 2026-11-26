//! Parser: source text -> [`Expr`] tree.
//!
//! Grammar (whitespace-insensitive around tokens):
//!
//! ```text
//! expr    = sum
//! sum     = product (("+" | "-") product)*
//! product = atom    (("*" | "/") atom)*
//! atom    = literal | "(" expr ")"
//! literal = duration | timepoint | scalar
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
    ast::Expr,
    error::ParseError,
    helpers::{checked_dhms_to_seconds, HOURS_PER_DAY, MINUTES_PER_HOUR, SECONDS_PER_MINUTE},
    op::{BinOp, UnaryOp},
    value::Value,
};

type Extra<'src> = extra::Err<Rich<'src, char>>;

/// Parse a complete source string into an [`Expr`] tree.
///
/// Collapses chumsky's diagnostics into a [`ParseError`] holding the reported
/// problems as messages. The parser stops at the first error (no recovery is
/// configured), so this is normally a single message. For span-aware reporting
/// (e.g. via the `diagnostics` feature), use [`parse_rich`] instead, which
/// preserves the full error set and their spans.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    parse_rich(src).map_err(|errs| {
        if errs.is_empty() {
            ParseError::unexpected_eof()
        } else {
            ParseError::new(errs.iter().map(|e| e.to_string()))
        }
    })
}

/// Parse a source string, returning the full set of chumsky [`Rich`] errors on
/// failure (with spans intact) so callers can render rich diagnostics.
///
/// The returned errors are `'static` (owned), so they outlive `src`.
pub fn parse_rich(src: &str) -> Result<Expr, Vec<Rich<'static, char>>> {
    expr_parser()
        .parse(src)
        .into_result()
        .map_err(|errs| errs.into_iter().map(|e| e.into_owned()).collect())
}

/// A reusable, boxed expression parser.
///
/// [`parse`]/[`parse_rich`] rebuild the combinator tree on every call, which is
/// fine for one-shot use but wasteful in a loop. A long-lived consumer such as
/// a REPL should build the parser once with [`parser`] and run it per line via
/// [`run`], avoiding repeated construction.
pub type ExprParser<'src> = Boxed<'src, 'src, &'src str, Expr, Extra<'src>>;

/// Build the expression parser once, as a reusable boxed handle.
///
/// # Example (REPL-style reuse)
/// ```
/// use time_expr::parser::{parser, run};
///
/// let p = parser();
/// for line in ["2h + 3h", "6h / 2h"] {
///     let value = run(&p, line).unwrap().eval().unwrap();
///     println!("{value}");
/// }
/// ```
pub fn parser<'src>() -> ExprParser<'src> {
    expr_parser().boxed()
}

/// Run a prebuilt [`parser`] over one input, collapsing errors into a
/// [`ParseError`] (same contract as [`parse`]).
pub fn run<'a>(parser: &ExprParser<'a>, src: &'a str) -> Result<Expr, ParseError> {
    parser.parse(src).into_result().map_err(|errs| {
        if errs.is_empty() {
            ParseError::unexpected_eof()
        } else {
            ParseError::new(errs.iter().map(|e| e.to_string()))
        }
    })
}

/// The top-level expression parser, trimming surrounding whitespace and
/// requiring the whole input to be consumed.
fn expr_parser<'src>() -> impl Parser<'src, &'src str, Expr, Extra<'src>> {
    recursive(|expr| {
        // --- primitive: a bounded, overflow-checked unsigned integer -----

        // A run of one or more ASCII digits, parsed to i64. Unlike
        // `text::int`, this accepts leading zeros (e.g. `08`), which timepoint
        // clock fields require. Overflowing i64 is a clean error, not a panic.
        let uint = one_of('0'..='9')
            .repeated()
            .at_least(1)
            .to_slice()
            .try_map(|s: &str, span| {
                s.parse::<i64>()
                    .map_err(|_| Rich::custom(span, format!("number '{s}' is too large")))
            });

        // --- scalar ------------------------------------------------------

        // An integer or decimal number, e.g. `3` or `2.5`. Parsed to f64 via a
        // fallible map so a malformed/huge literal errors instead of panicking.
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

        // A `<number><unit>` chunk such as `2h`, parsed to its i64 magnitude.
        let unit_part = {
            let uint = uint.clone();
            move |unit: char| uint.clone().then_ignore(just(unit))
        };

        // Optional leading '-' then at least one of d/h/m/s in order, each
        // optional. e.g. `1d2h3m4s`, `5h`, `-2h30m`, `0s`. Component combination
        // is overflow-checked.
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
                // Require at least one component so a bare number isn't a
                // zero-length duration.
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

        // A single colon-separated field: optional '-' then digits, as i64.
        let field = just('-')
            .or_not()
            .then(uint.clone())
            .map(|(sign, n)| if sign.is_some() { -n } else { n });

        // 2, 3, or 4 colon-separated fields. Structural parse only (always
        // succeeds for a well-shaped field list); range validation is applied
        // via `validate` *after* `choice` commits to this shape, so an
        // out-of-range value reports a precise error instead of being
        // out-competed by the scalar branch inside `choice`.
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
                    // at_least(2)/at_most(4) guarantees 2..=4 fields.
                    _ => unreachable!("timepoint field count is bounded to 2..=4"),
                };

                // Hours/mins/secs within a day must be canonical; day is free.
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

                // On a range error we still return a (wrapped) value so parsing
                // continues; the emitted error fails the overall parse anyway.
                Value::TimePoint(crate::value::TimePoint::from_dhms_normalized(
                    day,
                    h.rem_euclid(hours_max) as u32,
                    m.rem_euclid(mins_max) as u32,
                    s.rem_euclid(secs_max) as u32,
                ))
            });

        // Order matters: timepoint (colons) and duration (unit letters) are
        // tried before scalar (a bare number), the fallback shape.
        let literal = choice((timepoint, duration, scalar)).map(Expr::Literal);

        // --- atoms, grouping, precedence ---------------------------------

        let atom = literal.or(expr.delimited_by(just('('), just(')'))).padded();

        let op = |c: char, binop: BinOp| just(c).padded().to(binop);

        // unary: either an atom directly, or a prefix `-` applied to a unary.
        // Trying `atom` first lets a signed-day timepoint literal (`-1:23:00:00`,
        // whose leading `-` belongs to the day field) win before the `-` is
        // interpreted as a negation operator. Binds tighter than `*`/`/`.
        let unary = recursive(|unary| {
            atom.clone().or(just('-')
                .padded()
                .ignore_then(unary)
                .map(|operand| Expr::unary(UnaryOp::Neg, operand)))
        });

        // product: * and / bind tighter than + and -; left-associative.
        let product = unary.clone().foldl(
            choice((op('*', BinOp::Mul), op('/', BinOp::Div)))
                .then(unary)
                .repeated(),
            |lhs, (binop, rhs)| Expr::binary(binop, lhs, rhs),
        );

        // sum: + and -; left-associative.
        product.clone().foldl(
            choice((op('+', BinOp::Add), op('-', BinOp::Sub)))
                .then(product)
                .repeated(),
            |lhs, (binop, rhs)| Expr::binary(binop, lhs, rhs),
        )
    })
    .then_ignore(end())
    .padded()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    // Parse then evaluate, returning the resulting Value as a string via Display.
    fn eval_str(src: &str) -> String {
        parse(src).unwrap().eval().unwrap().to_string()
    }

    fn parsed_value(src: &str) -> Value {
        parse(src).unwrap().eval().unwrap()
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
        // 3 * -2h = -6h, not -(3*2h) (same value here, but checks binding)
        assert_eq!(eval_str("3 * -2h"), "-6h");
    }

    #[test]
    fn unary_minus_on_group() {
        assert_eq!(eval_str("-(5h - 2h)"), "-3h");
    }

    #[test]
    fn binary_minus_not_confused_with_unary() {
        // `8h - 24h` is subtraction, not `8h` then unary `-24h`.
        assert_eq!(eval_str("8h - 24h"), "-16h");
        assert_eq!(eval_str("8h-24h"), "-16h");
        // mixed: binary minus followed by unary minus
        assert_eq!(eval_str("2h - -3h"), "5h");
    }

    #[test]
    fn unary_minus_on_timepoint_is_type_error() {
        let err = parse("-(08:00)").unwrap().eval().unwrap_err();
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
        // 2h + 3h*2 = 2h + 6h = 8h  (scalar on the right)
        assert_eq!(eval_str("2h + 3h * 2"), "8h");
    }

    #[test]
    fn parens_override_precedence() {
        // (2h + 3h) * 2 = 10h
        assert_eq!(eval_str("(2h + 3h) * 2"), "10h");
    }

    #[test]
    fn left_associative_subtraction() {
        // 5h - 2h - 1h = 2h
        assert_eq!(eval_str("5h - 2h - 1h"), "2h");
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(eval_str("  2h   +   3h  "), "5h");
    }

    // --- the motivating expression --------------------------------------

    #[test]
    fn nested_timepoint_expression() {
        // ( 17:48 - 10:10 ) + ( 16:41 - 08:19 ) = 16h
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
        // The offending field is named, not a misleading "found ':'".
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
        // The error aggregates one message per reported problem (at least one),
        // each a non-empty rendered reason.
        let err = parse("2h +").unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(err.len(), err.messages.len());
        assert!(err.messages.iter().all(|m| !m.is_empty()));
    }

    // --- round-trip: Display output must parse back to the same value ----

    #[test]
    fn display_output_round_trips() {
        // Every string the evaluator can emit must be valid input that parses
        // and evaluates back to an equal value.
        for src in [
            "16h", "-16h", "0s", "2h15m", "1d2h3m4s", "-2h30m", "-1d",
            "08:00", "08:00:30", "1:14:30:00", "-1:23:00:00", "3", "-2.5",
        ] {
            let v1 = parse(src).unwrap().eval().unwrap();
            let shown = v1.to_string();
            let v2 = parse(&shown)
                .unwrap_or_else(|e| panic!("re-parsing Display output {shown:?} failed: {e}"))
                .eval()
                .unwrap();
            assert_eq!(v1, v2, "round-trip mismatch: {src:?} -> {shown:?}");
        }
    }

    // --- overflow robustness: must error, never panic --------------------

    #[test]
    fn oversized_number_errors_not_panics() {
        // In i64 contexts (duration magnitude, timepoint field) an oversized
        // number must report an error, never panic.
        assert!(parse("99999999999999999999d").is_err());
        assert!(parse("99999999999999999999:00").is_err());
    }

    #[test]
    fn oversized_bare_scalar_is_allowed() {
        // A bare number is a Scalar (f64), which has range for huge magnitudes,
        // so this parses rather than erroring. (Documents the i64-vs-f64 split.)
        assert!(parse("99999999999999999999").is_ok());
    }

    #[test]
    fn duration_component_overflow_errors() {
        // Fits in i64 as a bare number, but *d means *86400, which overflows.
        assert!(parse("9000000000000000d").is_err());
    }

    #[test]
    fn large_but_valid_duration_parses() {
        // ~1e11 seconds: large but well within i64, must succeed.
        assert_eq!(eval_str("1000000d"), "1000000d");
    }
}
