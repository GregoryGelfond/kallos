//! The relation `≈`: the machine-checkable certificate of the
//! token-stream invariant. Two parse trees are equivalent iff their
//! *structural token streams* agree (node kind, field labels, child order, and at
//! leaves the terminal text — including author-significant anonymous tokens like
//! `,`/`;`/`|`) AND their *comment-text sequences* agree (modulo trailing
//! whitespace).
//!
//! This lives at library scope: it hosts the public self-check `verify`
//! and the `Mismatch` witness it returns, layered on the boolean certificate
//! `≈`. `first_divergence` is the single definition of `≈` — both `verify` and the
//! test-only `equivalent` route through it, so the certificate and its witness cannot
//! disagree.
//!
//! Both walks are iterative explicit-cursor traversals (no recursion), so an
//! arbitrarily deep tree cannot overflow the stack — the certificate obeys
//! the same totality discipline as the formatter it checks.

use crate::style::Style;
use std::ops::Range;
use tree_sitter::{Node, Tree};

/// One element of the structural token stream: a node's field label (if the grammar
/// assigns one), its kind id, the count of its non-comment children, and — at a leaf
/// — its terminal text. Interior nodes carry no text; their identity is their kind,
/// field, child arity, and their (separately recorded) children. Positions and
/// whitespace are deliberately absent.
///
/// `child_count` is what makes the flat pre-order serialization INJECTIVE on tree
/// shape: pre-order + per-node arity uniquely determines a tree, so two streams agree
/// iff the (comment-stripped) trees are structurally identical — not merely equal
/// under some flattening that loses nesting (`R[a[c], b]` ≠ `R[a[c, b]]`). Without it
/// the certificate would rest on the subtle "the parser is deterministic, so equal
/// leaves ⟹ equal trees" argument; lifting arity into the representation makes it
/// self-evidently correct instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Item<'a> {
    field: Option<&'static str>,
    kind: u16,
    child_count: usize,
    text: Option<&'a str>,
}

/// A node's place in the structural token stream: the comparison key (`item`)
/// plus — for witnessing only — the node's source byte range and kind name. Two
/// streams are compared on `item` alone (positions are ignored); `range` and
/// `kind_name` ride alongside so [`first_divergence`] can localize a mismatch to the
/// input without a second tree walk. `Item.kind` (a `u16` id) stays the hot-path
/// equality key; `kind_name` is the cold-path human label, captured for free during
/// the walk (`Node::kind`).
#[derive(Clone, Debug)]
struct Entry<'a> {
    item: Item<'a>,
    range: Range<usize>,
    kind_name: &'static str,
}

/// One side of a structural divergence: a node's kind name, its field label
/// and child arity, and — at a leaf — its terminal text. These mirror the
/// comparison key [`Item`] exactly, so the witness represents precisely what the
/// `≈` check compared: `Display` disambiguates two interior nodes that diverge on
/// field OR arity (rather than both reading as a bare kind name), and
/// the `field` adds locating context (which role the node plays in its parent) to
/// every witness. Owned (a `Mismatch` outlives the borrowed source); crate-private,
/// surfaced only through `Display`/`Debug` on the enclosing [`Mismatch`], which
/// boxes this witness so the common `Ok` path keeps a small `Result`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NodeWitness {
    kind: &'static str,
    field: Option<&'static str>,
    child_count: usize,
    text: Option<String>,
}

/// The private witness for [`Mismatch::Structural`]: the byte range in the INPUT
/// localizing the first diverging node, and the diverging node on each side (`None`
/// where one stream is exhausted — an ERROR-domain length divergence). Fields
/// are crate-private; `Display` surfaces them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralMismatch {
    range: Range<usize>,
    expected: Option<NodeWitness>,
    found: Option<NodeWitness>,
}

/// The private witness for [`Mismatch::Comment`] (the comment conjunct): the
/// index in the in-order comment sequence and the expected/found comment text (`None`
/// where one sequence is shorter). Fields are crate-private; `Display` surfaces them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentMismatch {
    index: usize,
    expected: Option<String>,
    found: Option<String>,
}

/// What [`verify`] witnesses when `format(s)` is not `≈` to `s`.
///
/// On error-free input `verify` always returns `Ok(())` — the `≈` guarantee holds by
/// construction — so a `Mismatch` localizes a divergence in the ERROR
/// recovery regime, the one regime where `≈` may legitimately fail (reflow around a
/// preserved `ERROR` span can move tree-sitter's error boundary on re-parse).
///
/// `#[non_exhaustive]`: an `Embedded` variant lands when ruff/stylua embedded-code
/// formatting ships. The payloads are crate-private — `--safe` keys on the
/// `Ok`/`Err` discrimination, not the witness fields (error *locations* are a
/// diagnostics concern) — but each carries a localizing witness, surfaced via
/// `Display` / `Debug`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mismatch {
    /// The structural token streams diverged: node kind, field label,
    /// child arity, or leaf text. Boxed so the (common) `Ok` path keeps a small
    /// `Result` (`clippy::result_large_err`).
    Structural(Box<StructuralMismatch>),
    /// The in-order comment-text sequences diverged (comment conjunct).
    Comment(CommentMismatch),
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::Structural(s) => {
                write!(
                    f,
                    "structural divergence at input bytes {}..{}: expected ",
                    s.range.start, s.range.end
                )?;
                fmt_node(f, s.expected.as_ref())?;
                write!(f, ", found ")?;
                fmt_node(f, s.found.as_ref())
            }
            Mismatch::Comment(c) => write!(
                f,
                "comment #{} diverged: expected {:?}, found {:?}",
                c.index,
                c.expected.as_deref().unwrap_or("<none>"),
                c.found.as_deref().unwrap_or("<none>"),
            ),
        }
    }
}

impl std::error::Error for Mismatch {}

/// Render one side of a structural witness for [`Mismatch`]'s `Display`. A leaf
/// shows its kind and terminal text; an interior node shows its kind and child
/// arity (so two interior nodes diverging only on arity read distinctly);
/// a field label, when present, prefixes either form.
fn fmt_node(f: &mut std::fmt::Formatter<'_>, node: Option<&NodeWitness>) -> std::fmt::Result {
    match node {
        None => write!(f, "<none>"),
        Some(n) => {
            if let Some(field) = n.field {
                write!(f, "{field}: ")?;
            }
            match &n.text {
                Some(t) => write!(f, "{} {t:?}", n.kind),
                None => write!(f, "{} ({} children)", n.kind, n.child_count),
            }
        }
    }
}

/// The comment `extras` removed before the structural comparison: the two
/// trivia comment kinds. `doc_comment` is a first-class statement, NOT trivia, so it
/// is deliberately absent here and flows through the structural stream.
fn is_comment_extra(node: Node) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}

/// The number of NON-comment children of `node` — exactly the count of child items
/// `token_stream` emits for it. Comments are excluded so the structural comparison is
/// insensitive to comment placement (comments are compared separately, via the
/// comment conjunct), and so the recorded arity matches the recorded subtrees.
fn noncomment_child_count(node: Node) -> usize {
    let mut n = 0;
    crate::cst::walk_children(node, |_, child| {
        if !is_comment_extra(child) {
            n += 1;
        }
    });
    n
}

