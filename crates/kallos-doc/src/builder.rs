//! The Doc algebra as a builder writing into the flat arena (keep the algebra in
//! the builder so the representation stays reversible).
//!
//! The arena is a child-id buffer (the rust-analyzer idiom): the builder holds
//! both a `nodes` vector and a `child_ids` vector, and every interior node
//! (`Seq`/`Group`/`Nest`) names its children by a [`Range`] into `child_ids`.
//! Concatenation is therefore "append these ids to `child_ids`, record the
//! range" — correct regardless of the order in which the children were built,
//! and with no `Concat` node. A finished [`Doc`] is the two vectors plus the id
//! of its root node.

use crate::arena::{DocNode, NodeId, Range, Span};
use unicode_width::UnicodeWidthStr;

/// A finished document: the arena (nodes + child-id buffer) and its root.
#[derive(Clone, Debug)]
pub struct Doc {
    pub(crate) nodes: Vec<DocNode>,
    pub(crate) child_ids: Vec<NodeId>,
    pub(crate) root: NodeId,
}

impl Doc {
    /// The node at `id`.
    pub(crate) fn node(&self, id: NodeId) -> DocNode {
        self.nodes[id as usize]
    }

    /// The child ids named by `range` (an index range into the child-id buffer).
    pub(crate) fn children(&self, range: Range) -> &[NodeId] {
        children_of(&self.child_ids, range)
    }
}

/// Builds a [`Doc`] in the flat arena. `Leaf`/`Verbatim` are the only text
/// carriers and are constructed only from a source slice (by construction — there
/// is no constructor that takes a synthesized `String`, and no synthesized-literal
/// `text` primitive; inter-token spacing is `Line`).
#[derive(Default, Debug)]
pub struct DocBuilder {
    nodes: Vec<DocNode>,
    child_ids: Vec<NodeId>,
    /// Index-parallel to `nodes`: `forced[i]` is `true` iff `nodes[i]`'s subtree
    /// contains a `Hardline` or `Verbatim`. Maintained at push-time (see `push` /
    /// `forced_of`) so `group` reads a child's forced-break in O(1) instead of
    /// descending — the property that keeps document construction O(1)-stack.
    /// Build-only: dropped at `finish`; the renderer reads each `Group`'s baked
    /// `forced_break`, never this vector.
    forced: Vec<bool>,
}

