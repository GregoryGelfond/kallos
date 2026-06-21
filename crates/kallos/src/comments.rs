//! The comment pre-pass: a pure, total, single-valued (NOT injective)
//! classification of every trivia comment to exactly one anchor, in ONE
//! source-order pass (O(n+c) — never a per-comment tree search). The emitter
//! filters comments out of its own walk and re-injects them from this table at
//! their anchors.
//!
//! Trivia is exactly `line_comment` (`% …`) and `block_comment` (`%* … *%`), the
//! grammar's two comment `extras`. `doc_comment` (`%*! … *%`) is NOT trivia: it
//! is a first-class `statement` that flows through the emitter
//! verbatim, so it is not attached here. The classifier keys on the
//! tree-sitter kind id, so a `doc_comment` node is simply not a comment to it.
//!
//! ## Totality and single-valuedness (the load-bearing correctness property)
//!
//! `attach` is total and single-valued by construction, and a reader can see why:
//!
//! - [`collect`] is one DFS that visits every node exactly once; at each node it
//!   sweeps the direct children left-to-right, so every `line_comment` /
//!   `block_comment` in the tree is handled exactly once (each comment has one
//!   enclosing node, visited once).
//! - Each comment yields EXACTLY ONE push into the table. It is either resolved
//!   inline as `trailing` (it starts on its preceding non-comment node's line),
//!   or it is buffered in `pending` and drained exactly once — to `leading` /
//!   `dangling` when the next leading-anchor sibling arrives ([`drain_leading`]),
//!   or to `dangling` at a boundary / end-of-children when none follows
//!   ([`flush_dangling`]). `pending` is cleared by each drain, so no comment is
//!   drained twice, and the end-of-children flush guarantees none is left behind.
//! - The enclosing node always exists — `source_file` at the root is the bottom
//!   anchor — so the cascade never falls through.
//!
//! Therefore `total_attached()` equals the number of trivia comments in the
//! source. The function is NOT injective: a contiguous run of leading comments
//! all attach to the one following node, pushed in source order.
//!
//! ## The blank-line detach rule is block-aware
//!
//! The rule: a blank line between a leading comment and its node detaches it.
//! Applied per comment over a *run* of leading comments, this means the whole
//! contiguous run immediately above a node leads it (they share the anchor),
//! while any comment cut off from the node by a blank line detaches to
//! `dangling`. [`drain_leading`] computes this with one upward scan. (Measuring
//! the blank gap against the next *named* sibling instead would wrongly detach
//! every comment but the last in a run; that contradicts the rule that
//! multiple leading comments legitimately share one anchor.)

use crate::cst;
use rustc_hash::FxHashMap;
use std::ops::Range;
use tree_sitter::{Language, Node, Tree};

/// Which trivia comment kind this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommentKind {
    /// `% …` — runs to end of line; never contains a newline.
    Line,
    /// `%* … *%` — may span lines.
    Block,
}

/// One attached trivia comment: its byte `range`, plus a `multiline` flag set for a
/// multi-line `block_comment` so the emitter renders it as a verbatim span with
/// no re-indent. This pass carries the byte `range` only; it neither strips trailing
/// whitespace nor mutates the text — both are emit-time concerns.
#[derive(Clone, Debug)]
pub(crate) struct Trivia {
    pub(crate) range: Range<usize>,
    pub(crate) multiline: bool,
}

/// The comments attached to one anchor node, split by slot. Within each slot the
/// comments are in source order (the classifier is not injective: a slot may hold
/// several).
#[derive(Clone, Default, Debug)]
pub(crate) struct AnchorComments {
    pub(crate) leading: Vec<Trivia>,
    pub(crate) trailing: Vec<Trivia>,
    pub(crate) dangling: Vec<Trivia>,
}