/// The structural token stream of `tree`: a pre-order serialization of the FULL
/// cursor child sequence (named AND anonymous tokens) with comment extras removed,
/// each node tagged with its non-comment arity (so the flat stream is injective on
/// tree shape — see [`Item`]). Each [`Entry`] also carries the node's source byte
/// range and kind name for witnessing (ignored by the `≈` comparison, which is on
/// `Entry.item` alone).
fn token_stream<'a>(tree: &Tree, src: &'a str) -> Vec<Entry<'a>> {
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if !is_comment_extra(node) {
            let text = (node.child_count() == 0).then(|| &src[node.byte_range()]);
            out.push(Entry {
                item: Item {
                    field: cursor.field_name(),
                    kind: node.kind_id(),
                    child_count: noncomment_child_count(node),
                    text,
                },
                range: node.byte_range(),
                kind_name: node.kind(),
            });
            if cursor.goto_first_child() {
                continue;
            }
        }
        // Leaf, or a skipped comment: advance to the next sibling, or climb until one
        // exists; climbing past the root ends the walk.
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
        }
    }
}

/// The in-order sequence of comment token texts (the comment conjunct), each
/// trimmed of trailing whitespace — the formatter strips trailing whitespace inside
/// single-line comments, so a byte-exact conjunct would false-fail on the
/// formatter's own output. Sequence, not multiset: pure layout never reorders
/// comments, so the stronger invariant is free and catches a transposition bug.
fn comment_stream<'a>(tree: &Tree, src: &'a str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if is_comment_extra(node) {
            out.push(src[node.byte_range()].trim_end());
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
        }
    }
}

/// The first divergence between two parse trees under `≈`, or `None` when
/// `T_a ≈ T_b`. The structural conjunct (the primary token-stream invariant)
/// is checked before the comment conjunct, so a structural witness is preferred. The
/// witness localizes to `src_a`, treated as the INPUT / "expected" side (`src_b` is
/// the "found" side).
///
/// PRECONDITION: both trees were parsed from their respective `src` with the SAME
/// grammar, so kind ids and names align across the two trees.
/// POSTCONDITION: returns `None` iff the two trees are `≈` — so `equivalent` below is
/// exactly `first_divergence(..).is_none()` (one definition of `≈`).
/// TERMINATION: each stream is a finite `Vec`; the two scans are bounded index walks.
pub(crate) fn first_divergence(
    tree_a: &Tree,
    src_a: &str,
    tree_b: &Tree,
    src_b: &str,
) -> Option<Mismatch> {
    // Structural conjunct first. Walk both streams in lockstep; the first index
    // whose keys differ — or at which one stream is exhausted (an ERROR-domain length
    // divergence) — is the witness. (`0..max(len_a, len_b)` is not a single
    // collection's `len()`, so `needless_range_loop` does not apply.)
    let a = token_stream(tree_a, src_a);
    let b = token_stream(tree_b, src_b);
    for i in 0..a.len().max(b.len()) {
        let (ea, eb) = (a.get(i), b.get(i));
        if let (Some(x), Some(y)) = (ea, eb) {
            if x.item == y.item {
                continue;
            }
        }
        // For any two real single-rooted parse trees, `ea` and `eb` are both `Some` here:
        // the `child_count`-keyed serialization is self-terminating, so a structural
        // difference always surfaces as a both-present key mismatch BEFORE either stream
        // exhausts (the comment stream below, being flat, is where exhaustion is actually
        // reachable). The `None`-side handling — an empty range at end-of-input, a `None`
        // witness side — is defensive: it keeps the function total on the
        // impossible exhaustion case rather than panicking.
        return Some(Mismatch::Structural(Box::new(StructuralMismatch {
            range: ea.map_or(src_a.len()..src_a.len(), |e| e.range.clone()),
            expected: ea.map(node_witness),
            found: eb.map(node_witness),
        })));
    }
    // Comment conjunct: in-order comment texts, modulo trailing whitespace.
    let ca = comment_stream(tree_a, src_a);
    let cb = comment_stream(tree_b, src_b);
    for i in 0..ca.len().max(cb.len()) {
        let (xa, xb) = (ca.get(i), cb.get(i));
        if xa == xb {
            continue;
        }
        return Some(Mismatch::Comment(CommentMismatch {
            index: i,
            expected: xa.map(|&s| s.to_string()),
            found: xb.map(|&s| s.to_string()),
        }));
    }
    None
}

/// Build one side of a structural witness from a stream entry, owning the leaf text
/// (the `Mismatch` outlives the borrowed source).
fn node_witness(e: &Entry) -> NodeWitness {
    NodeWitness {
        kind: e.kind_name,
        field: e.item.field,
        child_count: e.item.child_count,
        text: e.item.text.map(str::to_string),
    }
}

/// The pure-layout self-check: parse → format → re-parse → compare the
/// two trees under `≈`. Returns `Ok(())` when the formatted output is `≈` the
/// input, the witnessing [`Mismatch`] otherwise.
///
/// It does NOT short-circuit on parse errors. On error-free input `Ok(())` holds by
/// construction; ERROR-bearing input is the one regime where it may legitimately
/// return a `Mismatch` (reflow can move tree-sitter's error boundary on re-parse) —
/// the signal the CLI's `--safe` mode keys on. It is total: `format` always returns,
/// so `verify` always returns. It needs no solver, no grounding, and no external oracle.
///
/// Standalone and callable on a bare `&str`: the test harness and the CLI's `--safe`
/// mode depend on this exact shape. (It calls the public `format`.)
///
/// # Errors
///
/// Returns the witnessing [`Mismatch`] when `format(src)` does not re-parse `≈` to `src`.
/// By construction this never happens on error-free input; it is reachable only in
/// the ERROR-recovery regime, where reflow around a preserved `ERROR` span can move
/// tree-sitter's error boundary on re-parse.
pub fn verify(src: &str, style: &Style) -> Result<(), Mismatch> {
    // `format` runs first (totality), exercising the formatter even on ERROR
    // input before the `≈` comparison — there is no `has_error` short-circuit.
    let formatted = crate::emit::format(src, style);
    let tree_in = crate::cst::parse(src);
    let tree_out = crate::cst::parse(&formatted);
    match first_divergence(&tree_in, src, &tree_out, &formatted) {
        None => Ok(()),
        Some(mismatch) => Err(mismatch),
    }
}

/// `T_a ≈ T_b`: structural token streams equal AND comment-text sequences
/// equal (modulo trailing whitespace). Defined as "no first divergence" so there is a
/// single definition of `≈` (the certificate and its witness cannot disagree).
#[cfg(test)]
pub(crate) fn equivalent(tree_a: &Tree, src_a: &str, tree_b: &Tree, src_b: &str) -> bool {
    first_divergence(tree_a, src_a, tree_b, src_b).is_none()
}

/// The result of a round-trip `≈` check, scoped to the error domain. Test-only: it is
/// the property-test view (with the `InputHasError` short-circuit and the formatted
/// output for failure messages), distinct from the public `verify`'s `Result`.
#[cfg(test)]
#[derive(Debug)]
pub(crate) enum Outcome {
    /// Error-free input whose formatted output re-parses `≈` to it (the full guarantee).
    Equivalent,
    /// Input whose parse carries `ERROR`/`MISSING` nodes: the contract is totality +
    /// verbatim error-span preservation, NOT full `≈`. The format ran without
    /// panicking; we deliberately do not assert `≈` here.
    InputHasError,
    /// Error-free input whose output is NOT `≈` to it — a real violation. Carries the
    /// formatted output for the failure message.
    NotEquivalent { formatted: String },
}