impl DocBuilder {
    /// A fresh, empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, node: DocNode) -> NodeId {
        let id = to_u32(self.nodes.len());
        let forced = self.forced_of(node);
        self.nodes.push(node);
        self.forced.push(forced);
        debug_assert_eq!(
            self.forced.len(),
            self.nodes.len(),
            "the forced-break flag vector must stay index-parallel to nodes"
        );
        id
    }

    /// The forced-break flag for a node about to be pushed: `true` iff its subtree
    /// contains a `Hardline` or `Verbatim`. Reads only its DIRECT children's
    /// already-computed flags (the arena builds children before parents), so it is
    /// O(arity) with no descent — the push-time fold that keeps construction
    /// O(1)-stack.
    fn forced_of(&self, node: DocNode) -> bool {
        match node {
            DocNode::Hardline | DocNode::Verbatim(_) => true,
            DocNode::Group { forced_break, .. } => forced_break,
            DocNode::Nest { child, .. } | DocNode::Seq(child) => {
                children_of(&self.child_ids, child)
                    .iter()
                    .any(|&c| self.forced[c as usize])
            }
            DocNode::Nil
            | DocNode::Leaf(_)
            | DocNode::Line
            | DocNode::SoftLine
            | DocNode::Space
            | DocNode::BlankLine => false,
        }
    }

    /// Append `items` to the child-id buffer and return the range they occupy.
    fn push_children(&mut self, items: &[NodeId]) -> Range {
        let start = to_u32(self.child_ids.len());
        self.child_ids.extend_from_slice(items);
        let end = to_u32(self.child_ids.len());
        Range { start, end }
    }

    /// The empty document.
    #[must_use]
    pub fn nil(&mut self) -> NodeId {
        self.push(DocNode::Nil)
    }

    /// A leaf from a source slice at byte offset `start`. INVARIANT: `slice`
    /// contains no `'\n'` (the caller routes newline-bearing slices to
    /// [`verbatim`](Self::verbatim)). The display width is precomputed once.
    #[must_use]
    pub fn leaf(&mut self, start: u32, slice: &str) -> NodeId {
        debug_assert!(
            !slice.contains('\n'),
            "Leaf must not contain a newline; use verbatim"
        );
        let span = Span {
            start,
            len: to_u32(slice.len()),
            width: to_u32(slice.width()),
        };
        self.push(DocNode::Leaf(span))
    }

    /// A byte-exact verbatim span (the 8th primitive). Internal
    /// newlines stay literal (no re-indent); it forces enclosing groups broken.
    /// Its width is left at zero because a verbatim span can never render flat.
    #[must_use]
    pub fn verbatim(&mut self, start: u32, slice: &str) -> NodeId {
        let span = Span {
            start,
            len: to_u32(slice.len()),
            width: 0,
        };
        self.push(DocNode::Verbatim(span))
    }

    /// A space when flat, a newline + indent when broken.
    #[must_use]
    pub fn line(&mut self) -> NodeId {
        self.push(DocNode::Line)
    }

    /// A mode-invariant literal blank: a single `' '` whether the enclosing group
    /// is flat or broken (unlike [`line`](Self::line), which becomes a newline
    /// when broken). This is the emitter's non-breaking space — the join that
    /// keeps a connective neck on its head's line without synthesizing token text
    /// (a blank is not token text, and the renderer pushes a `char`, never a leaf
    /// slice).
    #[must_use]
    pub fn space(&mut self) -> NodeId {
        self.push(DocNode::Space)
    }

    /// Nothing when flat, a newline + indent when broken.
    #[must_use]
    pub fn softline(&mut self) -> NodeId {
        self.push(DocNode::SoftLine)
    }

    /// A forced newline + indent; forces enclosing groups broken.
    #[must_use]
    pub fn hardline(&mut self) -> NodeId {
        self.push(DocNode::Hardline)
    }

    /// A non-forcing conditional blank line: nothing when flat; a blank
    /// line when the enclosing context breaks (it promotes the adjacent line break to
    /// an extra newline). NOT a forced break — it never itself explodes a group. The
    /// emitter places it right after a base break: `hardline ⊕ blank_line` between
    /// statements, `line`/`softline` ⊕ `blank_line` between exploded-block elements.
    #[must_use]
    pub fn blank_line(&mut self) -> NodeId {
        self.push(DocNode::BlankLine)
    }

    /// Indent `child` by a relative `delta` (composes additively because nest is
    /// relative — the renderer pushes `indent + delta`).
    #[must_use]
    pub fn nest(&mut self, delta: i32, child: NodeId) -> NodeId {
        let child = self.push_children(&[child]);
        self.push(DocNode::Nest { delta, child })
    }

    /// A group: laid flat iff it fits AND contains no forced break (the
    /// renderer's `fits` skips a group that can never be flat).
    ///
    /// INVARIANT: for every node `n`, `forced[n]` holds iff `n`'s subtree contains
    /// a `Hardline` or `Verbatim`. A group's `forced_break` is therefore exactly
    /// its child's flag — read here in O(1), never by descent (the flag is folded
    /// in at push-time; see `push` / `forced_of`).
    #[must_use]
    pub fn group(&mut self, child: NodeId) -> NodeId {
        let forced_break = self.forced[child as usize];
        let child = self.push_children(&[child]);
        self.push(DocNode::Group {
            child,
            forced_break,
        })
    }

    /// Concatenate `items`, in order. Returns `nil` for an empty slice and the
    /// sole element for a one-element slice (a concatenation of one *is* that
    /// element); otherwise records the children and pushes a `Seq`.
    #[must_use]
    pub fn seq(&mut self, items: &[NodeId]) -> NodeId {
        match items {
            [] => self.nil(),
            [single] => *single,
            _ => {
                let child = self.push_children(items);
                self.push(DocNode::Seq(child))
            }
        }
    }

    /// Finish the build, naming `root` as the document's root node.
    #[must_use]
    pub fn finish(self, root: NodeId) -> Doc {
        Doc {
            nodes: self.nodes,
            child_ids: self.child_ids,
            root,
        }
    }
}

/// Slice the child-id buffer by a [`Range`]. The single place the child-range
/// cast lives, shared by the builder (pre-finish) and [`Doc`] (post-finish).
fn children_of(buf: &[NodeId], range: Range) -> &[NodeId] {
    &buf[range.start as usize..range.end as usize]
}

