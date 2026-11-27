//! Hand-written recursive descent parser: source text → [`Expr`] / [`Program`].
//!
//! Grammar (whitespace-insensitive around tokens):
//!
//! ```text
//! program   = statement (";" statement)* ";"?
//! statement = assign | expr
//! assign    = "$" ident "=" expr
//!
//! expr      = sum
//! sum       = product (("+" | "-") product)*
//! product   = unary   (("*" | "/") unary)*
//! unary     = atom | "-" unary
//! atom      = literal | var_ref | "(" expr ")"
//! var_ref   = "$" ident
//! literal   = duration | timepoint | scalar
//! ident     = [a-zA-Z_][a-zA-Z0-9_]*
//! ```
//!
//! Literals are disambiguated by lookahead:
//! - A digit run followed by `:` → **timepoint** (2–4 colon-separated fields).
//! - A digit run followed by `d`/`h`/`m`/`s` → **duration** (optional leading `-`).
//! - A digit run followed by `.` or end-of-token → **scalar** (`f64`).

use crate::{
    ast::{Expr, Program, Statement},
    error::ParseError,
    helpers::{checked_dhms_to_seconds, HOURS_PER_DAY, MINUTES_PER_HOUR, SECONDS_PER_MINUTE},
    op::{BinOp, UnaryOp},
    value::Value,
};

// =========================================================================
// Comment stripping
// =========================================================================

/// Strip `#` line comments and normalize newlines to `;`.
///
/// Each line's comment (from `#` to end of line) is removed, and blank /
/// comment-only lines are dropped. Remaining lines are joined with ` ; ` so
/// the parser's `;`-separation handles both semicolons and line breaks.
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
// Public API
// =========================================================================

/// Parse a source string into a single [`Expr`] (no statements / variables).
///
/// Line comments (`# ...`) are stripped before parsing.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let clean = strip_comments(src);
    let mut p = Parser::new(&clean);
    let expr = p.parse_expr()?;
    p.expect_eof()?;
    Ok(expr)
}

/// Parse a source string into a [`Program`] (statements separated by `;`).
///
/// Line comments (`# ...`) are stripped before parsing.
pub fn parse_program(src: &str) -> Result<Program, ParseError> {
    let clean = strip_comments(src);
    let mut p = Parser::new(&clean);
    let prog = p.parse_program()?;
    p.expect_eof()?;
    Ok(prog)
}

// =========================================================================
// Parser internals
// =========================================================================

struct Parser<'src> {
    src: &'src str,
    pos: usize,
}

impl<'src> Parser<'src> {
    fn new(src: &'src str) -> Self {
        Self { src, pos: 0 }
    }

    // --- helpers ---------------------------------------------------------