/// Format `src` at `width`, re-parse, and classify the round trip against `≈`. The
/// format ALWAYS runs (exercising totality by returning); the `≈` assertion is
/// gated to error-free input. The property-test view of the round
/// trip; it shares the `first_divergence` core with the public [`verify`] but keeps the
/// `InputHasError` short-circuit (the property tests skip the ERROR domain).
#[cfg(test)]
pub(crate) fn roundtrip_outcome(src: &str, width: usize) -> Outcome {
    let out = crate::test_support::format_at_width(src, width);
    let tree_in = crate::cst::parse(src);
    if tree_in.root_node().has_error() {
        return Outcome::InputHasError;
    }
    let tree_out = crate::cst::parse(&out);
    if equivalent(&tree_in, src, &tree_out, &out) {
        Outcome::Equivalent
    } else {
        Outcome::NotEquivalent { formatted: out }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        equivalent, first_divergence, roundtrip_outcome, verify, Mismatch, NodeWitness, Outcome,
        StructuralMismatch,
    };
    use crate::cst::parse;
    use crate::style::Style;
    use proptest::prelude::*;

    /// The proptest case count for the battle-hardened safety nets. The `≈`
    /// round-trip nets, the totality fuzzers, and the generator-validity net all run at
    /// this aggressive default — chosen so the whole module finishes in a few seconds
    /// (aggressive, not painful). `PROPTEST_CASES=N` overrides it for heavier ad-hoc
    /// stress runs (it takes precedence over `with_cases`).
    const NET_CASES: u32 = 2048;

    #[test]
    fn verify_agrees_with_roundtrip_on_error_free_input() {
        // The public wrapper and the test-only certificate agree on error-free input:
        // both report `≈`. (Their domains diverge only on ERROR input.)
        for src in [
            "a :- b, c.\n",
            "p(X) :- X = (a; b).\n",
            ":~ p(X). [1@2, X]\n",
        ] {
            for width in [8usize, 100] {
                assert_eq!(
                    verify(src, &Style::default().with_line_width(width)),
                    Ok(()),
                    "verify must be Ok on error-free {src:?} @ {width}"
                );
                assert!(matches!(roundtrip_outcome(src, width), Outcome::Equivalent));
            }
        }
    }

    #[test]
    fn first_divergence_localizes_a_structural_mismatch() {
        // `a :- b.` vs `a :- c.` agree structurally up to the `identifier` leaf, which
        // diverges in text. The witness carries the INPUT byte range (5..6, where `b`
        // sits in "a :- b.") and the (kind, text) on each side. (`b`/`c` parse as
        // `function name: (identifier)`; the diverging leaf is the `identifier`.)
        let m = first_divergence(&parse("a :- b."), "a :- b.", &parse("a :- c."), "a :- c.")
            .expect("a differing leaf must witness a Structural mismatch");
        let Mismatch::Structural(s) = m else {
            panic!("expected Structural, got {m:?}");
        };
        assert_eq!(s.range, 5..6);
        let expected = s.expected.as_ref().expect("input side present");
        let found = s.found.as_ref().expect("output side present");
        assert_eq!(expected.kind, "identifier");
        assert_eq!(expected.text.as_deref(), Some("b"));
        assert_eq!(found.text.as_deref(), Some("c"));
    }

    #[test]
    fn first_divergence_localizes_a_comment_mismatch() {
        // Structural streams are identical (comments are stripped from the structural
        // stream), so the witness is the Comment conjunct at index 0.
        let m = first_divergence(&parse("a. % hi"), "a. % hi", &parse("a. % bye"), "a. % bye")
            .expect("a differing comment must witness a Comment mismatch");
        let Mismatch::Comment(c) = m else {
            panic!("expected Comment, got {m:?}");
        };
        assert_eq!(c.index, 0);
        assert_eq!(c.expected.as_deref(), Some("% hi"));
        assert_eq!(c.found.as_deref(), Some("% bye"));
    }

    #[test]
    fn first_divergence_is_none_when_equivalent() {
        // Same tokens, different inter-token spacing → ≈ → no witness.
        assert!(first_divergence(&parse("a :- b."), "a :- b.", &parse("a:-b."), "a:-b.").is_none());
        // Same comment modulo trailing whitespace → ≈.
        assert!(first_divergence(
            &parse("a. % hi"),
            "a. % hi",
            &parse("a. % hi   "),
            "a. % hi   ",
        )
        .is_none());
    }

    #[test]
    fn first_divergence_witnesses_a_child_count_divergence() {
        // A rule-with-body vs a bare fact: both parse to a `rule` node, but with different
        // child arity. The divergence is a both-present `rule`/`rule` mismatch (the
        // `child_count` half of the key differs) — NOT a leaf-text difference —
        // localized to the rule's INPUT byte range.
        let m = first_divergence(&parse("a :- b."), "a :- b.", &parse("a."), "a.")
            .expect("differing child arity must witness a Structural mismatch");
        let Mismatch::Structural(s) = m else {
            panic!("expected Structural, got {m:?}");
        };
        assert_eq!(s.range, 0..7); // the whole `a :- b.` rule, in the INPUT
        let expected = s.expected.as_ref().expect("input side present");
        let found = s.found.as_ref().expect("output side present");
        assert_eq!(expected.kind, "rule");
        assert_eq!(found.kind, "rule");
        assert!(
            expected.text.is_none(),
            "an interior node carries no leaf text"
        );
    }

    #[test]
    fn first_divergence_witnesses_comment_stream_exhaustion() {
        // The reachable length-divergence branch: the input carries a trailing comment the
        // other side lacks. Structural streams are identical (comments are stripped),
        // so the witness is the Comment conjunct at index 0 with the `found` side exhausted
        // (`None`). (The *structural* None-side is unreachable — a child_count-keyed tree
        // serialization is self-terminating — so it has no real-input test; see the
        // defensive note in `first_divergence`.)
        let m = first_divergence(&parse("a. %x"), "a. %x", &parse("a."), "a.")
            .expect("a trailing comment with no counterpart must witness a Comment mismatch");
        let Mismatch::Comment(c) = m else {
            panic!("expected Comment, got {m:?}");
        };
        assert_eq!(c.index, 0);
        assert_eq!(c.expected.as_deref(), Some("%x"));
        assert!(
            c.found.is_none(),
            "the shorter (output) comment sequence is exhausted"
        );
    }

    #[test]
    fn mismatch_display_is_localizing() {
        let m = first_divergence(&parse("a :- b."), "a :- b.", &parse("a :- c."), "a :- c.")
            .expect("structural witness");
        let s = m.to_string();
        assert!(
            s.contains("5..6"),
            "structural Display carries the byte range: {s}"
        );
        assert!(
            s.contains("\"b\"") && s.contains("\"c\""),
            "and the diverging text on each side: {s}"
        );
        let cm = first_divergence(&parse("a. % hi"), "a. % hi", &parse("a. % bye"), "a. % bye")
            .expect("comment witness");
        let cs = cm.to_string();
        assert!(
            cs.contains("% hi") && cs.contains("% bye"),
            "comment Display carries both texts: {cs}"
        );
    }

    #[test]
    fn display_distinguishes_interior_nodes_by_child_count() {
        // Two interior nodes (no leaf text) that differ only in arity
        // must NOT both render as a bare kind name — the child counts disambiguate
        // the otherwise-identical `expected ERROR, found ERROR` reading.
        let m = Mismatch::Structural(Box::new(StructuralMismatch {
            range: 0..5,
            expected: Some(NodeWitness {
                kind: "rule",
                field: None,
                child_count: 3,
                text: None,
            }),
            found: Some(NodeWitness {
                kind: "rule",
                field: None,
                child_count: 4,
                text: None,
            }),
        }));
        let s = m.to_string();
        assert!(
            s.contains("rule (3 children)"),
            "expected side carries arity: {s}"
        );
        assert!(
            s.contains("rule (4 children)"),
            "found side carries arity: {s}"
        );
    }

    #[test]
    fn display_carries_the_field_label() {
        // When the diverging nodes sit in named fields the
        // field label is shown, so a divergence differing ONLY in field reads
        // distinctly on each side (not identically), and every witness gains the
        // locating context of which role the node plays in its parent.
        let m = Mismatch::Structural(Box::new(StructuralMismatch {
            range: 0..3,
            expected: Some(NodeWitness {
                kind: "term",
                field: Some("head"),
                child_count: 1,
                text: None,
            }),
            found: Some(NodeWitness {
                kind: "term",
                field: Some("body"),
                child_count: 1,
                text: None,
            }),
        }));
        let s = m.to_string();
        assert!(
            s.contains("head: term"),
            "expected side shows its field: {s}"
        );
        assert!(s.contains("body: term"), "found side shows its field: {s}");
    }

    #[test]
    fn roundtrip_preserves_error_free_input() {
        assert!(matches!(
            roundtrip_outcome("a :- b, c.\n", 80),
            Outcome::Equivalent
        ));
        assert!(matches!(
            roundtrip_outcome(":- d(X), e(X).\n", 10),
            Outcome::Equivalent
        ));
    }

    #[test]
    fn roundtrip_skips_assertion_for_error_input() {
        // ERROR-bearing input gets totality only (no panic), not full ≈.
        assert!(matches!(
            roundtrip_outcome("a :- :- .\n", 80),
            Outcome::InputHasError
        ));
    }

    #[test]
    fn roundtrip_preserves_family4_arity_and_pools() {
        // The forms whose spacing changed must still round-
        // trip ≈ at every width — a synthesized or dropped comma is an arity
        // violation, a fused operator/pool seam is a token violation. (Each input is
        // error-free, so a non-`Equivalent` here is a real defect, not a parse skip.)
        for src in [
            "p(X) :- X = (a,).\n",   // 1-tuple — comma is semantic
            "p :- q = ().\n",        // 0-tuple
            "p :- f(a; b).\n",       // argument pool
            "p(X) :- X = (a; b).\n", // tuple pool
            "p :- q(X, g(Y,Z)).\n",  // depth-2 comma tighten
            "p :- g(f(a; b)).\n",    // depth-2 pool `;` (the produced Tight `;`)
            "p(X) :- X = 1..3.\n",   // interval
            "p :- q(1+-2*2**0).\n",  // tight operator chain
            "p(X) :- X = |a; b|.\n", // abs pool
            "a :- p :.\n",           // empty-condition conditional `:` abutting `.`
        ] {
            for width in [1usize, 8, 100] {
                assert!(
                    matches!(roundtrip_outcome(src, width), Outcome::Equivalent),
                    "≈ violation at width {width}: {src:?}"
                );
            }
        }
    }

    #[test]
    fn roundtrip_preserves_family5_theory() {
        // The theory forms whose spacing changed must still round-trip ≈
        // at every width — an always-spaced operator must not fuse a seam, a normalized
        // comma must not be synthesized or dropped, the `#theory` definition layout must
        // preserve every token. (Each input is error-free, so a non-`Equivalent` is a
        // real defect, not a parse skip.)
        for src in [
            "&a { x - y } <= 3 :- b.\n",      // binary op + the `op term` upper
            "&a { - x } :- b.\n",             // unary (always-spaced)
            "&a { x + + y } :- b.\n",         // adjacent operators never fuse (`+ +`≠`++`)
            "&a { x : -p } :- b.\n",          // element `:` before a sign never fuses to `:-`
            "&a { f(x, y); (p, q) } :- b.\n", // theory function + tuple elements
            "&a { [x]; {y} } :- b.\n",        // theory list + set
            "#theory t { d { - : 0, binary, left }; &a/0 : d, any }.\n", // #theory definition
            "#theory t { &a/0 : trm, {<=, >=}, g, body }.\n", // guard set (theory-op vs `{ } ,`)
        ] {
            for width in [1usize, 8, 100] {
                assert!(
                    matches!(roundtrip_outcome(src, width), Outcome::Equivalent),
                    "≈ violation at width {width}: {src:?}"
                );
            }
        }
    }

    #[test]
    fn roundtrip_preserves_family6_directives() {
        // The directive forms whose spacing changed must round-trip ≈ at every
        // width — a glued dot, a spaced keyword, a normalized signature/weight, the verbatim
        // #script body, must all preserve every token. (Each input is error-free.)
        for src in [
            "#show p/1. [true]\n",
            "#defined q/2.\n",
            "#const n = 1+2. [override]\n",
            "#external p(X) : q(X). [false]\n",
            "#heuristic a(X) : b(X). [2@1, sign]\n",
            "#edge (a, b; c, d).\n",
            "#program acid(k, t).\n",
            "#include <incmode>.\n",
            ":~ p(X), q(X). [1@2, X]\n",
            "#script (python)\nx = 1\n#end.\n",
        ] {
            for width in [1usize, 8, 100] {
                assert!(
                    matches!(roundtrip_outcome(src, width), Outcome::Equivalent),
                    "≈ violation at width {width}: {src:?}"
                );
            }
        }
    }

    #[test]
    fn equivalent_ignores_whitespace_but_not_tokens() {
        // Same tokens, different inter-token spacing → ≈.
        assert!(equivalent(
            &parse("a :- b."),
            "a :- b.",
            &parse("a:-b."),
            "a:-b."
        ));
        // A different leaf token → not ≈.
        assert!(!equivalent(
            &parse("a :- b."),
            "a :- b.",
            &parse("a :- c."),
            "a :- c."
        ));
        // An author-significant anonymous token (',' vs ';') → not ≈.
        assert!(!equivalent(
            &parse("p(a,b)."),
            "p(a,b).",
            &parse("p(a;b)."),
            "p(a;b)."
        ));
    }

    #[test]
    fn equivalent_compares_comment_sequence_modulo_trailing_ws() {
        // Same comment modulo trailing whitespace → ≈.
        assert!(equivalent(
            &parse("a. % hi"),
            "a. % hi",
            &parse("a. % hi   "),
            "a. % hi   ",
        ));
        // Different comment text → not ≈.
        assert!(!equivalent(
            &parse("a. % hi"),
            "a. % hi",
            &parse("a. % bye"),
            "a. % bye",
        ));
    }

    #[test]
    fn roundtrip_preserves_step7_comments() {
        // Re-injecting comments must keep the output `≈` to the input at
        // every width — the token stream AND the comment-text sequence (modulo trailing
        // whitespace) both preserved. Covers leading / trailing / interior re-injection,
        // the source_file bottom anchor (comment-only file, post-final-statement), and
        // the still-degraded cases (a dangling comment inside a bracket — verbatim, also
        // `≈`). Each input is error-free, so a non-`Equivalent` here is a real defect.
        for src in [
            "a :- b. % trailing\n",
            "% leading\na :- b.\n",
            "% one\n% two\na.\n",
            "a :- b, % interior\nc.\n",
            "a :- %* inline *% b.\n",
            "%* multi\n  line *%\np.\n",
            "% lonely\n",
            "a.\n% tail\n",
            "a :- #count{ p; q\n% dangling\n}.\n",
            "p(a, b\n% after\n).\n",
            "a :- b\n% c\n.\n",
            "p(\n% only\n).\n",
            // Blank-line-detached comments at the top / middle of the file must keep
            // their source position (not relocate to the bottom and reorder past a
            // later comment — the detach-to-root-dangling reorder bug).
            "%d1\n\np. %t\n",
            "%d\n\np.\n%t2\nq.\n",
            "%a\n\n%b\np. %t\n",
            // Formerly imperfect-layout warts (a comment inside a flat operator / before a
            // separator): the structural lift now puts each leading comment on its own
            // line. Still `≈`, and now clean — see the goldens below.
            "p :- X = 1 + % c\n2.\n",
            "p(a % c\n, b).\n",
            // Transposition fix (sequence conjunct): an own-line comment between
            // an element and a FOLLOWING separator that carries a trailing comment must NOT
            // jump the separator (which reordered the pair → CommentMismatch). The `;`
            // aggregate-element list (the minimal repro) and the `,` rule body.
            "a :- #sum { x : p\n%bb\n; %aa\ny : q } >= 0.\n",
            "a :- p\n%bb\n, %aa\nq.\n",
            // Transposition fix — class extension to the whole "leading comment before
            // a following anonymous token" family: an own-line comment between an
            // element and a FOLLOWING connective neck (`:-`/`:~`/`:`), bracket opener
            // (`{`/`(`/`[`), infix (`=`/`@`), or disjunction-head `|` separator that
            // carries a trailing comment must NOT jump that token. The rule `:-`, the
            // conditional `:`, the aggregate `{` opener, the `#const` `=`, the weight `@`,
            // and the disjunction `|` (whose abs-delimiter twin keeps dangling — last case).
            "a\n%bb\n:- %aa\nb.\n",
            "p :- q\n%bb\n: %aa\nr.\n",
            ":- #count\n%bb\n{ %aa\nx : p } >= 1.\n",
            "#const c\n%bb\n= %aa\n5.\n",
            ":~ b. [2\n%bb\n@ %aa\n1]\n",
            // The disjunction-head `|` separator is a leading anchor too (the |-class
            // completion); its abs-delimiter twin is NOT — a comment before a closing abs
            // `|` dangles inside, never leading the bar (the carve-out, last case).
            "a\n%bb\n| %aa\nb.\n",
            "p(X)\n%bb\n| %aa\nq(X) :- r(X).\n",
            "a :- X = |Y\n% c\n|.\n",
            // Idempotence: the example.lp drift region — a neck comment
            // ABOVE a nested aggregate whose element-list comment drifted indent on reformat
            // (12 then 8). Coupled to the neck mis-attachment; fixed by the same change.
            "a\n%bb\n:- %aa\n#sum { x : p\n%dd\n; y : q } >= 0.\n",
        ] {
            for width in [1usize, 8, 100] {
                assert!(
                    matches!(roundtrip_outcome(src, width), Outcome::Equivalent),
                    "≈ violation at width {width}: {src:?}"
                );
            }
        }
    }

    #[test]
    fn comment_before_separator_preserves_source_order() {
        // Sequence conjunct (transposition fix): an own-line comment that sits
        // between an element and a FOLLOWING `;`/`,` separator whose own line carries a
        // trailing comment must emit BEFORE that trailing comment — it must NOT jump the
        // separator to lead the next element (which reordered the pair, so `verify`
        // returned `CommentMismatch`). The `;` aggregate-element list (the minimal
        // repro) and the `,` rule body are both exercised, at every width.
        for src in [
            "a :- #sum { x : p\n%bb\n; %aa\ny : q } >= 0.\n", // `;` — minimal repro
            "a :- p\n%bb\n, %aa\nq.\n",                       // `,` body analogue
        ] {
            for w in [1usize, 8, 100] {
                assert_eq!(
                    verify(src, &Style::default().with_line_width(w)),
                    Ok(()),
                    "comment source-order must round-trip ≈ @ width {w}: {src:?}"
                );
            }
        }
    }

    #[test]
    fn comment_before_connective_neck_preserves_source_order() {
        // Sequence conjunct (transposition fix, class extension): an own-line
        // comment that sits between an element and a FOLLOWING connective NECK
        // (`:-` / `:~` / the conditional `:`) whose own line carries a trailing comment
        // must emit BEFORE that trailing comment — it must NOT jump the neck (which
        // reordered the pair, so `verify` returned `CommentMismatch`). The same class the
        // `,`/`;` separator fix closed, now at the neck tokens. Idempotence is asserted
        // separately by the adversarial net; here the load-bearing property is `≈`.
        for src in [
            "a\n%bb\n:- %aa\nb.\n",               // rule neck `:-`
            "p :- q\n%bb\n: %aa\nr.\n",           // conditional `:`
            ":~ p(X)\n%bb\n: %aa\nq(X). [1@2]\n", // conditional `:` inside a weak body
        ] {
            for w in [1usize, 8, 100] {
                assert_eq!(
                    verify(src, &Style::default().with_line_width(w)),
                    Ok(()),
                    "comment source-order must round-trip ≈ @ width {w}: {src:?}"
                );
            }
        }
    }

    #[test]
    fn flat_operator_trailing_comment_has_no_stray_leading_space() {
        // Formerly a layout wart: a trailing comment inside a flat operator forces a
        // break; the operand that follows lands at the block indent with NO stray leading
        // space — the operator's spacing space, stranded at line start, is dropped.
        assert_eq!(
            crate::test_support::format_at_width("p :- X = 1 + % c\n2.\n", 80),
            "p :-\n    X = 1 + % c\n    2.\n"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(NET_CASES))]
        /// The generator must emit error-free programs (else the round-trip test below
        /// silently passes via `InputHasError`). A regression here means a generator
        /// production is invalid — fix the generator, not this test.
        #[test]
        fn generator_emits_parsable_programs(prog in crate::test_support::valid_program()) {
            prop_assert!(
                !crate::cst::has_error(&prog),
                "generator emitted a non-parsing program:\n{prog}"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(NET_CASES))]
        /// THE safety net: formatting a valid program at any width
        /// preserves its token stream. This is the guard that fires the moment the
        /// formatter emits a Tight join that would merge or drop a token.
        #[test]
        fn formatting_preserves_the_token_stream(prog in crate::test_support::valid_program()) {
            prop_assume!(!crate::cst::has_error(&prog));
            for width in [1usize, 8, 20, 100] {
                match roundtrip_outcome(&prog, width) {
                    Outcome::Equivalent => {}
                    Outcome::InputHasError => prop_assume!(false),
                    Outcome::NotEquivalent { formatted } => prop_assert!(
                        false,
                        "≈ VIOLATION at width {width}:\n--- input ---\n{prog}\n--- output ---\n{formatted}"
                    ),
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(NET_CASES))]
        /// The comment safety net: formatting a COMMENT-bearing program
        /// at any width preserves both its token stream AND its comment-text sequence
        /// (modulo trailing whitespace). Shallow statements + rich comment decoration
        /// (leading / trailing / standalone / blank-detached), so the net stresses
        /// re-injection and comment ORDER — the class the detached-comment reorder bug
        /// violated — without the deep-term generation that overflows the test stack.
        #[test]
        fn formatting_preserves_comments(prog in crate::test_support::commented_program()) {
            prop_assume!(!crate::cst::has_error(&prog));
            for width in [1usize, 8, 100] {
                match roundtrip_outcome(&prog, width) {
                    Outcome::Equivalent => {}
                    Outcome::InputHasError => prop_assume!(false),
                    Outcome::NotEquivalent { formatted } => prop_assert!(
                        false,
                        "≈ COMMENT VIOLATION at width {width}:\n--- input ---\n{prog}\n--- output ---\n{formatted}"
                    ),
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(NET_CASES))]
        /// The transposition-class regression net. The shallow
        /// [`crate::test_support::commented_program`] net above decorates whole statements
        /// and so never reaches INTERNAL token boundaries — the exact placement the
        /// comment-transposition bug class lived in (a leading comment on its own line
        /// before an anonymous token that itself carries a trailing comment, for `,` `;`
        /// `|` `:-` `:~` `:` `{` `(` `[` `=` `@`, plus the multi-boundary torture statement).
        /// This net closes that gap. It asserts BOTH facets the bug violated — `≈` (comment
        /// source-ORDER) AND idempotence (`format∘format == format`) — over
        /// that family at widths [1, 8, 100]. A failure here is a residual transposition or
        /// idempotence bug the explicit cases + corpus missed: minimize and fix the library,
        /// never weaken this net.
        #[test]
        fn intra_statement_comments_preserve_order_and_idempotent(
            prog in crate::test_support::intra_comment_program(),
        ) {
            prop_assume!(!crate::cst::has_error(&prog));
            for width in [1usize, 8, 100] {
                match roundtrip_outcome(&prog, width) {
                    Outcome::Equivalent => {}
                    Outcome::InputHasError => prop_assume!(false),
                    Outcome::NotEquivalent { formatted } => prop_assert!(
                        false,
                        "≈ TRANSPOSITION VIOLATION at width {width}:\n--- input ---\n{prog}--- output ---\n{formatted}"
                    ),
                }
                let once = crate::test_support::format_at_width(&prog, width);
                let twice = crate::test_support::format_at_width(&once, width);
                prop_assert!(
                    once == twice,
                    "NON-IDEMPOTENT at width {width}:\n--- input ---\n{prog}--- once ---\n{once}--- twice ---\n{twice}"
                );
            }
        }
    }

    /// Coverage + validity guard for [`crate::test_support::intra_comment_program`]: it must
    /// (a) emit error-free programs — a net that `prop_assume`s its inputs away tests nothing
    /// — and (b) actually exercise EVERY token class (a generator that compiles but never
    /// fires a class closes no gap). Each class tags its trailing comment `%a<code>`, so a
    /// marker's presence proves that arm fired. Deterministic sampling, so this is stable.
    #[test]
    fn intra_comment_generator_covers_every_token_class_error_free() {
        use proptest::strategy::{Strategy, ValueTree};
        use proptest::test_runner::TestRunner;

        // (class label, the unique trailing-comment marker the generator stamps for it).
        const MARKERS: &[(&str, &str)] = &[
            (",", "%ac"),
            (";", "%as"),
            ("|", "%ad"),
            (":-", "%an"),
            (":~", "%aw"),
            (":", "%aj"),
            ("{", "%ao"),
            ("(", "%ap"),
            ("[", "%ak"),
            ("@", "%at"),
            ("=", "%ae"),
            ("torture", "%ax"),
        ];
        const SAMPLES: usize = 4000;

        let strat = crate::test_support::intra_comment_program();
        let mut runner = TestRunner::deterministic();
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut errors = 0usize;
        for _ in 0..SAMPLES {
            let prog = strat
                .new_tree(&mut runner)
                .expect("sample the intra-comment generator")
                .current();
            if crate::cst::has_error(&prog) {
                errors += 1;
            }
            for (label, marker) in MARKERS {
                if prog.contains(marker) {
                    seen.insert(label);
                }
            }
        }
        assert_eq!(
            errors, 0,
            "{errors}/{SAMPLES} generated intra-comment programs failed to parse (assume rate must be ~0)"
        );
        let missing: Vec<&str> = MARKERS
            .iter()
            .map(|(label, _)| *label)
            .filter(|label| !seen.contains(label))
            .collect();
        assert!(
            missing.is_empty(),
            "intra-comment generator never exercised these token classes: {missing:?}"
        );
    }

    /// Coverage + validity guard for [`crate::test_support::commented_program`] (the net behind
    /// [`formatting_preserves_comments`]), analogous to the intra-comment guard above: it must
    /// (a) emit error-free programs — that net `prop_assume`s its inputs away on a parse error,
    /// so a generator silently regressed to mostly-ERROR would make it test nothing — and (b)
    /// actually exercise EVERY comment-decoration class (leading / trailing / standalone /
    /// blank-detached). Each class carries a unique `% <c>` prefix the generator stamps (`% l` /
    /// `% t` / `% s` / `% d`); the comment-text charset excludes `%`, so the prefixes never
    /// collide and a marker's presence proves that arm fired. Deterministic sampling, so stable.
    #[test]
    fn commented_generator_covers_every_class_error_free() {
        use proptest::strategy::{Strategy, ValueTree};
        use proptest::test_runner::TestRunner;

        // (class label, the unique comment prefix the generator stamps for it).
        const CLASSES: &[(&str, &str)] = &[
            ("leading", "% l"),
            ("trailing", "% t"),
            ("standalone", "% s"),
            ("blank-detached", "% d"),
        ];
        const SAMPLES: usize = 4000;

        let strat = crate::test_support::commented_program();
        let mut runner = TestRunner::deterministic();
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut errors = 0usize;
        for _ in 0..SAMPLES {
            let prog = strat
                .new_tree(&mut runner)
                .expect("sample the commented-program generator")
                .current();
            if crate::cst::has_error(&prog) {
                errors += 1;
            }
            for (label, marker) in CLASSES {
                if prog.contains(marker) {
                    seen.insert(label);
                }
            }
        }
        assert_eq!(
            errors, 0,
            "{errors}/{SAMPLES} generated commented programs failed to parse (the prop_assume reject rate must be ~0)"
        );
        let missing: Vec<&str> = CLASSES
            .iter()
            .map(|(label, _)| *label)
            .filter(|label| !seen.contains(label))
            .collect();
        assert!(
            missing.is_empty(),
            "commented generator never exercised these comment classes: {missing:?}"
        );
    }

    /// Coverage + validity guard for the four families [`crate::test_support::valid_program`]
    /// was widened to fuzz: body conditional literals (incl. the empty-condition edge),
    /// argument AND tuple pools, disjunctive heads (BOTH `;` and `|` separators), and the
    /// VARYING-length weak-constraint tail. It must (a) emit error-free programs — a net that
    /// `prop_assume`s its inputs away tests nothing — and (b) actually FIRE every family AND
    /// each load-bearing sub-variant (a family the generator never emits closes no gap).
    /// Detection is on the PARSE TREE — named node kinds plus the author-significant
    /// `;`/`|` anonymous tokens — not source text, since a body `:` is shared by aggregate
    /// elements and the `#external`/`#show` colon-bodies. Deterministic sampling, so stable.
    #[test]
    fn valid_program_covers_every_widened_family_error_free() {
        use proptest::strategy::{Strategy, ValueTree};
        use proptest::test_runner::TestRunner;
        use std::collections::BTreeSet;

        // The widened families / load-bearing sub-variants, each detected on the PARSE TREE
        // by [`accumulate`]. A `;` argument pool and a `|` head share neither the comma's nor
        // the abs `|`'s context, so the detection keys on the exact (parent-kind, token) pair.
        const FAMILIES: &[&str] = &[
            "conditional literal",
            "empty-condition conditional",
            "argument pool",
            "tuple pool",
            "disjunction `;`",
            "disjunction `|`",
            "varying weak-constraint tail",
            // The three base construct families the generator also emits: guarding these too
            // means a regression that STOPS firing them (e.g. a dropped `statement()` arm) is
            // caught here, not silently passed over.
            "aggregate",
            "theory atom",
            "directive",
        ];

        // Accumulate the family labels present in `src` into `seen` via an iterative explicit-
        // stack tree walk (no recursion — matches the totality discipline, and cannot
        // overflow on a pathological sample). The `pool` rule is hidden in the grammar, so an
        // argument pool is detected as a node carrying ≥2 `arguments`-field children (a comma
        // arg-list is a single `arguments` field); a tuple pool as a `tuple` node with a `;`
        // child (a `,`-tuple has a comma); the varying weak tail as a `weak_constraint` with a
        // `terms`-field child (the short `[w@p]` form has none).
        fn accumulate(src: &str, seen: &mut BTreeSet<&'static str>) {
            let tree = parse(src);
            let mut stack = vec![tree.root_node()];
            while let Some(node) = stack.pop() {
                let kind = node.kind();
                let mut arg_fields = 0usize;
                let mut has_condition = false;
                crate::cst::walk_children(node, |field, child| {
                    stack.push(child);
                    match field {
                        Some("arguments") => arg_fields += 1,
                        Some("condition") => has_condition = true,
                        Some("terms") if kind == "weak_constraint" => {
                            seen.insert("varying weak-constraint tail");
                        }
                        _ => {}
                    }
                    match (kind, child.kind()) {
                        ("disjunction", ";") => {
                            seen.insert("disjunction `;`");
                        }
                        ("disjunction", "|") => {
                            seen.insert("disjunction `|`");
                        }
                        ("tuple", ";") => {
                            seen.insert("tuple pool");
                        }
                        _ => {}
                    }
                });
                if arg_fields >= 2 {
                    seen.insert("argument pool");
                }
                if kind == "conditional_literal" {
                    seen.insert("conditional literal");
                    if !has_condition {
                        seen.insert("empty-condition conditional");
                    }
                }
                // The three base construct families, detected on the node's OWN kind.
                // `valid_program` emits body aggregates, theory atoms, and the
                // directives; the match keys the whole aggregate / directive kind-set so the
                // family still registers if the generated mix later shifts.
                match kind {
                    "body_aggregate" | "head_aggregate" | "set_aggregate" | "minimize"
                    | "maximize" => {
                        seen.insert("aggregate");
                    }
                    "theory_atom" => {
                        seen.insert("theory atom");
                    }
                    "show_term" | "show_signature" | "defined" | "edge" | "const" | "program"
                    | "external" | "script" | "heuristic" | "project_signature"
                    | "project_atom" => {
                        seen.insert("directive");
                    }
                    _ => {}
                }
            }
        }

        const SAMPLES: usize = 4000;
        let strat = crate::test_support::valid_program();
        let mut runner = TestRunner::deterministic();
        let mut seen: BTreeSet<&'static str> = BTreeSet::new();
        let mut errors = 0usize;
        for _ in 0..SAMPLES {
            let prog = strat
                .new_tree(&mut runner)
                .expect("sample the valid-program generator")
                .current();
            if crate::cst::has_error(&prog) {
                errors += 1;
            }
            accumulate(&prog, &mut seen);
        }
        assert_eq!(
            errors, 0,
            "{errors}/{SAMPLES} generated programs failed to parse (the prop_assume reject rate must be ~0)"
        );
        let missing: Vec<&str> = FAMILIES
            .iter()
            .copied()
            .filter(|label| !seen.contains(label))
            .collect();
        assert!(
            missing.is_empty(),
            "valid_program never exercised these widened families/sub-variants: {missing:?}"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(NET_CASES))]
        /// Totality / normal termination: format must RETURN (no panic, no
        /// overflow) on ANY input — valid, truncated, or adversarial. The `(?s)` flag
        /// makes `.` match newlines too, so multi-line garbage (statement boundaries,
        /// unterminated comments/strings, CRLF) is exercised, not just single lines.
        /// The output is not asserted; only that the call completes.
        #[test]
        fn formatting_is_total_on_arbitrary_input(src in "(?s).{0,200}") {
            let _ = crate::test_support::format_at_width(&src, 100);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(NET_CASES))]
        /// Totality for the PUBLIC `verify`: it re-parses twice and runs the
        /// `first_divergence` byte-walks on top of `format`, and its doc claims totality —
        /// so guard that claim directly on the public surface. `verify` must RETURN (Ok or
        /// a Mismatch; no panic, no overflow) on ANY input. The verdict is not asserted,
        /// only that the call completes.
        #[test]
        fn verify_is_total_on_arbitrary_input(src in "(?s).{0,200}") {
            let _ = verify(&src, &Style::default());
        }
    }
}