/// The result of the pre-pass: every trivia comment classified to one anchor,
/// keyed by [`Node::id`] (an `FxHashMap` over the stable node id,
/// which the emitter looks up as it re-encounters each node in its own walk).
#[derive(Clone, Default, Debug)]
pub(crate) struct Attachments {
    by_anchor: FxHashMap<usize, AnchorComments>,
}

impl Attachments {
    /// The comments attached to `id`, if any. `None` means the node anchors no
    /// comments (the common case), so the emitter pays nothing for clean nodes.
    #[must_use]
    pub(crate) fn for_node(&self, id: usize) -> Option<&AnchorComments> {
        self.by_anchor.get(&id)
    }

    /// The anchor's mutable comment record, created empty on first touch.
    fn entry(&mut self, id: usize) -> &mut AnchorComments {
        self.by_anchor.entry(id).or_default()
    }

    /// Total comments attached across all anchors and slots. Equals the number of
    /// trivia comments in the source — the totality invariant, asserted by tests.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn total_attached(&self) -> usize {
        self.by_anchor
            .values()
            .map(|a| a.leading.len() + a.trailing.len() + a.dangling.len())
            .sum()
    }
}

/// The kind ids the classify pass dispatches on, resolved once from the grammar so the
/// walk compares `u16` rather than re-comparing `&str` kind names at every node.
/// Two structural categories matter to the drain, and the rest
/// of the grammar's tokens need no id at all:
///
/// - the two trivia comment kinds (`kind_of`);
/// - the BOUNDARIES — the three bracket closers `)` / `]` / `}` and the statement
///   terminator `.` (`is_boundary`) — past which a pending comment does NOT drain: it
///   belongs *inside* the enclosing construct (it dangles on the enclosing node), never on
///   a sibling that follows the boundary (an aggregate's `} >= 1` guard, a weak
///   constraint's `. [w@p]` tail);
/// - the `abs` node kind (`is_abs`) and the `|` bar (`is_pipe`): a `|` is a leading anchor
///   EXCEPT when it delimits an `abs` (`|t|` / `|t; …|`), where a pending comment must
///   dangle inside the abs rather than lead the closing bar (see [`classify_children`]).
struct ClassifyIds {
    line: u16,
    block: u16,
    rparen: u16,
    rbracket: u16,
    rbrace: u16,
    dot: u16,
    pipe: u16,
    abs: u16,
}

impl ClassifyIds {
    fn resolve(lang: &Language) -> Self {
        let anon = |k: &str| lang.id_for_node_kind(k, false);
        Self {
            line: lang.id_for_node_kind("line_comment", true),
            block: lang.id_for_node_kind("block_comment", true),
            rparen: anon(")"),
            rbracket: anon("]"),
            rbrace: anon("}"),
            dot: anon("."),
            pipe: anon("|"),
            abs: lang.id_for_node_kind("abs", true),
        }
    }

    /// `Some(kind)` iff `node` is a trivia comment; `None` for every other node
    /// (notably `doc_comment`, which is a statement, not trivia).
    fn kind_of(&self, node: Node) -> Option<CommentKind> {
        let id = node.kind_id();
        if id == self.line {
            Some(CommentKind::Line)
        } else if id == self.block {
            Some(CommentKind::Block)
        } else {
            None
        }
    }

    /// Whether `node` is a BOUNDARY for a pending comment — a bracket closer `)` /
    /// `]` / `}`, or the statement terminator `.` — past which a comment does not drain
    /// (it dangles on the enclosing node instead, e.g. before an aggregate's `} >= 1`
    /// guard or a weak constraint's `. [w@p]` tail).
    fn is_boundary(&self, node: Node) -> bool {
        let id = node.kind_id();
        id == self.rparen || id == self.rbracket || id == self.rbrace || id == self.dot
    }

