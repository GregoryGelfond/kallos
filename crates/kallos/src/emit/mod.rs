//! The CST → Doc emitter: "identity plus whitespace redistribution".
//! [`lower`] drives an explicit-stack **post-order** walk (no recursion, and the
//! Doc engine it builds into folds its group-break flags at push-time rather than
//! by descent, so input of any depth is safe): each node's children
//! are lowered first,
//! then a per-construct rule ([`constructs`]) assigns [`assemble`] roles and feeds
//! the one generic assembler; every leaf and every still-unhandled or `ERROR` /
//! `MISSING` node emits a verbatim whole-node span through the sole text path
//! [`token`](token::token) (totality). Spacing follows [`spacing`].
//! The foundation is the identity; the per-construct rules flip composites to handled.

mod assemble;
mod constructs;
mod embedded;
mod reinject;
mod spacing;
mod token;

use crate::comments::Attachments;
use crate::cst::KindIds;
use crate::style::Style;
use doc::{Doc, DocBuilder, NodeId};
use rustc_hash::FxHashMap;
use tree_sitter::{Node, Tree};

/// Lower a parsed tree to a [`Doc`] (the emitter entry point). Total: every node
/// is emitted, and any node without a construct rule — plus `ERROR` / `MISSING`
/// nodes — falls back to a verbatim source span. `at` (comment
/// re-injection) and `style` (the assembler) are consumed by the
/// construct rules.
// Reached via the public `format` (below) — the permanent pipeline root — so `lower`, and
// transitively `lower_tree` / `Ctx` / the construct dispatch, is live without its own
// dead-code guard.
pub(crate) fn lower(tree: &Tree, src: &str, at: &Attachments, style: &Style) -> Doc {
    let kinds = KindIds::new();
    let ctx = Ctx {
        src,
        kinds: &kinds,
        at,
        style,
    };
    let mut b = DocBuilder::new();
    let root = lower_tree(&ctx, tree.root_node(), &mut b);
    b.finish(root)
}

/// Format ASP/clingo source — the public formatter entry. Pure (no I/O) and
/// **total**: it always returns a `String` and terminates normally (no panic, no
/// stack-overflow abort) for any `&str`. **Idempotent** on error-free input
/// (best-effort on ERROR-bearing input). On the well-formed fragment the result is
/// `≈` the input — pure whitespace redistribution; input whose parse carries `ERROR`
/// nodes gets best-effort layout with each `ERROR` span preserved verbatim.
///
/// The whole pipeline in one call: parse → attach comments → lower to a [`Doc`] under `style`
/// → render at `style.line_width()` → normalize the file's final newline (a non-empty
/// output ends in exactly one `\n`; an empty or whitespace-only document stays empty). The
/// caller's `style` governs both stages — the assembled layout and the render width.
#[must_use]
pub fn format(src: &str, style: &Style) -> String {
    let tree = crate::cst::parse(src);
    let at = crate::comments::attach(&tree, src);
    let doc = lower(&tree, src, &at, style);
    let mut out = doc::render(&doc, src, style.line_width());
    // A non-empty output ends in EXACTLY one terminating newline; an empty or
    // whitespace-only document stays empty. The renderer already drops a trailing break,
    // so this only ADDS the single newline — whitespace-only and `≈`-safe, and idempotent
    // (a re-format of `…\n` strips and re-adds the same one newline).
    out.truncate(out.trim_end_matches('\n').len());
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// The emitter's read-only context, threaded through the walk.
struct Ctx<'a> {
    src: &'a str,
    kinds: &'a KindIds,
    at: &'a Attachments,
    style: &'a Style,
}

/// Which of a node's two post-order visits a [`Frame`] represents.
enum Visit {
    /// First visit: decide whether to descend (a handled composite) or emit the
    /// node verbatim whole (a leaf / `ERROR` / `MISSING` / unhandled node).
    Pre,
    /// Second visit: every child is in `built`; build this node from them.
    Post,
}

/// A node on the explicit work-list, tagged with its traversal phase and the
/// bracket depth it sits at (which the depth-sensitive spacing consults).
struct Frame<'t> {
    node: Node<'t>,
    bracket_depth: usize,
    visit: Visit,
}