    fn remaining(&self) -> &'src str {
        &self.src[self.pos..]
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, ch: u8) -> bool {
        self.skip_whitespace();
        if self.peek() == Some(ch) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, ch: u8) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek() == Some(ch) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(format!(
                "expected '{}', found {}",
                ch as char,
                self.describe_current()
            )))
        }
    }

    fn expect_eof(&mut self) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.at_eof() {
            Ok(())
        } else {
            Err(self.error(format!(
                "unexpected trailing input: '{}'",
                self.remaining().trim()
            )))
        }
    }

    fn error(&self, msg: String) -> ParseError {
        ParseError::new([msg])
    }

    fn describe_current(&self) -> String {
        self.skip_whitespace_peek()
            .map(|b| format!("'{}'", b as char))
            .unwrap_or_else(|| "end of input".to_string())
    }

    fn skip_whitespace_peek(&self) -> Option<u8> {
        let mut i = self.pos;
        while i < self.src.len() && self.src.as_bytes()[i].is_ascii_whitespace() {
            i += 1;
        }
        self.src.as_bytes().get(i).copied()
    }

    // --- number scanning -------------------------------------------------

    /// Scan a run of ASCII digits, returning the slice. Does NOT skip leading
    /// whitespace — the caller must have positioned `pos` at a digit.
    fn scan_digits(&mut self) -> &'src str {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        &self.src[start..self.pos]
    }

    /// Parse an unsigned integer (run of digits) into `i64`.
    fn parse_u64_digits(&mut self) -> Result<i64, ParseError> {
        let digits = self.scan_digits();
        if digits.is_empty() {
            return Err(self.error("expected a number".to_string()));
        }
        digits
            .parse::<i64>()
            .map_err(|_| self.error(format!("number '{}' is too large", digits)))
    }

    // --- identifier ------------------------------------------------------

    fn parse_ident(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        match self.peek() {
            Some(b) if b.is_ascii_alphabetic() || b == b'_' => self.advance(),
            _ => return Err(self.error("expected identifier".to_string())),
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.advance();
            } else {
                break;
            }
        }
        Ok(self.src[start..self.pos].to_string())
    }

    // --- literal parsing -------------------------------------------------

    /// Try to parse a literal or variable reference. This is the core
    /// disambiguation logic:
    ///
    /// - `$` → variable reference
    /// - `-` followed by digits+unit or digits+`:` → signed duration/timepoint
    /// - digits followed by `:` → timepoint
    /// - digits followed by d/h/m/s → duration
    /// - digits (optionally with `.`) → scalar
    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        self.skip_whitespace();

        match self.peek() {
            Some(b'$') => {
                self.advance();
                let name = self.parse_ident()?;
                Ok(Expr::Var(name))
            }
            Some(b'(') => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(b')')?;
                Ok(expr)
            }
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number_or_time_literal(),
            Some(b) => Err(self.error(format!("unexpected character '{}'", b as char))),
            None => Err(self.error("unexpected end of input".to_string())),
        }
    }

    /// Parse a numeric literal: timepoint, duration, or scalar.
    /// Handles optional leading `-` for durations and signed timepoint days.
    fn parse_number_or_time_literal(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.advance();
        }

        // We need digits after an optional sign.
        if !matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos = start;
            return Err(self.error("expected a number".to_string()));
        }

        // Scan the digit run without parsing yet — we may need it as f64.
        let digits_start = self.pos;
        self.scan_digits();
        let digits_end = self.pos;
        let digit_str = &self.src[digits_start..digits_end];

        // Lookahead to disambiguate.
        match self.peek() {
            // Colon → timepoint — needs i64 for field value.
            Some(b':') => {
                let n: i64 = digit_str
                    .parse()
                    .map_err(|_| self.error(format!("number '{}' is too large", digit_str)))?;
                let signed = if negative { -n } else { n };
                self.parse_timepoint_rest(signed)
                    .map(Expr::Literal)
            }
            // Unit letter → duration — needs i64.
            Some(b'd' | b'h' | b'm' | b's') => {
                let n: i64 = digit_str
                    .parse()
                    .map_err(|_| self.error(format!("number '{}' is too large", digit_str)))?;
                self.parse_duration_rest(n, negative)
                    .map(Expr::Literal)
            }
            // Dot → decimal scalar, parse full slice as f64.
            Some(b'.') => {
                self.advance(); // consume '.'
                let frac_start = self.pos;
                self.scan_digits();
                if self.pos == frac_start {
                    return Err(self.error("expected digits after decimal point".to_string()));
                }
                let full = &self.src[start..self.pos];
                let val: f64 = full
                    .parse()
                    .map_err(|_| self.error(format!("invalid number '{full}'")))?;
                Ok(Expr::Literal(Value::Scalar(val)))
            }
            // Anything else → bare integer = scalar (parsed as f64 to handle
            // values that overflow i64 but fit in f64).
            _ => {
                let full = &self.src[start..self.pos];
                let val: f64 = full
                    .parse()
                    .map_err(|_| self.error(format!("invalid number '{full}'")))?;
                Ok(Expr::Literal(Value::Scalar(val)))
            }
        }
    }

    /// Parse remaining timepoint fields after the first (which may be signed).
    /// Expects the current position to be at the first `:`.
    fn parse_timepoint_rest(&mut self, first: i64) -> Result<Value, ParseError> {
        let mut fields = vec![first];

        while self.peek() == Some(b':') {
            self.advance(); // consume ':'
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("expected digits after ':'".to_string()));
            }
            let n = self.parse_u64_digits()?;
            fields.push(n);
            if fields.len() > 4 {
                return Err(self.error("too many timepoint fields (max 4)".to_string()));
            }
        }

        if fields.len() < 2 {
            return Err(self.error("timepoint needs at least 2 fields".to_string()));
        }

        let (day, h, m, s) = match fields.as_slice() {
            [h, m] => (0, *h, *m, 0),
            [h, m, s] => (0, *h, *m, *s),
            [d, h, m, s] => (*d, *h, *m, *s),
            _ => unreachable!(),
        };

        // Validate ranges.
        let hours_max = HOURS_PER_DAY as i64;
        let mins_max = MINUTES_PER_HOUR as i64;
        let secs_max = SECONDS_PER_MINUTE as i64;

        if !(0..hours_max).contains(&h) {
            return Err(self.error(format!("hour {h} out of range (0..{hours_max})")));
        }
        if !(0..mins_max).contains(&m) {
            return Err(self.error(format!("minute {m} out of range (0..{mins_max})")));
        }
        if !(0..secs_max).contains(&s) {
            return Err(self.error(format!("second {s} out of range (0..{secs_max})")));
        }

        Ok(Value::TimePoint(
            crate::value::TimePoint::from_dhms_normalized(
                day,
                h as u32,
                m as u32,
                s as u32,
            ),
        ))
    }

    /// Parse duration components after the first number. `first_num` is the
    /// magnitude of the first component (always positive), and the current
    /// position is at the first unit letter.
    fn parse_duration_rest(&mut self, first_num: i64, negative: bool) -> Result<Value, ParseError> {
        let mut d: i64 = 0;
        let mut h: i64 = 0;
        let mut m: i64 = 0;
        let mut s: i64 = 0;
        let mut has_any = false;

        // The first_num is pending — assign it to whatever unit we see.
        let mut pending = Some(first_num);

        loop {
            if let Some(num) = pending.take() {
                match self.peek() {
                    Some(b'd') => {
                        self.advance();
                        d = num;
                        has_any = true;
                    }
                    Some(b'h') => {
                        self.advance();
                        h = num;
                        has_any = true;
                    }
                    Some(b'm') => {
                        self.advance();
                        m = num;
                        has_any = true;
                    }
                    Some(b's') => {
                        self.advance();
                        s = num;
                        has_any = true;
                    }
                    _ => {
                        if has_any {
                            // We've consumed at least one component but hit a
                            // number not followed by a unit. This is likely the
                            // start of the next token (e.g., `2h 3` where 3 is
                            // a scalar). Rewind the number.
                            // But we parsed the digits already... this case
                            // shouldn't arise since we only enter this loop
                            // when we see a unit letter after scan_digits.
                            break;
                        }
                        return Err(self.error("empty duration".to_string()));
                    }
                }
            }

            // Check if more number+unit follows.
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                let num = self.parse_u64_digits()?;
                pending = Some(num);
            } else {
                break;
            }
        }

        if !has_any {
            return Err(self.error("empty duration".to_string()));
        }

        let secs = checked_dhms_to_seconds(d, h, m, s)
            .ok_or_else(|| self.error("duration is too large".to_string()))?;

        let secs = if negative {
            secs.checked_neg()
                .ok_or_else(|| self.error("duration is too large".to_string()))?
        } else {
            secs
        };

        Ok(Value::Duration(crate::value::Duration::from_secs(secs)))
    }

    // --- recursive descent -----------------------------------------------

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_sum()
    }

    /// sum = product (("+" | "-") product)*
    fn parse_sum(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_product()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'+') => {
                    self.advance();
                    let rhs = self.parse_product()?;
                    lhs = Expr::binary(BinOp::Add, lhs, rhs);
                }
                Some(b'-') => {
                    // Distinguish binary `-` from unary `-` or a negative
                    // literal. After a complete operand, `-` is binary.
                    self.advance();
                    let rhs = self.parse_product()?;
                    lhs = Expr::binary(BinOp::Sub, lhs, rhs);
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    /// product = unary (("*" | "/") unary)*
    fn parse_product(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'*') => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    lhs = Expr::binary(BinOp::Mul, lhs, rhs);
                }
                Some(b'/') => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    lhs = Expr::binary(BinOp::Div, lhs, rhs);
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    /// unary = "-" unary | atom
    ///
    /// We try `atom` first. If the current character is `-`, we need to decide
    /// whether it starts a negative literal (duration/timepoint/scalar) or is
    /// a unary negation operator. The heuristic:
    /// - `-` followed by digits then `:` → signed timepoint day, parse as atom
    /// - `-` followed by digits then d/h/m/s → negative duration, parse as atom
    /// - `-` followed by digits then `.` or end → negative scalar, parse as atom
    /// - `-` followed by `(` or `$` → unary negation
    /// - `-` followed by `-` → unary negation (double negative)
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        self.skip_whitespace();
        if self.peek() == Some(b'-') {
            // Look ahead to decide: literal or unary?
            if self.is_negative_literal_ahead() {
                // Parse the whole thing (including the `-`) as a literal atom.
                self.parse_atom()
            } else {
                // Unary negation.
                self.advance(); // consume '-'
                let operand = self.parse_unary()?;
                Ok(Expr::unary(UnaryOp::Neg, operand))
            }
        } else {
            self.parse_atom()
        }
    }

    /// Check if `-` at current position starts a negative literal rather than
    /// a unary operator. Peeks ahead without consuming.
    fn is_negative_literal_ahead(&self) -> bool {
        // pos is at '-'. Check what follows the '-'.
        let after_minus = self.pos + 1;
        if after_minus >= self.src.len() {
            return false;
        }
        let b = self.src.as_bytes()[after_minus];
        if !b.is_ascii_digit() {
            return false;
        }
        // Scan past digits to find what follows them.
        let mut i = after_minus;
        while i < self.src.len() && self.src.as_bytes()[i].is_ascii_digit() {
            i += 1;
        }
        // If digits are followed by a unit letter or ':', it's a literal.
        // If followed by '.' or end/whitespace/operator, it's a scalar literal.
        match self.src.as_bytes().get(i) {
            Some(b':') => true,          // timepoint
            Some(b'd' | b'h' | b'm' | b's') => true,  // duration
            Some(b'.') => true,          // decimal scalar
            // Bare integer after '-': only treat as literal if there's no
            // operator context. We can't easily know, so treat `-N` where N
            // has no suffix as unary. This matches the original chumsky
            // behavior where `-3` in `2h - 3` is binary minus + scalar 3.
            _ => false,
        }
    }

    // --- program & statement parsing -------------------------------------

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut statements = Vec::new();

        self.skip_whitespace();
        if self.at_eof() {
            return Err(self.error("unexpected end of input".to_string()));
        }

        loop {
            self.skip_whitespace();
            if self.at_eof() {
                break;
            }

            let stmt = self.parse_statement()?;
            statements.push(stmt);

            self.skip_whitespace();
            if self.eat(b';') {
                // Consume any additional semicolons (from newline joining).
                while self.eat(b';') {}
                continue;
            } else {
                break;
            }
        }

        Ok(Program { statements })
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        self.skip_whitespace();

        // Try assignment: $ident = expr
        if self.peek() == Some(b'$') {
            // Speculatively parse: save position, try $ident =
            let saved = self.pos;
            self.advance(); // consume '$'
            if let Ok(name) = self.parse_ident() {
                self.skip_whitespace();
                if self.peek() == Some(b'=') {
                    self.advance(); // consume '='
                    let expr = self.parse_expr()?;
                    return Ok(Statement::Assign { name, expr });
                }
            }
            // Not an assignment — rewind and parse as expression.
            self.pos = saved;
        }

        let expr = self.parse_expr()?;
        Ok(Statement::Expr(expr))
    }
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