    /// Whether `node` is the `|` bar. `|` is dual-purpose: an abs `|…|` / `|t; …|` bracket
    /// DELIMITER, and a disjunction-head SEPARATOR. It is a leading anchor in the separator
    /// role (like `,`/`;`), but NOT in the delimiter role — a pending comment before a
    /// CLOSING abs bar must dangle inside the abs (the end-of-children flush), not lead the
    /// bar. [`classify_children`] disambiguates by the enclosing node: a `|` is carved out
    /// from draining ONLY when its parent is an `abs` ([`is_abs`]).
    fn is_pipe(&self, node: Node) -> bool {
        node.kind_id() == self.pipe
    }

    /// Whether `node` is an `abs` (`|t|` / `|t; …|`) — the one context where a `|` child is
    /// a bracket delimiter (not a disjunction separator), so a pending comment before the
    /// closing bar dangles inside rather than leading it (see [`classify_children`]).
    fn is_abs(&self, node: Node) -> bool {
        node.kind_id() == self.abs
    }
}

/// Classify every `line_comment` / `block_comment` in `tree` to exactly one
/// anchor. Pure, total, single-valued (not injective); one O(n+c) source-order
/// pass. The priority cascade is trailing > leading > dangling, with
/// `source_file` as the bottom anchor that makes the function total.
// Reached via the public `format` (the permanent pipeline root), so `attach` is live
// without its own dead-code guard.
#[must_use]
pub(crate) fn attach(tree: &Tree, src: &str) -> Attachments {
    let language: Language = tree_sitter_clingo::LANGUAGE.into();
    let ids = ClassifyIds::resolve(&language);
    let mut at = Attachments::default();
    collect(&ids, tree.root_node(), src, &mut at);
    at
}

/// Classify `node`'s DIRECT comment children to anchors in one source-order sweep
/// over its children. Self-contained: it reads only `node`'s own children and
/// writes only `node`-local anchors (a child, or the enclosing `node`), so nodes
/// may be classified in any order. Neighbour information (the preceding non-comment
/// node, the following leading-anchor sibling) comes from the sweep's local state —
/// never a re-search — so the whole pass is O(n+c).
fn classify_children<'t>(ids: &ClassifyIds, node: Node<'t>, src: &str, at: &mut Attachments) {
    // `prev` = the immediately-preceding non-comment sibling (named OR anonymous,
    // e.g. a `:-` neck or a `;` separator). `pending` = the non-trailing comments
    // seen since the last drain, awaiting their following leading-anchor sibling to
    // decide leading-vs-dangling. `pending` only allocates when a comment is actually
    // buffered, so clean nodes cost nothing.
    let mut prev: Option<Node<'t>> = None;
    let mut pending: Vec<(Node<'t>, CommentKind)> = Vec::new();
    cst::walk_children(node, |_field, child| {
        if let Some(kind) = ids.kind_of(child) {
            // TRAILING wins (priority 1): a comment starting on its
            // preceding node's last line attaches to that node.
            if let Some(p) = prev.filter(|p| p.end_position().row == child.start_position().row) {
                at.entry(p.id()).trailing.push(trivia_of(child, kind, src));
            } else {
                pending.push((child, kind));
            }
        } else if ids.is_boundary(child) {
            // A closer `}`/`)`/`]` or the terminator `.` is a BOUNDARY:
            // comments pending INSIDE the construct dangle on the enclosing node — they
            // must not drain across the boundary to a sibling that follows it (an
            // aggregate's `} >= 1` guard, a weak constraint's `. [w@p]` tail). Flushing
            // here also keeps the attachment idempotent: the comment re-injects inside
            // the construct, where a re-parse of the output classifies it the same way.
            flush_dangling(&mut pending, node, src, at);
            prev = Some(child);
        } else {
            // Any non-boundary, non-comment sibling is the `next` anchor for everything
            // pending (priority 2: a leading comment leads the immediately-
            // FOLLOWING node — be it a named element, a `,`/`;` separator, a connective neck
            // `:-`/`:~`/`:`, a bracket opener `{`/`(`/`[`, or an infix `=`/`@`). Draining at
            // EVERY such token keeps a pending comment that sits before it BEFORE it in the
            // output, so the comment can never jump that token to land after the token's own
            // trailing comment — the source-order invariant (the transposition
            // fix, lifted from the original `,`/`;` case to the whole class). The SOLE
            // exception is a `|` that DELIMITS an `abs` (`is_pipe(child) && is_abs(node)`):
            // there it merely advances `prev`, so a pending comment before the closing bar
            // dangles inside the abs at the end-of-children flush. A `|` in its OTHER role —
            // a disjunction-head separator — IS a leading anchor (like `,`/`;`), so its
            // preceding comment cannot jump it either. Every drained token's emit site
            // re-injects its `leading` slot via `child_doc → reinject::wrap`, so source order
            // is preserved and re-derives identically on a second pass.
            if !(ids.is_pipe(child) && ids.is_abs(node)) {
                drain_leading(&mut pending, child, node, src, at);
            }
            prev = Some(child);
        }
    });
    // No following leading-anchor sibling for the leftovers (alone in a block, after
    // the last item before a closer, or a comment-only file) → dangling on the
    // enclosing node (priority 3; `source_file` makes this total).
    flush_dangling(&mut pending, node, src, at);
}

/// Classify every trivia comment to an anchor. An explicit work-list,
/// NOT recursion, so an adversarially-deep tree cannot overflow the stack — the
/// same precaution the renderer takes. Each node's classification is self-contained
/// ([`classify_children`]), so the processing order is free; the whole pass is
/// O(n+c).
fn collect<'t>(ids: &ClassifyIds, root: Node<'t>, src: &str, at: &mut Attachments) {
    let mut work: Vec<Node<'t>> = vec![root];
    while let Some(node) = work.pop() {
        classify_children(ids, node, src, at);
        cst::walk_children(node, |_field, child| work.push(child));
    }
}

