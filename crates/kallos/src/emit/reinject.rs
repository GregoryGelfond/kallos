//! Comment re-injection: the emitter filters trivia comments out of its
//! part-building walks and re-injects them, from the `comments::attach` table, at their
//! anchors, in "mode A" — every re-injected comment is followed by a `hardline`, so the
//! next token can never share the comment's line (a `%`-line comment runs to
//! end-of-line). The renderer's lazy newline coalesces that hardline with any following
//! structural break, so a trailing comment in a list renders `e, % c⏎ next` with a
//! single newline. The hardline also forces the enclosing group to explode,
//! while an end-trailing comment on a whole statement rides after it (the trailing
//! hardline is dropped at end-of-file / coalesced with the inter-statement break).
//!
//! A comment's TEXT is a `leaf` (single-line: a line comment or a one-line block,
//! trailing-whitespace stripped) or a `verbatim` (a multi-line `block_comment`:
//! byte-exact, internal newlines literal, exempt from whitespace normalization). The
//! single-line leaf is deliberate: a verbatim reports "can never be flat", which would
//! make `fits` explode a host group merely because a trailing comment sits in its
//! continuation; a leaf carries a real width, and the hardline does the breaking.
//!
//! Calibrated to rustfmt POLS (verified empirically 2026-06-18): a one-space trailing
//! gap; multi-line `block_comment`s kept byte-exact (no re-indent — rustfmt's re-indent
//! is unreliable and unsafe); a single-line `block_comment` between code tokens forces
//! the break (the in-place `Inline` role is reserved for later).

use super::assemble::{Part, Role};
use super::Ctx;
use crate::comments::Trivia;
use crate::cst::KindIds;
use doc::{DocBuilder, NodeId};
use std::ops::Range;
use tree_sitter::Node;

/// One re-injected trivia comment as a Doc node. A **multi-line** `block_comment` (the
/// `multiline` flag) is a byte-exact `verbatim` span: its internal newlines pass
/// through literally (no re-indent — exempt from whitespace normalization), and it
/// forces a break. A **single-line** comment (a line comment, or a one-line block) is a
/// `leaf`, trailing-whitespace stripped (`% n   ` → `% n`). The leaf — not a
/// verbatim — matters: a verbatim reports "can never be flat", so a trailing comment in
/// a group's continuation would make `fits` needlessly explode the host group; a leaf
/// carries a real width instead, and the `hardline` that [`wrap`] places after the
/// comment supplies the forced break (mode A).
pub(super) fn comment_doc(src: &str, t: &Trivia, b: &mut DocBuilder) -> NodeId {
    let start = u32::try_from(t.range.start).expect("a source offset fits in u32 (< 4 GiB)");
    if t.multiline {
        b.verbatim(start, &src[t.range.start..t.range.end])
    } else {
        let end = trim_comment_end(src, &t.range);
        b.leaf(start, &src[t.range.start..end])
    }
}

/// The end offset of `range` in `src` with trailing whitespace removed, so an emitted
/// single-line comment carries none. The result is still a slice of `src`
/// (re-injected text is always source text), just a shorter one.
fn trim_comment_end(src: &str, range: &Range<usize>) -> usize {
    range.start + src[range.start..range.end].trim_end().len()
}

/// Wrap `base` (a child's already-lowered Doc) with the comments anchored to `child`,
/// in "mode A": every re-injected comment is followed by a `hardline`, so the
/// next token can never share the comment's line (a `%`-line comment runs to
/// end-of-line). A leading comment sits on its own line above, with a break BEFORE the
/// run too so it never glues to a preceding same-line token (`hardline ⊕ comment ⊕
/// hardline ⊕ base`); a trailing comment follows `base` with a one-space gap,
/// then its hardline.
/// The renderer's lazy newline coalesces that hardline with any following structural
/// break, so a trailing comment in a list renders `e, % c⏎ next` with a single newline
/// (rustfmt POLS). A clean child (no comments) returns `base` untouched, so it pays
/// nothing. NOTE: `dangling` is NOT read here — a node's dangling comments are injected
/// when the node itself is built (`push_dangling`); this wraps leading/trailing only.
pub(super) fn wrap(ctx: &Ctx, base: NodeId, child: Node, b: &mut DocBuilder) -> NodeId {
    let Some(ac) = ctx.at.for_node(child.id()) else {
        return base;
    };
    if ac.leading.is_empty() && ac.trailing.is_empty() {
        return base;
    }
    let mut items = Vec::with_capacity(ac.leading.len() * 2 + 2 + ac.trailing.len() * 2);
    if !ac.leading.is_empty() {
        // Structural lift: a leading comment always starts on its OWN line, so it
        // never glues to a preceding same-line token (a baked `;`/`,` separator, a flat
        // operator — the cases that were `≈`-but-not-idempotent). The renderer DROPS this
        // break at file start (leading blank → 0) and COALESCES it with any preceding
        // statement / separator break, so a normal leading comment is unaffected; only a
        // mid-line one gains the break that puts it on its own line (and re-parses the
        // same way → idempotent).
        let lead = b.hardline();
        items.push(lead);
    }
    for t in &ac.leading {
        let c = comment_doc(ctx.src, t, b);
        items.push(c);
        let nl = b.hardline();
        items.push(nl);
    }
    items.push(base);
    for t in &ac.trailing {
        let sp = b.space();
        items.push(sp);
        let c = comment_doc(ctx.src, t, b);
        items.push(c);
        // Mode A: a hardline after the comment so the next token never shares its line.
        // The lazy renderer coalesces it with any following structural break, so a list
        // renders `e, % c⏎ next` with a single newline (rustfmt POLS).
        let nl = b.hardline();
        items.push(nl);
    }
    b.seq(&items)
}