#[cfg(test)]
/// The adversarial suite (permanent): ~60 comment-bearing inputs covering
/// dangling in every bracket / directive / theory construct, `doc_comment` adjacency,
/// nasty comment text, detached/blank placements, and multi-line blocks — asserting `≈`
/// (the safety invariant) and idempotence, with the deferred baked-separator
/// cases tracked by a tripwire.
mod comment_adversarial {
    use super::{roundtrip_outcome, Outcome};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    const WIDTHS: [usize; 6] = [1, 2, 4, 8, 40, 100];

    // Error-free, comment-bearing inputs. Each MUST parse clean (else it lands in
    // InputHasError and is reported as a coverage gap, not a pass).
    const CASES: &[&str] = &[
        // ---- dangling in function args / nesting ----
        "q :- p(a, b\n% d\n).\n",
        "q :- p(\n% only\n).\n",
        "q :- p(a\n% d1\n% d2\n).\n",
        "q :- p(a\n% d1\n\n% d2\n).\n",
        "q :- p(f(a\n% d\n)).\n",
        "q :- p(\n% lead\na, b).\n",
        // ---- argument pool ----
        "q :- p(a\n% d\n; b).\n",
        "q :- p(a; b\n% d\n).\n",
        // ---- tuple ----
        "p(X) :- X = (a, b\n% d\n).\n",
        "p(X) :- X = (a\n% d\n,).\n",
        "p(X) :- X = (a; b\n% d\n).\n",
        // ---- abs ----
        "p(X) :- X = |a\n% d\n|.\n",
        "p(X) :- X = |a\n% d\n; b|.\n",
        // ---- aggregates (#count/#sum/#min/#max) ----
        ":- #count{ X : p(X)\n% d\n} >= 1.\n",
        ":- #sum{ X : p(X)\n% d\n} >= 1.\n",
        ":- #min{ X : p(X)\n% d\n} >= 1.\n",
        ":- #max{ X : p(X)\n% d\n} >= 1.\n",
        // ---- choice / set aggregate ----
        "{ a; b\n% d\n} :- c.\n",
        "{ a; b\n% d\n}.\n",
        "{\n% only\n} :- c.\n",
        // ---- head aggregate ----
        "#count{ X : p(X)\n% d\n} >= 1 :- q.\n",
        // ---- minimize / maximize ----
        "#minimize{ C@1 : c(C)\n% d\n}.\n",
        "#maximize{ C@1 : c(C)\n% d\n}.\n",
        // ---- theory atom + theory brackets ----
        "&a{ x\n% d\n} :- b.\n",
        "&a{ x : y\n% d\n} :- b.\n",
        "&a{ f(x\n% d\n) } :- b.\n",
        "&a{ [x\n% d\n] } :- b.\n",
        "&a{ {x\n% d\n} } :- b.\n",
        "&a{ (x\n% d\n) } :- b.\n",
        // ---- #theory definition block ----
        "#theory t { d { - : 0, unary }\n% d\n }.\n",
        // ---- #edge ----
        "#edge (a, b\n% d\n).\n",
        "#edge (a, b; c, d\n% d\n).\n",
        "#edge (a, b\n% d\n; c, d).\n",
        // ---- #program ----
        "#program p(k, t\n% d\n).\n",
        // ---- bracket [..] tails: comments inside / before ----
        "#heuristic a. [3\n% d\n, true]\n",
        "#const n = 1. [default]\n% trail-after-stmt\n",
        // ---- before glued tails ----
        "a :- b\n% c\n.\n",
        ":~ p(X)\n% c\n. [1@2]\n",
        ":~ p(X). [1@2]\n% after\n",
        // ---- colon-body directive seams ----
        "#external p(X)\n% d\n : q(X).\n",
        "#show f(X)\n% d\n : p(X).\n",
        // ---- transposition class: own-line comment before a FOLLOWING anonymous token
        //      that carries a trailing comment (neck / opener / infix). Must preserve source
        //      order (≈) AND be idempotent at every width. The `,`/`;`
        //      separator case is covered above; these are the class extension.
        "a\n%bb\n:- %aa\nb.\n",                   // rule neck `:-`
        "p :- q\n%bb\n: %aa\nr.\n",               // conditional `:`
        ":- #count\n%bb\n{ %aa\nx : p } >= 1.\n", // aggregate `{` opener
        "p :- q(a\n%bb\n; %aa\nb).\n",            // pool `;` inside `(` (opener + sep)
        "#const c\n%bb\n= %aa\n5.\n",             // `#const` `=`
        ":~ b. [2\n%bb\n@ %aa\n1]\n",             // weight `@`
        // example.lp's nested-drift region: a neck comment above a nested
        // aggregate whose element-list comment drifted indent on reformat. Idempotent now.
        "a\n%bb\n:- %aa\n#sum { x : p\n%dd\n; y : q } >= 0.\n",
        // ---- trailing comment on inner tokens ----
        "a :- b, % t\nc.\n",
        "a :- b ; % t\nc.\n",
        "p(a % t\n, b).\n",
        "p(a, % t\nb).\n",
        // ---- doc_comment adjacent to trivia ----
        "%*! doc *%\n% trivia\np(1).\n",
        "% trivia\n%*! doc *%\np(1).\n",
        "p(1).\n%*! doc *%\n% after\nq(2).\n",
        // ---- special comment text ----
        "a :- b. % we use 100% of it.\n",
        "a :- b. %* note: has *% nested-ish\n",
        "a :- b. % \"quoted\" and : colon ; semi , comma .\n",
        "% ends-with-dot.\np.\n",
        "p. %\n",
        "p. %*x*%\n",
        // ---- leading run with blanks at top/mid/bottom ----
        "% h1\n% h2\np.\n",
        "%d1\n\np. %t\n",
        "%a\n\n%b\np. %t\n",
        "p.\n\n% tail-detached\n",
        // ---- multi-line block comment placements ----
        "%* multi\n  line *%\np.\n",
        "p(\n%* multi\n  line *%\na).\n",
        "a :- b. %* trailing\n  multi *%\n",
        // ---- comment-only / whitespace edges ----
        "% lonely\n",
        "%* block only *%\n",
        "\n\n% spaced\n\n\np.\n",
        // ---- blank lines × comments (≈ + idempotence stress) ----
        "a.\n\nb.\n",
        "a. % t\n\nb.\n",
        "a.\n\n% c\nb.\n",
        "a.\n\n\n#program p.\nb.\n",
        "a :- b,\n\n c,\n\n d.\n",
        ":- #count{ a;\n\n b } >= 0.\n",
        ":- #count{ a : p;\n\n% c\n b : q } >= 0.\n",
        "p(a;\n\n b % t\n).\n",
        // Blank fidelity around a separator + comment
        // (no-blank must NOT phantom; blank-before-separator must survive). All ≈ + idem.
        ":- a,\n% c\n b.\n",
        ":- a,\n\n% c\n b.\n",
        ":- #count{ a;\n% c\n b } >= 0.\n",
        "a;\n% c\n b :- d.\n",
        ":- aaaa\n\n, bbbb.\n",
    ];

