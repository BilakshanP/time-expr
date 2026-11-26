//! Abstract syntax tree for time expressions.
//!
//! An [`Expr`] is a tree of literals and binary operations.
//! The [`parser`](crate::parser) produces an `Expr`; [`Expr::eval`] walks it
//! and reduces it to a single [`Value`], surfacing any [`EvalError`] from the
//! underlying time algebra.
//!
//! A [`Program`] is a sequence of [`Statement`]s (assignments and bare
//! expressions), separated by `;`. Variables live in an [`Env`] that the
//! caller owns — in a REPL, the same `Env` is shared across lines.

use std::collections::HashMap;

use crate::{
    error::EvalError,
    op::{BinOp, UnaryOp},
    value::Value,
};

// =========================================================================
// Environment
// =========================================================================

/// Variable environment for expression evaluation.
///
/// Maps variable names (without the `$` prefix) to their current [`Value`].
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, Value>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.vars.get(name).copied()
    }

    pub fn set(&mut self, name: String, value: Value) {
        self.vars.insert(name, value);
    }
}

// =========================================================================
// Expression tree
// =========================================================================

/// A node in the expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A leaf value: a parsed `TimePoint`, `Duration`, or `Scalar`.
    Literal(Value),
    /// A variable reference (e.g. `$x`). The name does not include the `$`.
    Var(String),
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
    /// Variable references are resolved against `env`. Operands are evaluated
    /// depth-first; `?` propagates the first [`EvalError`] encountered.
    pub fn eval(&self, env: &Env) -> Result<Value, EvalError> {
        match self {
            Expr::Literal(v) => Ok(*v),
            Expr::Var(name) => env.get(name).ok_or_else(|| EvalError::UndefinedVariable {
                name: name.clone(),
            }),
            Expr::Unary { op, operand } => {
                let v = operand.eval(env)?;
                match op {
                    UnaryOp::Neg => v.neg(),
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                let l = lhs.eval(env)?;
                let r = rhs.eval(env)?;
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

// =========================================================================
// Statements and programs
// =========================================================================

/// A single statement: either an assignment or a bare expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `$name = expr` — assigns the evaluated value to a variable.
    Assign { name: String, expr: Expr },
    /// A bare expression whose value is the result of this statement.
    Expr(Expr),
}

/// A sequence of semicolon-separated statements.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

impl Program {
    /// Evaluate all statements in order, returning the value of the last one.
    ///
    /// Assignments bind a variable in `env` and also produce a value (the
    /// assigned value), so `$x = 2h; $y = $x / 2` returns `1h` without
    /// needing a trailing `; $y`.
    ///
    /// Returns `None` only if the program has no statements (empty input).
    pub fn eval(&self, env: &mut Env) -> Result<Option<Value>, EvalError> {
        let mut last = None;
        for stmt in &self.statements {
            match stmt {
                Statement::Assign { name, expr } => {
                    let val = expr.eval(env)?;
                    env.set(name.clone(), val);
                    last = Some(val);
                }
                Statement::Expr(expr) => {
                    last = Some(expr.eval(env)?);
                }
            }
        }
        Ok(last)
    }
}

// =========================================================================
// Tests
// =========================================================================

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
        assert_eq!(e.eval(&Env::new()).unwrap(), Value::Scalar(42.0));
    }

    // --- unary -----------------------------------------------------------

    #[test]
    fn eval_unary_negates_duration() {
        let e = Expr::unary(UnaryOp::Neg, lit_dur(0, 2, 0, 0));
        assert_eq!(
            e.eval(&Env::new()).unwrap(),
            Value::Duration(Duration::from_secs(-2 * 3600))
        );
    }

    #[test]
    fn eval_unary_on_timepoint_errors() {
        let e = Expr::unary(UnaryOp::Neg, lit_tp(0, 8, 0, 0));
        assert_eq!(
            e.eval(&Env::new()),
            Err(EvalError::InvalidUnaryOperation {
                op: UnaryOp::Neg,
                operand: "TimePoint",
            })
        );
    }

    // --- single binary op ------------------------------------------------

    #[test]
    fn eval_duration_plus_duration() {
        let e = Expr::binary(BinOp::Add, lit_dur(0, 2, 0, 0), lit_dur(0, 3, 0, 0));
        assert_eq!(
            e.eval(&Env::new()).unwrap(),
            Value::Duration(Duration::from_secs(5 * 3600))
        );
    }

    #[test]
    fn eval_timepoint_minus_timepoint_is_duration() {
        let e = Expr::binary(BinOp::Sub, lit_tp(0, 5, 0, 0), lit_tp(0, 2, 0, 0));
        assert_eq!(
            e.eval(&Env::new()).unwrap(),
            Value::Duration(Duration::from_secs(3 * 3600))
        );
    }

    // --- nested tree -----------------------------------------------------

    #[test]
    fn eval_nested_tree_bubbles_values_up() {
        let left = Expr::binary(BinOp::Sub, lit_tp(0, 17, 48, 0), lit_tp(0, 10, 10, 0));
        let right = Expr::binary(BinOp::Sub, lit_tp(0, 16, 41, 0), lit_tp(0, 8, 19, 0));
        let root = Expr::binary(BinOp::Add, left, right);
        assert_eq!(
            root.eval(&Env::new()).unwrap(),
            Value::Duration(Duration::from_secs(16 * 3600))
        );
    }

    // --- error propagation -----------------------------------------------

    #[test]
    fn eval_invalid_subexpression_bubbles_error() {
        let bad = Expr::binary(BinOp::Add, lit_tp(0, 1, 0, 0), lit_tp(0, 2, 0, 0));
        let root = Expr::binary(BinOp::Add, bad, lit_dur(0, 1, 0, 0));
        assert_eq!(
            root.eval(&Env::new()),
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
        assert_eq!(e.eval(&Env::new()), Err(EvalError::DivisionByZero));
    }

    // --- variables -------------------------------------------------------

    #[test]
    fn eval_var_lookup() {
        let mut env = Env::new();
        env.set("x".to_string(), Value::Duration(Duration::from_secs(3600)));
        let e = Expr::Var("x".to_string());
        assert_eq!(
            e.eval(&env).unwrap(),
            Value::Duration(Duration::from_secs(3600))
        );
    }

    #[test]
    fn eval_undefined_var_errors() {
        assert_eq!(
            Expr::Var("nope".to_string()).eval(&Env::new()),
            Err(EvalError::UndefinedVariable {
                name: "nope".to_string()
            })
        );
    }

    // --- program ---------------------------------------------------------

    #[test]
    fn program_assign_and_ref() {
        let prog = Program {
            statements: vec![
                Statement::Assign {
                    name: "x".to_string(),
                    expr: lit_dur(0, 2, 0, 0),
                },
                Statement::Expr(Expr::Var("x".to_string())),
            ],
        };
        let mut env = Env::new();
        assert_eq!(
            prog.eval(&mut env).unwrap().unwrap(),
            Value::Duration(Duration::from_secs(2 * 3600))
        );
    }

    #[test]
    fn program_assignment_returns_value() {
        // Assignment itself is the last statement — its value is the result.
        let prog = Program {
            statements: vec![Statement::Assign {
                name: "x".to_string(),
                expr: lit_dur(0, 5, 0, 0),
            }],
        };
        let mut env = Env::new();
        assert_eq!(
            prog.eval(&mut env).unwrap().unwrap(),
            Value::Duration(Duration::from_secs(5 * 3600))
        );
    }

    #[test]
    fn program_empty_returns_none() {
        let prog = Program {
            statements: vec![],
        };
        let mut env = Env::new();
        assert_eq!(prog.eval(&mut env).unwrap(), None);
    }
}
