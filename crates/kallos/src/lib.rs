//! `kallos` is a pure-layout source formatter for ASP / clingo (`.lp`): it
//! re-spaces and re-indents a program to a consistent house style, preserving
//! the program's meaning and its comments exactly. See the README for the design
//! and the formatting rules.

mod comments;
mod cst;
mod emit;
mod equiv;
mod fusion;
mod style;
#[cfg(test)]
mod test_support;

pub use cst::has_error;
pub use emit::format;
pub use equiv::{verify, CommentMismatch, Mismatch, StructuralMismatch};
pub use style::Style;