    #[test]
    fn reinjection_is_equivalent_and_idempotent() {
        let mut viol: Vec<String> = Vec::new();
        let mut skipped: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for &src in CASES {
            // The load-bearing property: `≈` at EVERY width, owed for every case. (An
            // error-bearing input gets totality + verbatim preservation, not full `≈`.)
            for w in WIDTHS {
                match roundtrip_outcome(src, w) {
                    Outcome::NotEquivalent { formatted } => viol.push(format!(
                        "NOT-≈ @w={w}: {src:?}\n      ---out---\n{formatted}\n      ---end---"
                    )),
                    Outcome::InputHasError => {
                        skipped.insert(src);
                    }
                    Outcome::Equivalent => {}
                }
            }
            // Idempotence at 40/80, owed for every ERROR-FREE case (not owed in the
            // error/totality domain). The structural lift closed the former baked-separator deferral, so
            // the net now covers every case unconditionally.
            if crate::cst::has_error(src) {
                continue;
            }
            for w in [40usize, 80] {
                let once = crate::test_support::format_at_width(src, w);
                let twice = crate::test_support::format_at_width(&once, w);
                if once != twice {
                    viol.push(format!(
                        "NON-IDEMPOTENT @w={w}: {src:?}\n  once:\n{once}\n  twice:\n{twice}"
                    ));
                }
            }
        }
        if !skipped.is_empty() {
            eprintln!("COVERAGE-NOTE (error-bearing input — totality only, `≈` not asserted):");
            for s in &skipped {
                eprintln!("    {s:?}");
            }
        }
        assert!(
            viol.is_empty(),
            "\n==== {} adversarial violation(s) ====\n{}\n",
            viol.len(),
            viol.join("\n")
        );
    }