/// Lower the whole tree to a Doc root by an explicit-stack **post-order** walk:
/// each node is visited twice — `Pre` (decide descent) then `Post` (build from the
/// already-lowered children) — so a child's Doc is always in `built` before its
/// parent is assembled. The work-list replaces recursion, so input of any depth
/// returns normally (no stack overflow, no depth bound). A leaf, an
/// `ERROR` / `MISSING` node, or a composite with no construct rule is emitted as
/// one verbatim whole-node span on its `Pre` visit and is not descended into (the
/// foundation identity; the per-construct rules flip composites to handled).
///
/// INVARIANT: when a `Post` frame is popped, every child of its node is already a
/// key in `built` — the `Pre` visit pushes `Post` *under* the children, so the
/// whole child subforest drains first. TERMINATION: each `Pre` either inserts into
/// `built` (the leaf path) or pushes one `Post` plus a `Pre` per child, each for a
/// strictly smaller subtree; the tree is finite, so the work-list empties.
fn lower_tree<'t>(ctx: &Ctx, root: Node<'t>, b: &mut DocBuilder) -> NodeId {
    // Each lowered node's Doc, keyed by `node.id()`. `build` does field-aware role
    // assignment — it re-walks the CST children and looks each one's Doc up by
    // identity — so the map (not a positional results-stack) is the structure that
    // exposes that algorithm; a `None`-decline stays local instead of corrupting an
    // ancestor's stack. O(n) live entries; a per-`Post`-frame child buffer is the
    // optimization if it ever bites (it never does within the 4 GiB source bound).
    let mut built: FxHashMap<usize, NodeId> = FxHashMap::default();
    let mut stack: Vec<Frame<'t>> = vec![Frame {
        node: root,
        bracket_depth: 0,
        visit: Visit::Pre,
    }];
    while let Some(frame) = stack.pop() {
        match frame.visit {
            // Children are all lowered: build this node, or fall to the verbatim
            // default (an unhandled but descended kind — none in practice).
            Visit::Post => {
                let doc = constructs::build(ctx, frame.node, frame.bracket_depth, &built, b)
                    .unwrap_or_else(|| token::token(frame.node, ctx.src, b));
                built.insert(frame.node.id(), doc);
            }
            // No descent: one verbatim whole-node span (totality).
            Visit::Pre if is_verbatim_leaf(ctx, frame.node) => {
                let doc = token::token(frame.node, ctx.src, b);
                built.insert(frame.node.id(), doc);
            }
            // Descend: schedule this node's `Post` under its children (popped L→R).
            Visit::Pre => {
                stack.push(Frame {
                    node: frame.node,
                    bracket_depth: frame.bracket_depth,
                    visit: Visit::Post,
                });
                push_children_reversed(&mut stack, frame.node, frame.bracket_depth, ctx.kinds);
            }
        }
    }
    *built
        .get(&root.id())
        .expect("the root is lowered last, so it is a key in `built`")
}

/// Whether `node` is emitted as one verbatim whole-node span with no descent: a
/// leaf, an `ERROR` / `MISSING` node (the totality backstop), or a composite with
/// no construct rule.
fn is_verbatim_leaf(ctx: &Ctx, node: Node) -> bool {
    node.is_error()
        || node.is_missing()
        || crate::cst::is_leaf(node)
        || !constructs::handles(ctx, node)
}

