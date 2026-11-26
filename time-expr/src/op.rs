use std::fmt;

/// A binary operator in the time-expression language.
///
/// This is the single, shared operator vocabulary used across the crate:
/// the evaluator ([`crate::value::Value`]) dispatches on it, errors
/// ([`crate::error::EvalError`]) carry it, and the AST/parser build with it.
/// Keeping it as a leaf type (depended on, never depending) avoids any module
/// cycle between the error, evaluator, and AST layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl BinOp {
    /// The source symbol for this operator (`+`, `-`, `*`, `/`).
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
        }
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// A unary (prefix) operator in the time-expression language.
///
/// Currently just negation. Like [`BinOp`], it is a leaf type shared by the
/// parser, AST, evaluator, and errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
}

impl UnaryOp {
    /// The source symbol for this operator (`-`).
    pub fn symbol(self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
        }
    }
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}
