//! Abstract syntax tree for time expressions.
//!
//! An [`Expr`] is a tree of literals and binary operations.
//! The [`parser`](crate::parser) produces an `Expr`; [`Expr::eval`] walks it and
//! reduces it to a single [`Value`], surfacing any [`EvalError`] from the
//! underlying time algebra.

use crate::{
    error::EvalError,
    op::{BinOp, UnaryOp},
    value::Value,
};

/// A node in the expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A leaf value: a parsed `TimePoint`, `Duration`, or `Scalar`.
    Literal(Value),
    /// A unary operation applied to one sub-expression (e.g. negation).
    Unary { op: UnaryOp, operand: Box<Expr> },
    /// A binary operation applied to two sub-expressions. Grouping/precedence
    /// is encoded by the tree shape, so there is no explicit paren node.
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

impl Expr {
    /// Convenience constructor for a literal node.
    pub fn literal(value: Value) -> Self {
        Expr::Literal(value)
    }

    /// Convenience constructor for a binary node, boxing the operands.
    pub fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Self {
        Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    /// Convenience constructor for a unary node, boxing the operand.
    pub fn unary(op: UnaryOp, operand: Expr) -> Self {
        Expr::Unary {
            op,
            operand: Box::new(operand),
        }
    }

    /// Evaluate the tree to a single [`Value`].
    ///
    /// Operands are evaluated depth-first; `?` propagates the first
    /// [`EvalError`] encountered (the error "bubbles" up the tree), so an
    /// invalid sub-expression short-circuits the whole evaluation.
    pub fn eval(&self) -> Result<Value, EvalError> {
        match self {
            Expr::Literal(v) => Ok(*v),
            Expr::Unary { op, operand } => {
                let v = operand.eval()?;

                match op {
                    UnaryOp::Neg => v.neg(),
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                let l = lhs.eval()?;
                let r = rhs.eval()?;

                match op {
                    BinOp::Add => l.add(r),
                    BinOp::Sub => l.sub(r),
                    BinOp::Mul => l.mul(r),
                    BinOp::Div => l.div(r),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Duration, TimePoint};

    // --- helpers ---------------------------------------------------------

    fn lit_tp(day: i64, h: u32, m: u32, s: u32) -> Expr {
        Expr::literal(Value::TimePoint(
            TimePoint::from_dhms_strict(day, h, m, s).unwrap(),
        ))
    }

    fn lit_dur(day: i64, h: u32, m: u32, s: u32) -> Expr {
        Expr::literal(Value::Duration(Duration::from_dhms(day, h, m, s)))
    }

    fn lit_scalar(s: f64) -> Expr {
        Expr::literal(Value::Scalar(s))
    }

    // --- single literal --------------------------------------------------

    #[test]
    fn eval_literal_returns_value() {
        let e = lit_scalar(42.0);
        assert_eq!(e.eval().unwrap(), Value::Scalar(42.0));
    }

    // --- unary -----------------------------------------------------------

    #[test]
    fn eval_unary_negates_duration() {
        let e = Expr::unary(UnaryOp::Neg, lit_dur(0, 2, 0, 0));
        assert_eq!(
            e.eval().unwrap(),
            Value::Duration(Duration::from_secs(-2 * 3600))
        );
    }

    #[test]
    fn eval_unary_on_timepoint_errors() {
        let e = Expr::unary(UnaryOp::Neg, lit_tp(0, 8, 0, 0));
        assert_eq!(
            e.eval(),
            Err(EvalError::InvalidUnaryOperation {
                op: UnaryOp::Neg,
                operand: "TimePoint",
            })
        );
    }

    // --- single binary op ------------------------------------------------

    #[test]
    fn eval_duration_plus_duration() {
        // 2h + 3h = 5h
        let e = Expr::binary(BinOp::Add, lit_dur(0, 2, 0, 0), lit_dur(0, 3, 0, 0));
        assert_eq!(
            e.eval().unwrap(),
            Value::Duration(Duration::from_secs(5 * 3600))
        );
    }

    #[test]
    fn eval_timepoint_minus_timepoint_is_duration() {
        // 05:00:00 - 02:00:00 = 3h
        let e = Expr::binary(BinOp::Sub, lit_tp(0, 5, 0, 0), lit_tp(0, 2, 0, 0));
        assert_eq!(
            e.eval().unwrap(),
            Value::Duration(Duration::from_secs(3 * 3600))
        );
    }

    // --- nested tree: ( 17:48 - 10:10 ) + ( 16:41 - 08:19 ) = 16h --------

    #[test]
    fn eval_nested_tree_bubbles_values_up() {
        let left = Expr::binary(BinOp::Sub, lit_tp(0, 17, 48, 0), lit_tp(0, 10, 10, 0));
        let right = Expr::binary(BinOp::Sub, lit_tp(0, 16, 41, 0), lit_tp(0, 8, 19, 0));
        let root = Expr::binary(BinOp::Add, left, right);

        // 7h38m + 8h22m = 16h
        assert_eq!(
            root.eval().unwrap(),
            Value::Duration(Duration::from_secs(16 * 3600))
        );
    }

    // --- error propagation -----------------------------------------------

    #[test]
    fn eval_invalid_subexpression_bubbles_error() {
        // TimePoint + TimePoint is a type error; it must propagate out of the
        // enclosing operation rather than being swallowed.
        let bad = Expr::binary(BinOp::Add, lit_tp(0, 1, 0, 0), lit_tp(0, 2, 0, 0));
        let root = Expr::binary(BinOp::Add, bad, lit_dur(0, 1, 0, 0));

        assert_eq!(
            root.eval(),
            Err(EvalError::InvalidOperation {
                op: BinOp::Add,
                lhs: "TimePoint",
                rhs: "TimePoint",
            })
        );
    }

    #[test]
    fn eval_division_by_zero_bubbles_error() {
        let e = Expr::binary(BinOp::Div, lit_dur(0, 1, 0, 0), lit_scalar(0.0));
        assert_eq!(e.eval(), Err(EvalError::DivisionByZero));
    }
}