/// Resolve the pending leading-candidate comments now that their following leading-
/// anchor sibling `next` is known (priority 2 with the blank-line detach).
///
/// A comment attaches to `next.leading` iff NO blank line lies between it and
/// `next`. The rows between a comment and `next` are occupied exactly by the
/// later pending comments, so "no blank line" is "sits in the contiguous comment
/// run immediately above `next`". Scanning upward from `next`, the first blank
/// gap is a single split point: comments below it lead `next`; the gapped comment
/// and everything above it detach to `enclosing.dangling`. One upward scan, so
/// the drain is linear in the run length and the whole pass stays O(n+c).
fn drain_leading<'t>(
    pending: &mut Vec<(Node<'t>, CommentKind)>,
    next: Node<'t>,
    enclosing: Node<'t>,
    src: &str,
    at: &mut Attachments,
) {
    // `reach` is the topmost row still contiguous with `next`; `split` is the
    // first index that leads `next` (everything below `split` dangles).
    let mut reach = next.start_position().row;
    let mut split = pending.len();
    for i in (0..pending.len()).rev() {
        let comment = pending[i].0;
        if comment.end_position().row + 1 >= reach {
            reach = comment.start_position().row;
            split = i;
        } else {
            break; // a blank gap: this comment and all above it detach
        }
    }
    for &(comment, kind) in &pending[..split] {
        at.entry(enclosing.id())
            .dangling
            .push(trivia_of(comment, kind, src));
    }
    for &(comment, kind) in &pending[split..] {
        at.entry(next.id())
            .leading
            .push(trivia_of(comment, kind, src));
    }
    pending.clear();
}

/// Drain whatever remains pending — no following named sibling exists — to the
/// enclosing node's `dangling` slot, in source order. This is the bottom of the
/// cascade that guarantees no comment is lost.
fn flush_dangling<'t>(
    pending: &mut Vec<(Node<'t>, CommentKind)>,
    enclosing: Node<'t>,
    src: &str,
    at: &mut Attachments,
) {
    for &(comment, kind) in pending.iter() {
        at.entry(enclosing.id())
            .dangling
            .push(trivia_of(comment, kind, src));
    }
    pending.clear();
}

