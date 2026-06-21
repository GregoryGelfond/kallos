//! The flat indexed Doc arena: a `Vec<DocNode>` of
//! `Copy` nodes; children are referenced by a [`Range`] into a *separate*
//! child-id buffer (the rust-analyzer arena idiom), so concatenation is "these
//! children, in order" and there is NO `Concat` node. Leaves carry source
//! offsets + a precomputed display width, keeping nodes `Copy` and
//! lifetime-free.

/// An index into the arena's node vector.
pub type NodeId = u32;

/// A half-open range `[start, end)` into the builder's child-id buffer (the
/// `child_ids` vector), NOT into the `DocNode` vector. Interior nodes
/// (`Seq`/`Group`/`Nest`) name their children through this indirection, which
/// is why concatenation needs no dedicated node and is correct regardless of
/// the order in which children were constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: NodeId,
    pub end: NodeId,
}

/// A leaf's source span + precomputed display width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Byte offset of the slice's start in the render-time source string.
    pub start: u32,
    /// Length of the slice in bytes.
    pub len: u32,
    /// Unicode display width of the slice (precomputed once).
    pub width: u32,
}

/// One Doc node. `Copy`, lifetime-free. The Lindig primitives (`Nil`, `Leaf`, the
/// three line kinds, `Nest`, `Group`, `Seq` for concatenation) plus three extras:
/// the `Verbatim` leaf for byte-exact spans, a mode-invariant `Space` for
/// non-breaking blanks, and the non-forcing `BlankLine` marker for author
/// blank lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocNode {
    /// The empty document.
    Nil,
    /// Text with NO embedded newline; the invariant is enforced by the builder.
    Leaf(Span),
    /// A byte-exact span: internal `'\n'` stays literal (no re-indent); forces
    /// every enclosing group broken. Its flat width is irrelevant (never flat).
    Verbatim(Span),
    /// A space when flat, a newline + indent when broken.
    Line,
    /// A mode-invariant literal blank: a single space whether the enclosing group
    /// is flat or broken. Unlike `Line` it never becomes a newline — the
    /// emitter's non-breaking space (keeps a connective neck on its head's line),
    /// and never a forced break.
    Space,
    /// Nothing when flat, a newline + indent when broken.
    SoftLine,
    /// A forced newline + indent; forces every enclosing group broken.
    Hardline,
    /// A NON-forcing conditional blank-line marker: nothing when flat;
    /// when broken it promotes the adjacent line break to a blank line (an extra
    /// newline), via the renderer's deferred-break `blank` bit. Unlike `Hardline` it
    /// does NOT force its group broken — a list with an author blank between elements
    /// may still lay flat (and the blank vanishes, since a one-line form has no blank
    /// to preserve). The emitter always places it adjoining a base break: `hardline ⊕
    /// blank_line` between statements, `line`/`softline` ⊕ `blank_line` between
    /// exploded elements.
    BlankLine,
    /// Indent the children by a relative `delta` (composes additively).
    Nest { delta: i32, child: Range },
    /// A group: laid flat iff it fits AND contains no forced break. The
    /// `forced_break` flag is precomputed at construction.
    Group { child: Range, forced_break: bool },
    /// Concatenation: these children, in order.
    Seq(Range),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docnode_is_copy_and_small() {
        // `Copy` is required for the flat arena; the size guard catches
        // accidental bloat that would hurt cache density.
        fn assert_copy<T: Copy>() {}
        assert_copy::<DocNode>();
        assert!(
            std::mem::size_of::<DocNode>() <= 24,
            "DocNode must stay compact for cache density"
        );
    }
}
