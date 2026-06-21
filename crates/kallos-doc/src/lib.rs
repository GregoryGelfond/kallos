//! A small strict (non-lazy) Wadler/Lindig pretty-printing engine.
//!
//! Build a document with [`DocBuilder`], then [`render`] it to a string at a
//! target width. The algebra is the eight primitives of Lindig's *Strictly
//! Pretty* plus a `verbatim` leaf for byte-exact spans. It is a roll-our-own
//! engine rather than the `pretty` crate.

mod arena;
mod builder;
mod render;

pub use arena::{DocNode, NodeId, Range, Span};
pub use builder::{Doc, DocBuilder};
pub use render::render;