/// Build the [`Trivia`] for one comment. `multiline` is set only for a
/// `block_comment` whose span contains a newline (a `line_comment` never does);
/// the emitter renders such a span verbatim. Carries the byte `range` only —
/// no whitespace stripping, no text mutation (emit-time concerns).
fn trivia_of(comment: Node, kind: CommentKind, src: &str) -> Trivia {
    let range = comment.byte_range();
    let multiline =
        matches!(kind, CommentKind::Block) && src[range.start..range.end].contains('\n');
    Trivia { range, multiline }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    enum Slot {
        Leading,
        Trailing,
        Dangling,
    }

    /// Depth-first search for the first node of `kind`, over ALL children (named
    /// and anonymous), so anchors like the `:-` neck token are reachable.
    fn find_first<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_first(child, kind) {
                return Some(found);
            }
        }
        None
    }

    /// The source text of the comment at `(id, slot, idx)`.
    fn text_of<'a>(at: &Attachments, id: usize, slot: Slot, idx: usize, src: &'a str) -> &'a str {
        let ac = at
            .for_node(id)
            .expect("anchor should have attached comments");
        let v = match slot {
            Slot::Leading => &ac.leading,
            Slot::Trailing => &ac.trailing,
            Slot::Dangling => &ac.dangling,
        };
        let t = &v[idx];
        &src[t.range.start..t.range.end]
    }

    // ---- formatting tests ----

    #[test]
    fn trailing_comment_attaches_to_preceding_node_on_same_line() {
        let src = "a :- b.  % note\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let rule = find_first(tree.root_node(), "rule").unwrap();
        let ac = at.for_node(rule.id());
        assert_eq!(ac.map(|a| a.trailing.len()), Some(1));
        assert_eq!(text_of(&at, rule.id(), Slot::Trailing, 0, src), "% note");
        assert!(ac.unwrap().leading.is_empty());
        assert_eq!(at.total_attached(), 1);
    }

    #[test]
    fn leading_comment_attaches_to_following_sibling() {
        let src = "% header\na :- b.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let rule = find_first(tree.root_node(), "rule").unwrap();
        assert_eq!(at.for_node(rule.id()).map(|a| a.leading.len()), Some(1));
        assert_eq!(text_of(&at, rule.id(), Slot::Leading, 0, src), "% header");
        assert_eq!(at.total_attached(), 1);
    }

    #[test]
    fn comment_only_file_dangles_on_root() {
        let src = "% lonely\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let root = tree.root_node();
        assert_eq!(at.for_node(root.id()).map(|a| a.dangling.len()), Some(1));
        assert_eq!(text_of(&at, root.id(), Slot::Dangling, 0, src), "% lonely");
        assert_eq!(at.total_attached(), 1);
    }

    #[test]
    fn every_comment_is_attached_exactly_once_totality() {
        // `% a` `% b` lead the rule; `% c` trails it (same line); `% d` dangles on
        // the root (no following node). Four in, four attached.
        let src = "% a\n% b\np.  % c\n% d\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        assert_eq!(at.total_attached(), 4);
    }

    #[test]
    fn totality_holds_with_comments_before_every_following_token_class() {
        // The transposition fix made EVERY following-token a leading anchor (necks
        // `:-`/`:`, opener `{`, separator `;`, infix `=`/`@`) except `|`. The cascade must
        // stay TOTAL and single-valued across the lot: a comment before each such token, a
        // trailing comment on it, a dangling-inside case before `}`, and the `|` carve-out
        // proper — a comment immediately before the abs CLOSING bar, which (unlike every other
        // following token) does NOT lead the `|` but dangles inside the abs (`|a; b %d |`).
        // Counting both `%b`-style leading and `%t`-style trailing comments below, every
        // comment lands in exactly one slot — `total_attached` equals the comment count, no
        // double-drain, none lost.
        for (src, n) in [
            ("a\n%b\n:- %t\nx.\n", 2),                   // rule neck `:-`
            ("a :- p\n%b\n: %t\nq.\n", 2),               // conditional `:`
            (":- #count\n%b\n{ %t\nx : p } >= 1.\n", 2), // opener `{` + dangling none
            ("a :- p\n%b\n; %t\nq.\n", 2),               // separator `;`
            ("#const c\n%b\n= %t\n5.\n", 2),             // infix `=`
            (":~ b. [2\n%b\n@ %t\n1]\n", 2),             // weight `@`
            ("a :- #count{ x : p\n%d\n}.\n", 1),         // dangling inside before `}`
            ("p :- q = |a\n%d\n; b|.\n", 1), // comment before `;` inside abs: the `;` leads it (drains there)
            ("p :- q = |a; b\n%d\n|.\n", 1), // `|` carve-out: comment before the CLOSING bar dangles inside abs
        ] {
            let tree = crate::cst::parse(src);
            let at = attach(&tree, src);
            assert_eq!(
                at.total_attached(),
                n,
                "classifier must attach every comment exactly once: {src:?}"
            );
        }
    }

    // ---- adversarial witnesses ----

    #[test]
    fn trailing_wins_over_leading_across_one_sweep() {
        // `% c` is same-line-as-rule → trailing (beats any leading reading); `% d`
        // has no following named sibling → dangling on the root. Exercises the
        // priority cascade end to end in a single enclosing sweep.
        let src = "p.  % c\n% d\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let root = tree.root_node();
        let rule = find_first(root, "rule").unwrap();
        assert_eq!(at.for_node(rule.id()).map(|a| a.trailing.len()), Some(1));
        assert_eq!(text_of(&at, rule.id(), Slot::Trailing, 0, src), "% c");
        assert_eq!(at.for_node(root.id()).map(|a| a.dangling.len()), Some(1));
        assert_eq!(text_of(&at, root.id(), Slot::Dangling, 0, src), "% d");
        assert_eq!(at.total_attached(), 2);
    }

    #[test]
    fn blank_line_detaches_leading_comment_to_dangling() {
        // Blank-line detach: a blank line between the comment and its node
        // detaches it to a standalone (dangling) comment on the enclosing root.
        let src = "% detached\n\np.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let root = tree.root_node();
        let rule = find_first(root, "rule").unwrap();
        assert!(at.for_node(rule.id()).is_none_or(|a| a.leading.is_empty()));
        assert_eq!(at.for_node(root.id()).map(|a| a.dangling.len()), Some(1));
        assert_eq!(
            text_of(&at, root.id(), Slot::Dangling, 0, src),
            "% detached"
        );
        assert_eq!(at.total_attached(), 1);
    }

    #[test]
    fn multiple_leading_comments_share_one_anchor() {
        // Single-valued but NOT injective: a contiguous run of leading comments
        // all attach to the one following node, in source order.
        let src = "% one\n% two\nfoo :- bar.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let rule = find_first(tree.root_node(), "rule").unwrap();
        let ac = at.for_node(rule.id()).expect("rule has leading comments");
        assert_eq!(ac.leading.len(), 2);
        assert_eq!(text_of(&at, rule.id(), Slot::Leading, 0, src), "% one");
        assert_eq!(text_of(&at, rule.id(), Slot::Leading, 1, src), "% two");
        assert_eq!(at.total_attached(), 2);
    }

    #[test]
    fn two_leading_blocks_split_on_the_blank() {
        // The blank between two comment blocks detaches the FIRST block (dangling
        // on root) while the block adjacent to the node leads it. This is the
        // block-aware blank rule that a per-`next` formula gets wrong.
        let src = "% a\n% b\n\n% c\n% d\np.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let root = tree.root_node();
        let rule = find_first(root, "rule").unwrap();
        assert_eq!(at.for_node(rule.id()).map(|a| a.leading.len()), Some(2));
        assert_eq!(at.for_node(root.id()).map(|a| a.dangling.len()), Some(2));
        assert_eq!(text_of(&at, rule.id(), Slot::Leading, 0, src), "% c");
        assert_eq!(text_of(&at, rule.id(), Slot::Leading, 1, src), "% d");
        assert_eq!(text_of(&at, root.id(), Slot::Dangling, 0, src), "% a");
        assert_eq!(text_of(&at, root.id(), Slot::Dangling, 1, src), "% b");
        assert_eq!(at.total_attached(), 4);
    }

    #[test]
    fn multiline_block_comment_sets_multiline_flag() {
        let src = "%* line1\nline2 *%\np.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let rule = find_first(tree.root_node(), "rule").unwrap();
        let ac = at
            .for_node(rule.id())
            .expect("rule has a leading block comment");
        assert_eq!(ac.leading.len(), 1);
        assert!(ac.leading[0].multiline);
        assert_eq!(at.total_attached(), 1);
    }

    #[test]
    fn single_line_comment_is_not_multiline() {
        let src = "% just one line\np.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let rule = find_first(tree.root_node(), "rule").unwrap();
        let ac = at.for_node(rule.id()).unwrap();
        assert!(!ac.leading[0].multiline);
    }

    #[test]
    fn comment_after_last_item_in_block_dangles_on_enclosing() {
        // A comment after the last aggregate element, before the `}` closer, has
        // no following NAMED sibling (the `}` is anonymous) → dangling on the
        // enclosing body_aggregate (a NON-root anchor). It must not be lost.
        let src = "c :- #count{ X : p(X)\n  % after last\n}.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let agg = find_first(tree.root_node(), "body_aggregate").unwrap();
        let ac = at
            .for_node(agg.id())
            .expect("aggregate has a dangling comment");
        assert_eq!(ac.dangling.len(), 1);
        assert_eq!(
            text_of(&at, agg.id(), Slot::Dangling, 0, src),
            "% after last"
        );
        assert_eq!(at.total_attached(), 1);
    }

    #[test]
    fn inline_block_between_code_classifies_as_trailing() {
        // The inline single-line block comment is classified by the
        // cascade (same line as the preceding `:-` token → trailing). The forced
        // break is the emitter's concern, NOT this pass — there is no Inline
        // slot here.
        let src = "a :- %* note *% b.\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        let neck = find_first(tree.root_node(), ":-").unwrap();
        let ac = at
            .for_node(neck.id())
            .expect("the neck token carries the trailing block");
        assert_eq!(ac.trailing.len(), 1);
        assert!(!ac.trailing[0].multiline);
        assert_eq!(at.total_attached(), 1);
    }

    #[test]
    fn doc_comment_is_not_trivia_and_is_not_attached() {
        // doc_comment (`%*! … *%`) is a first-class statement, not trivia, so the
        // pre-pass attaches nothing for it.
        let src = "%*! p/1 a predicate *%\np(1).\n";
        let tree = crate::cst::parse(src);
        let at = attach(&tree, src);
        assert_eq!(at.total_attached(), 0);
    }

    #[test]
    fn attach_does_not_overflow_on_deeply_nested_input() {
        // The pre-pass walks the tree with an explicit work-list, not recursion, so
        // an arbitrarily-deep nest cannot overflow the stack. On a deliberately tiny
        // 512 KiB stack — far too small for the ~3000-frame descent a recursive
        // collect would need — the iterative pass still returns. The clean nest
        // carries no comments.
        let total = crate::test_support::run_on_tiny_stack(|| {
            let n = 3000;
            let src = format!("p({}0{}).\n", "f(".repeat(n), ")".repeat(n));
            let tree = crate::cst::parse(&src);
            attach(&tree, &src).total_attached()
        });
        assert_eq!(total, 0);
    }
}