    // Structural lift: a leading comment always owns its line, so the two cases that
    // were `≈`-but-not-idempotent (a comment LEADING an element after a BAKED separator —
    // `#edge`'s `; `, a `[…]` weight tail's `, `) are now idempotent fixed points.
    #[test]
    fn baked_separator_leading_comment_is_idempotent() {
        for src in [
            "#edge (a, b\n% d\n; c, d).\n",
            "#heuristic a. [3\n% d\n, true]\n",
        ] {
            for w in [40usize, 80] {
                let once = crate::test_support::format_at_width(src, w);
                let twice = crate::test_support::format_at_width(&once, w);
                assert_eq!(once, twice, "now idempotent @w={w}: {src:?}\n{once}");
            }
        }
    }

    // Error-bearing inputs: totality ONLY (no panic / no overflow). ≈ is not owed.
    #[test]
    fn adversarial_error_input_is_total() {
        let errs: &[&str] = &[
            "a :- . % c\n",
            "p( % c\n",
            "p(a;;b\n% d\n).\n",
            "a :- b % c\n",
            "%* unterminated\np.\n",
            "&a{ % c\n",
            "#count{ % c\n",
            ":- #count{\n% only\n} >= 0.\n",
            "{\n% only\n}.\n",
            "}\n% c\n",
        ];
        let mut panics: Vec<String> = Vec::new();
        for &src in errs {
            for w in WIDTHS {
                if catch_unwind(AssertUnwindSafe(|| {
                    crate::test_support::format_at_width(src, w)
                }))
                .is_err()
                {
                    panics.push(format!("PANIC @w={w}: {src:?}"));
                }
            }
        }
        assert!(panics.is_empty(), "\n{}\n", panics.join("\n"));
    }

    // CRLF line endings: the parser, the attach pass, and ≈ must agree.
    #[test]
    fn adversarial_crlf() {
        let cases: &[&str] = &[
            "a :- b.\r\n% c\r\nc :- d.\r\n",
            "% lead\r\np.\r\n",
            "a :- b. % trail\r\n",
            "p(a, b\r\n% d\r\n).\r\n",
        ];
        let mut viol: Vec<String> = Vec::new();
        for &src in cases {
            for w in WIDTHS {
                match catch_unwind(AssertUnwindSafe(|| roundtrip_outcome(src, w))) {
                    Err(_) => viol.push(format!("PANIC @w={w}: {src:?}")),
                    Ok(Outcome::NotEquivalent { formatted }) => {
                        viol.push(format!("NOT-≈ @w={w}: {src:?}\n---\n{formatted}\n---"));
                    }
                    Ok(_) => {}
                }
            }
        }
        assert!(viol.is_empty(), "\n{}\n", viol.join("\n"));
    }
}