/// Narrow a buffer length/offset to the arena's `u32` index domain. Fails fast
/// rather than silently truncating: a `.lp` source larger than 4 GiB is out of
/// scope, and a silent wrap would corrupt offsets and break the formatter's
/// equivalence guarantee.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).expect(
        "kallos-doc: a source offset/length exceeds u32::MAX \
         (sources larger than 4 GiB are out of scope)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render;

    #[test]
    fn seq_of_leaves_concatenates_in_order() {
        // No synthesized-literal `text()` primitive: inter-token
        // spacing is a flat `Line`, and every leaf is a slice of the one `src`.
        let mut builder = DocBuilder::new();
        let src = "a b";
        let a = builder.leaf(0, "a");
        let sp = builder.line();
        let c = builder.leaf(2, "b");
        let s = builder.seq(&[a, sp, c]);
        let g = builder.group(s);
        let doc = builder.finish(g);
        assert_eq!(render(&doc, src, 80), "a b");
    }

    #[test]
    fn group_flat_when_fits_uses_line_as_space() {
        let mut builder = DocBuilder::new();
        let src = "ab";
        let a = builder.leaf(0, "a");
        let l = builder.line();
        let c = builder.leaf(1, "b");
        let inner = builder.seq(&[a, l, c]);
        let g = builder.group(inner);
        let doc = builder.finish(g);
        assert_eq!(render(&doc, src, 80), "a b"); // fits → flat → Line is a space
    }

    #[test]
    fn empty_seq_is_nil_and_single_seq_is_the_element() {
        let mut builder = DocBuilder::new();
        let leaf = builder.leaf(0, "x");
        assert_eq!(builder.seq(&[leaf]), leaf); // a concatenation of one IS that one

        let empty = builder.seq(&[]);
        let doc = builder.finish(empty);
        assert_eq!(render(&doc, "x", 80), ""); // empty concatenation renders to nothing
    }

    #[test]
    fn forced_break_is_false_without_a_hardline_or_verbatim() {
        // A group of leaves and a Line carries no forced break (it MAY lay flat).
        let mut builder = DocBuilder::new();
        let a = builder.leaf(0, "a");
        let l = builder.line();
        let c = builder.leaf(1, "b"); // src "ab"
        let inner = builder.seq(&[a, l, c]);
        let g = builder.group(inner);
        match builder.finish(g).node(g) {
            DocNode::Group { forced_break, .. } => assert!(!forced_break),
            other => panic!("expected a group, got {other:?}"),
        }
    }

    #[test]
    fn a_hardline_forces_its_enclosing_group() {
        let mut builder = DocBuilder::new();
        let h = builder.hardline();
        let g = builder.group(h);
        match builder.finish(g).node(g) {
            DocNode::Group { forced_break, .. } => assert!(forced_break),
            other => panic!("expected a group, got {other:?}"),
        }
    }

    #[test]
    fn a_verbatim_forces_its_enclosing_group() {
        let mut builder = DocBuilder::new();
        let v = builder.verbatim(0, "X\nY"); // src "X\nY"; a verbatim can never lay flat
        let g = builder.group(v);
        match builder.finish(g).node(g) {
            DocNode::Group { forced_break, .. } => assert!(forced_break),
            other => panic!("expected a group, got {other:?}"),
        }
    }

    #[test]
    fn forced_break_propagates_through_a_nested_group() {
        // The subtle case: a hardline buried inside an INNER group must still
        // mark the OUTER group's forced_break. This is the `Group => forced_break`
        // arm of the fold — a nested group's flag is reused, never re-descended.
        let mut builder = DocBuilder::new();
        let h = builder.hardline();
        let inner = builder.group(h); // inner.forced_break == true
        let x = builder.leaf(0, "x"); // src "x"
        let s = builder.seq(&[x, inner]);
        let outer = builder.group(s);
        match builder.finish(outer).node(outer) {
            DocNode::Group { forced_break, .. } => {
                assert!(
                    forced_break,
                    "a forced break in a nested group must propagate"
                );
            }
            other => panic!("expected a group, got {other:?}"),
        }
    }

    /// Run `f` on a worker thread with a deliberately tiny 512 KiB stack and
    /// return its result. A descent deep enough to matter overflows this stack,
    /// so a test that *returns* proves the work is iterative, not call-stack-bound.
    /// (The Doc crate's local sibling of `kallos`'s
    /// `test_support::run_on_tiny_stack`: the two crates cannot share a
    /// `#[cfg(test)]` helper across the dependency edge, and the Doc engine proves
    /// its own depth-safety in isolation.)
    fn run_on_tiny_stack<T, F>(f: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(f)
            .expect("spawn the tiny-stack worker")
            .join()
            .expect("the build under test returns without overflowing the tiny stack")
    }

    #[test]
    fn building_a_group_over_a_deep_chain_is_stack_safe() {
        // Models a long left-recursive `binary_operation` (`a+a+…`) lowered as
        // bare nested `Seq`s and wrapped in ONE group (the shape the emitter
        // produces for such expressions).
        // Nested *groups* would NOT exercise this: each group memoizes its flag, so
        // it is the un-grouped `Seq` chain that descends to chain depth. The group's
        // forced-break is read from the child's flag in O(1) and the push-time fold
        // never recurses, so the build returns on the 512 KiB probe. A *returning*
        // `join()` proves O(1)-stack construction, because a Rust stack overflow
        // aborts rather than unwinds; the former recursion overflowed at this depth.
        //
        // The exact byte length is the checkable post-condition that every leaf was
        // emitted in order, none dropped or merged (so the chain was built to full
        // depth, not collapsed by a `seq` passthrough). The output is a single line
        // because it has no break nodes, NOT because it lays flat: at width 80 the
        // 200_001-column content does not fit, so the group renders in Break mode,
        // and the byte length is the same in either mode.
        const DEEP_CHAIN: usize = 100_000;
        let out = run_on_tiny_stack(|| {
            let mut builder = DocBuilder::new();
            let src = "a+"; // leaves are slices of this: "a" at 0, "+" at 1
            let mut node = builder.leaf(0, "a");
            for _ in 0..DEEP_CHAIN {
                let op = builder.leaf(1, "+");
                let a = builder.leaf(0, "a");
                node = builder.seq(&[node, op, a]); // left-nested: ((node)+a)
            }
            let g = builder.group(node);
            render(&builder.finish(g), src, 80)
        });
        assert_eq!(
            out.len(),
            1 + 2 * DEEP_CHAIN,
            "every leaf emitted in order; the chain was built to full depth"
        );
        assert!(out.starts_with("a+a"));
    }
}
