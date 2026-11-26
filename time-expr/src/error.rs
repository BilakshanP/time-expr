use std::fmt;

use crate::op::{BinOp, UnaryOp};

/// Error returned when constructing a [`TimePoint`](crate::value::TimePoint)
/// with out-of-range components (e.g. hours >= 24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeError {
    pub day: i64,
    pub hours: u32,
    pub mins: u32,
    pub secs: u32,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            day,
            hours,
            mins,
            secs,
        } = self;

        // List only the components that actually violate their bound.
        let mut problems = Vec::new();
        if *hours >= 24 {
            problems.push(format!("hours={hours} (must be < 24)"));
        }
        if *mins >= 60 {
            problems.push(format!("mins={mins} (must be < 60)"));
        }
        if *secs >= 60 {
            problems.push(format!("secs={secs} (must be < 60)"));
        }

        write!(
            f,
            "non-canonical TimePoint {day}d {hours}h {mins}m {secs}s: {}",
            problems.join(", ")
        )
    }
}

impl std::error::Error for TimeError {}

/// Error returned when evaluating an operation between [`Value`](crate::value::Value)s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// The operator is not defined between these two value types
    /// (e.g. `TimePoint + TimePoint`).
    InvalidOperation {
        op: BinOp,
        lhs: &'static str,
        rhs: &'static str,
    },
    /// The unary operator is not defined for this value type
    /// (e.g. negating a `TimePoint`).
    InvalidUnaryOperation { op: UnaryOp, operand: &'static str },
    /// Division by a zero `Scalar` or zero-length `Duration`.
    DivisionByZero,
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::InvalidOperation { op, lhs, rhs } => write!(
                f,
                "operation '{op}' is not defined between {lhs} and {rhs}"
            ),
            EvalError::InvalidUnaryOperation { op, operand } => write!(
                f,
                "unary '{op}' is not defined on {operand}"
            ),
            EvalError::DivisionByZero => f.write_str("division by zero"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Error returned when parsing a source string into an [`Expr`](crate::ast::Expr).
///
/// Holds the parser's reported problems as a list of human-readable messages.
/// The parser currently stops at the first error (no recovery is configured),
/// so this is usually a single message; the `Vec` shape leaves room for
/// multi-error reporting if recovery is added later. Span-aware rendering is
/// available separately via [`parse_rich`](crate::parser::parse_rich) + the
/// `diagnostics` feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// One message per problem the parser found, in source order.
    pub messages: Vec<String>,
}

impl ParseError {
    /// Build from an iterator of rendered error messages.
    pub fn new(messages: impl IntoIterator<Item = String>) -> Self {
        Self {
            messages: messages.into_iter().collect(),
        }
    }

    /// A single catch-all "unexpected end of input" error, used when the parser
    /// failed but produced no diagnostics of its own.
    pub fn unexpected_eof() -> Self {
        Self::new([String::from("unexpected end of input")])
    }

    /// Number of distinct problems reported.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether there are no messages (should not normally happen on an error).
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.messages.as_slice() {
            [] => f.write_str("parse error"),
            [one] => write!(f, "{one}"),
            many => {
                writeln!(f, "{} parse errors:", many.len())?;
                for (i, m) in many.iter().enumerate() {
                    if i + 1 == many.len() {
                        write!(f, "  - {m}")?;
                    } else {
                        writeln!(f, "  - {m}")?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Top-level error unifying the two failure domains: parsing a source string
/// and evaluating the resulting expression. Lets `main`/a REPL handle both
/// with a single `?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeExprError {
    Parse(ParseError),
    Eval(EvalError),
}

impl fmt::Display for TimeExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeExprError::Parse(e) => write!(f, "parse error: {e}"),
            TimeExprError::Eval(e) => write!(f, "eval error: {e}"),
        }
    }
}

impl std::error::Error for TimeExprError {}

impl From<ParseError> for TimeExprError {
    fn from(e: ParseError) -> Self {
        TimeExprError::Parse(e)
    }
}

impl From<EvalError> for TimeExprError {
    fn from(e: EvalError) -> Self {
        TimeExprError::Eval(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn non_canonical(day: i64, hours: u32, mins: u32, secs: u32) -> String {
        TimeError {
            day,
            hours,
            mins,
            secs,
        }
        .to_string()
    }

    #[test]
    fn pinpoints_single_offending_field() {
        assert_eq!(
            non_canonical(0, 25, 0, 0),
            "non-canonical TimePoint 0d 25h 0m 0s: hours=25 (must be < 24)"
        );
        assert_eq!(
            non_canonical(0, 0, 60, 0),
            "non-canonical TimePoint 0d 0h 60m 0s: mins=60 (must be < 60)"
        );
        assert_eq!(
            non_canonical(0, 0, 0, 75),
            "non-canonical TimePoint 0d 0h 0m 75s: secs=75 (must be < 60)"
        );
    }

    #[test]
    fn pinpoints_multiple_offending_fields() {
        assert_eq!(
            non_canonical(2, 30, 90, 61),
            "non-canonical TimePoint 2d 30h 90m 61s: \
             hours=30 (must be < 24), mins=90 (must be < 60), secs=61 (must be < 60)"
        );
    }
}
