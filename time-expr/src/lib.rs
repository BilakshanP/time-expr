//! time-expr: an affine time-algebra library with a small expression language.
//!
//! Models two core types — [`TimePoint`](value::TimePoint) (an absolute
//! instant relative to Day 0, 00:00:00) and [`Duration`](value::Duration)
//! (a signed displacement) — plus a dynamic [`Value`](value::Value) wrapper
//! that enforces which operations are meaningful between them (e.g.
//! `TimePoint - TimePoint` yields a `Duration`, but `TimePoint + TimePoint`
//! is a type error).
//!
//! The [`parser`] turns source text into an [`ast::Expr`] tree (or a
//! [`ast::Program`] with variables and `;`-separated statements), which
//! evaluates to a [`Value`](value::Value).

pub mod ast;
pub mod error;
pub mod op;
pub mod parser;
pub mod value;

mod helpers;

/// Common imports for consumers of the library.
pub mod prelude {
    pub use crate::ast::{Env, Expr, Program, Statement};
    pub use crate::error::{EvalError, ParseError, TimeExprError};
    pub use crate::op::BinOp;
    pub use crate::parser::{parse, parse_program};
    pub use crate::value::Value;
}