/// Whether `node` is a trivia comment (`line_comment` / `block_comment`) — the `extras`
/// filtered from part-building and re-injected at their anchors. `doc_comment` is a
/// statement, not trivia, so it is deliberately excluded and flows through normally.
pub(super) fn is_comment(k: &KindIds, node: Node) -> bool {
    let id = node.kind_id();
    id == k.line_comment || id == k.block_comment
}

/// Walk `node`'s children in source order, skipping trivia comments. Construct rules
/// build their parts from this so a comment is never a stray layout part; its text is
/// re-injected at its anchor (`child_doc` for leading/trailing; `push_dangling` for the
/// enclosing node's dangling) instead.
pub(super) fn walk_code_children<'t>(
    k: &KindIds,
    node: Node<'t>,
    mut visit: impl FnMut(Option<&str>, Node<'t>),
) {
    crate::cst::walk_children(node, |field, child| {
        if !is_comment(k, child) {
            visit(field, child);
        }
    });
}

/// Inject `node`'s DANGLING comments (a comment with no following sibling) as
/// body parts, just before the closer (else before a glued tail run, else appended).
/// Each is an `Element` whose Doc is `hardline ⊕ comment ⊕ hardline` (mode A): the
/// leading hardline breaks it off the previous body line; the trailing hardline keeps
/// the closer / `.` off the comment's line. The renderer's lazy newline coalesces these
/// with the surrounding open / close breaks, so an empty-body bracket renders
/// `{⏎    % c⏎}` with no doubled blank. The assembler stays comment-agnostic — these are
/// ordinary `Element` parts that land in its body slice. A node with no dangling comment
/// inserts nothing, so a clean construct pays only the `for_node` lookup.
pub(super) fn push_dangling(ctx: &Ctx, node: Node, parts: &mut Vec<Part>, b: &mut DocBuilder) {
    let Some(ac) = ctx.at.for_node(node.id()) else {
        return;
    };
    if ac.dangling.is_empty() {
        return;
    }
    // Insert before a trailing `Closer`; else before a trailing run of `Tail`s (a
    // statement `.` / weak-constraint `[…]`); else append (a tail-less, closer-less node).
    let at = parts
        .iter()
        .rposition(|p| p.role == Role::Closer)
        .or_else(|| {
            let body_len = parts
                .iter()
                .rposition(|p| p.role != Role::Tail)
                .map_or(0, |i| i + 1);
            (body_len < parts.len()).then_some(body_len)
        })
        .unwrap_or(parts.len());
    let dangling: Vec<Part> = ac
        .dangling
        .iter()
        .map(|t| {
            let nl1 = b.hardline();
            let c = comment_doc(ctx.src, t, b);
            let nl2 = b.hardline();
            Part {
                role: Role::Element,
                doc: b.seq(&[nl1, c, nl2]),
            }
        })
        .collect();
    let tail = parts.split_off(at);
    parts.extend(dangling);
    parts.extend(tail);
}

#[cfg(test)]
mod tests {
    use super::{comment_doc, trim_comment_end};
    use crate::comments::Trivia;
    use doc::DocBuilder;

    #[test]
    fn trim_comment_end_strips_trailing_whitespace() {
        let src = "% note   \n";
        let range = 0..9; // the "% note   " line_comment span (sans newline)
        assert_eq!(&src[range.start..trim_comment_end(src, &range)], "% note");
    }

    #[test]
    fn trim_comment_end_is_identity_without_trailing_whitespace() {
        assert_eq!(trim_comment_end("% note", &(0..6)), 6);
    }

    #[test]
    fn comment_doc_emits_a_trailing_ws_stripped_single_line() {
        let src = "% note   ";
        let mut b = DocBuilder::new();
        let t = Trivia {
            range: 0..9,
            multiline: false,
        };
        let node = comment_doc(src, &t, &mut b);
        assert_eq!(doc::render(&b.finish(node), src, 80), "% note");
    }

    #[test]
    fn comment_doc_keeps_a_multiline_block_byte_exact() {
        // A multi-line block is verbatim: the internal newline + indentation pass
        // through unchanged, and the (would-be trailing) whitespace is NOT stripped.
        let src = "%* a\n  b *%";
        let mut b = DocBuilder::new();
        let t = Trivia {
            range: 0..src.len(),
            multiline: true,
        };
        let node = comment_doc(src, &t, &mut b);
        assert_eq!(doc::render(&b.finish(node), src, 80), "%* a\n  b *%");
    }
}