/// Push `node`'s children onto the work-list in REVERSE source order, so the LIFO
/// stack pops them left-to-right. Each child's bracket depth is computed here,
/// inside the walk, where its grammar `field` label is still live — `child_depth`
/// keys the bracket-entry bump off that field (the grammar's own statement of
/// "inside the brackets"). Children are collected once into a small scratch vector,
/// since `walk_children` drives a forward-only cursor that cannot itself be
/// reversed (O(children) scratch per composite; O(n) total).
fn push_children_reversed<'t>(
    stack: &mut Vec<Frame<'t>>,
    node: Node<'t>,
    parent_depth: usize,
    k: &KindIds,
) {
    let mut framed: Vec<(Node<'t>, usize)> = Vec::new();
    crate::cst::walk_children(node, |field, child| {
        framed.push((child, child_depth(k, node, field, child, parent_depth)));
    });
    for (child, bracket_depth) in framed.into_iter().rev() {
        stack.push(Frame {
            node: child,
            bracket_depth,
            visit: Visit::Pre,
        });
    }
}

/// The bracket depth a `child` of `node` sits at. A child INSIDE a bracketing
/// construct's brackets is one level deeper; every other child stays at the parent's
/// depth. Two signals lift "inside the brackets":
///
/// 1. **The grammar's field label** — the `arguments` of a `function` /
///    `symbolic_atom` / `theory_atom`, the `elements` of an aggregate — keyed off the
///    `field` rather than re-derived from `child.kind()`.
/// 2. **The parent kind, for the unfielded bracket-pairs** — a `tuple` (`( … )`)
///    carries its items with NO field, so its interior is keyed off the parent being
///    a `tuple`, excluding the paren tokens themselves. (`abs` `| … |` is handled the same way.)
///
/// The bump is owned at exactly ONE edge per construct, so no node double-counts; a
/// separated sequence then emits its OWN separators at its own frame depth via
/// `spacing::comma_spacing` / `spacing::term_operator_spacing`.
fn child_depth(
    k: &KindIds,
    node: Node,
    field: Option<&str>,
    child: Node,
    parent_depth: usize,
) -> usize {
    // `elements` is always an aggregate brace list; `arguments` is always a
    // parenthesized argument list. Soundness rests on the CONTENT (`terms`) landing
    // one level deep, which it does. Two field-key imprecisions, both harmless: a
    // `theory_atom`'s `arguments` field also tags its own `(` / `)` / `;`
    // delimiters (so they too get +1 — but they are verbatim, depth-blind tokens),
    // and `doc_comment.arguments` is a verbatim span never descended into. So no
    // node whose depth is ever READ is mis-counted.
    if matches!(field, Some("elements" | "arguments")) {
        return parent_depth + 1;
    }
    // The `tuple` (`( … )`) and `abs` (`| … |`) interiors are unfielded, so key off
    // the parent kind. The bracket tokens themselves sit AT the construct's depth,
    // not inside it, so they are excluded. (abs counts toward depth: `|X-F|` is tight —
    // the interior term op is one bracket deep.)
    let (pid, cid) = (node.kind_id(), child.kind_id());
    if pid == k.tuple && cid != k.lparen && cid != k.rparen {
        return parent_depth + 1;
    }
    if pid == k.abs && cid != k.pipe {
        return parent_depth + 1;
    }
    // The theory `( … )` / `[ … ]` / `{ … }` term brackets carry their
    // interior (the aliased `theory_terms`) with NO field, so — like `tuple` / `abs`
    // — key the bracket-entry bump off the parent kind, excluding the bracket tokens
    // themselves (which sit AT the construct's depth, not inside it). `theory_function`
    // is absent here: its interior rides the `arguments` field, already bumped above.
    if pid == k.theory_tuple && cid != k.lparen && cid != k.rparen {
        return parent_depth + 1;
    }
    if pid == k.theory_list && cid != k.lbracket && cid != k.rbracket {
        return parent_depth + 1;
    }
    if pid == k.theory_set && cid != k.lbrace && cid != k.rbrace {
        return parent_depth + 1;
    }
    parent_depth
}

#[cfg(test)]
mod tests {
    use crate::test_support::format_at_width;

    #[test]
    fn output_ends_in_exactly_one_newline() {
        // A non-empty output ends in EXACTLY one terminating newline (one is
        // added when missing); an empty / whitespace-only document stays empty.
        assert_eq!(format_at_width("a.\n", 80), "a.\n");
        assert_eq!(format_at_width("a.", 80), "a.\n"); // missing final newline added
        assert_eq!(format_at_width("a.\n\n\n", 80), "a.\n"); // trailing blanks → 0
        assert_eq!(format_at_width("% lonely", 80), "% lonely\n");
        assert_eq!(format_at_width("", 80), "");
        assert_eq!(format_at_width("  \n\n ", 80), ""); // whitespace-only → empty
    }

    #[test]
    fn source_file_blank_normalization() {
        // One author blank between statements is preserved; a run collapses to one;
        // no source blank → none; a blank is forced before a #program (but never at file
        // start, which has no preceding statement to separate from).
        assert_eq!(format_at_width("a.\n\nb.\n", 80), "a.\n\nb.\n");
        assert_eq!(format_at_width("a.\n\n\n\nb.\n", 80), "a.\n\nb.\n");
        assert_eq!(format_at_width("a.\nb.\n", 80), "a.\nb.\n");
        assert_eq!(
            format_at_width("a.\n#program p.\nb.\n", 80),
            "a.\n\n#program p.\nb.\n"
        );
        assert_eq!(
            format_at_width("#program p.\na.\n", 80),
            "#program p.\na.\n"
        );
    }

    #[test]
    fn source_file_blank_survives_a_reinjected_comment() {
        // The author blank survives across a re-injected trailing/leading comment —
        // the `carry` threads it onto the item the comment rides with.
        assert_eq!(format_at_width("a. % t\n\nb.\n", 80), "a. % t\n\nb.\n");
        assert_eq!(format_at_width("a.\n\n% c\nb.\n", 80), "a.\n\n% c\nb.\n");
    }

    #[test]
    fn interior_block_blank_normalization() {
        // One author blank between elements of an EXPLODED block is preserved; when
        // the block lays flat the blank vanishes (a one-line form has no blank to keep);
        // and it stays idempotent at the exploding width.
        let wide = "x :- #count{ aaaaaaaaaa : p; bbbbbbbbbb : q;\n\n cccccccccc : r } >= 1.\n";
        let out = format_at_width(wide, 32);
        assert_eq!(
            out.matches("\n\n").count(),
            1,
            "exactly one interior blank when exploded:\n{out}"
        );
        assert_eq!(
            format_at_width("x :- #count{ a;\n\n b } >= 1.\n", 80),
            "x :- #count{ a; b } >= 1.\n"
        );
        let twice = format_at_width(&out, 32);
        assert_eq!(out, twice, "interior blank is idempotent:\n{out}");
    }

    #[test]
    fn interior_comment_after_separator_does_not_phantom_a_blank() {
        // Fidelity: a comment on its own line between a
        // separator and its element must NOT read as an author blank. The no-blank and
        // with-blank variants must produce DISTINCT output (the bug made them identical,
        // erasing the author's intent — invisible to ≈ and idempotence).
        let no_blank = format_at_width(":- a,\n% c\n b.\n", 40);
        let blank = format_at_width(":- a,\n\n% c\n b.\n", 40);
        assert_eq!(no_blank, ":- a,\n   % c\n   b.\n");
        assert_eq!(blank, ":- a,\n\n   % c\n   b.\n");
        assert_ne!(no_blank, blank, "blank-vs-no-blank intent must survive");
        // An author blank BEFORE a separator is preserved too (the carry).
        assert_eq!(
            format_at_width(":- aaaaaaaaaaaaaaaaaaaa\n\n, bbbbbbbbbbbbbbbbbbbb.\n", 20),
            ":- aaaaaaaaaaaaaaaaaaaa,\n\n   bbbbbbbbbbbbbbbbbbbb.\n"
        );
    }

    #[test]
    fn lower_unhandled_kind_is_total() {
        // No construct rule fires yet, so the walk emits verbatim — its source is
        // preserved exactly (the emitter is total over the grammar).
        let out = format_at_width("a :- b.\n", 80);
        assert!(out.contains("a :- b."));
    }

    #[test]
    fn lower_preserves_an_error_span_verbatim() {
        // An ERROR-bearing input is not dropped: its span survives in the output.
        let out = format_at_width("a :- :- .\n", 80);
        assert!(out.contains(":-"));
    }

    #[test]
    fn child_depth_enters_brackets_on_elements_and_arguments_fields() {
        // The aggregate brace list (`elements`) and a parenthesized argument list
        // (`arguments`) count one bracket deeper; a bound / name / anonymous brace
        // stays at the parent depth. `node` / `child` are ignored (the grammar's
        // field label is the whole signal), so a placeholder node suffices.
        use super::child_depth;
        let k = crate::cst::KindIds::new();
        let tree = crate::cst::parse("a.\n");
        let n = tree.root_node();
        // `n` is the source_file root (not a tuple), so only the field-based bumps
        // fire; the parent-kind tuple bump is exercised by the emitter tests.
        assert_eq!(child_depth(&k, n, Some("elements"), n, 0), 1);
        assert_eq!(child_depth(&k, n, Some("arguments"), n, 2), 3);
        assert_eq!(child_depth(&k, n, Some("left"), n, 0), 0);
        assert_eq!(child_depth(&k, n, Some("name"), n, 1), 1);
        assert_eq!(child_depth(&k, n, None, n, 1), 1);
    }

    #[test]
    fn lower_deeply_nested_input_does_not_overflow() {
        // A deep balanced nest must return normally. Since `symbolic_atom` /
        // `function` are handled, this now genuinely DESCENDS 3000 levels
        // (no longer the verbatim no-descent case) — a real regression guard: the
        // per-construct walk is the iterative work-list above, and the Doc engine
        // folds its group-break flags at push-time, so deep input grows the
        // heap-allocated stack, not the call stack. It returns and renders the full
        // structure (outermost `p(` to the innermost `0`), exploded at width 80.
        let n = 3000;
        let src = format!("p({}0{}).\n", "f(".repeat(n), ")".repeat(n));
        let out = format_at_width(&src, 80);
        assert!(out.starts_with("p("), "deep nest must descend and format");
        assert!(out.contains('0'), "the innermost term must be reached");
    }

    #[test]
    fn pipeline_does_not_overflow_on_deeply_nested_input() {
        // The full parse → attach → lower → render pipeline returns on a deeply
        // nested input even on a 512 KiB stack: every walk (comments pre-pass,
        // emitter, renderer) is an explicit work-list, and the Doc engine folds its
        // group-break flags at push-time rather than by descent,
        // so no part of format is call-stack-bound. The emitter now DESCENDS this deep
        // bracket nest (`symbolic_atom` / `function` handled) and 3000 nested
        // `b.group`s build without overflowing the 512 KiB stack — returning proves
        // the build/render are heap-iterative. (The deep *un-grouped operator chain*
        // gets its own guard + the Doc engine's
        // `building_a_group_over_a_deep_chain_is_stack_safe`.)
        let out = crate::test_support::run_on_tiny_stack(|| {
            let n = 3000;
            let src = format!("p({}0{}).\n", "f(".repeat(n), ")".repeat(n));
            format_at_width(&src, 80)
        });
        assert!(out.starts_with("p("), "deep nest must descend and format");
        assert!(out.contains('0'), "the innermost term must be reached");
    }

    #[test]
    fn descended_deep_operator_chain_does_not_overflow() {
        // The operator-chain hazard: a left-nested
        // `binary_operation` chain `1+1+…+1` now DESCENDS (the emitter handles it) and
        // emits a deep ungrouped `Seq`. It must still return on a 512 KiB stack —
        // operators ship FLAT (one group at the enclosing `q(…)`), and `forced_break`
        // is precomputed at push time, so `build` never recurses on chain depth. The
        // chain sits inside `q(…)` (depth 1), so every op tightens (`1+1+…`).
        let out = crate::test_support::run_on_tiny_stack(|| {
            let n = 5000;
            let chain = vec!["1"; n].join("+");
            format_at_width(&format!("p :- q({chain}).\n"), 80)
        });
        assert!(
            out.contains("1+1"),
            "the deep operator chain must descend and stay tight"
        );
    }

    #[test]
    fn theory_descends_deeply_without_overflow() {
        // A deeply-nested theory term `h(h(…k0…))` inside a theory atom must descend and
        // RETURN on a 512 KiB stack — the theory descent rides the SAME iterative
        // work-list as the rest of the emitter (no recursion on tree depth), and
        // forced-break is folded at push time, so build/render stay heap-iterative.
        let out = crate::test_support::run_on_tiny_stack(|| {
            let n = 2000;
            let src = format!("&a {{ {}k0{} }} :- b.\n", "h(".repeat(n), ")".repeat(n));
            format_at_width(&src, 80)
        });
        assert!(
            out.starts_with("&a{"),
            "deep theory nest must descend and format"
        );
        assert!(
            out.contains("k0"),
            "the innermost theory term must be reached"
        );
    }
}
