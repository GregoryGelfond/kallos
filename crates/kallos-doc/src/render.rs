//! The strict (non-lazy) Lindig renderer: an iterative work-list (no recursion,
//! so render carries NO stack-depth risk) with an early-exit `fits`.
//!
//! Correctness is Lindig's *Strictly Pretty* argument. The work-list holds
//! `(indent, mode, node)` triples; the root starts in break mode. A `Group` is
//! laid flat iff it carries no forced break and `fits` reports that its flat
//! content plus the continuation up to the next line break stays within the
//! width. The linearity invariant (`fits` is invoked at most once per group; it
//! scans forward only, never rescanning a resolved group) is enforced as a
//! checkable post-condition by [`tests::fits_invoked_at_most_once_per_group`].

use crate::arena::{DocNode, NodeId};
use crate::builder::Doc;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// A DEFERRED line break: the `indent` it resumes at, plus whether it is a **blank**
/// break (an author blank line). Consecutive deferred breaks coalesce — the
/// last `indent` wins and the `blank` bit OR-s in — so a `blank_line` adjoining a base
/// break yields exactly one blank line (and never doubles a structural break into one).
#[derive(Clone, Copy)]
struct Pending {
    indent: i32,
    blank: bool,
}

#[cfg(test)]
thread_local! {
    /// Per-thread count of `fits` invocations. Thread-local rather
    /// than a shared atomic so the linearity assertion is immune to other tests
    /// running concurrently on sibling worker threads.
    static FITS_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A render-operation counter. Two impls: `()` is the no-op production
/// strategy — empty `#[inline(always)]` methods, the idiomatic zero-cost-abstraction
/// shape (the same generic-strategy pattern as `std::hash::Hasher`, a type parameter, not
/// a runtime branch) that the compiler is expected to compile away; the test-only
/// `RenderStats` accumulates the two quantities. What the render tests actually VERIFY
/// is output-identity — the production output is byte-identical with the counter threaded
/// (it is a side-channel, never touching the emitted string). The no-overhead property is
/// the idiom's intent, not separately benchmarked here.
///
/// The indirection earns its place (Clarity > Cleverness): it turns the claim —
/// render work is linear in document size and *width-invariant* — into a deterministic,
/// testable property, replacing a flaky wall-clock timer with exact op-counts.
trait Counter {
    /// One work-list pop (one node visited).
    fn step(&mut self);
    /// `n` columns examined by `fits` (the only width-sensitive quantity).
    fn scan(&mut self, n: usize);
}

impl Counter for () {
    // Empty + `#[inline(always)]`: the idiomatic zero-cost no-op strategy, which the
    // compiler is expected to inline away. What the render tests verify is output-identity —
    // the production output is byte-identical with this counter threaded.
    #[inline(always)]
    fn step(&mut self) {}
    #[inline(always)]
    fn scan(&mut self, _n: usize) {}
}

/// Test-only render-operation tallies for the linearity gate. `worklist_steps`
/// counts nodes visited (work-list pops) — exactly width-invariant; `fits_chars`
/// counts the columns `fits` measures against its budget — the width-sensitive
/// quantity the early-exit keeps at `O(n)` (never `O(n·w)`).
#[cfg(test)]
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RenderStats {
    pub worklist_steps: usize,
    pub fits_chars: usize,
}

#[cfg(test)]
impl Counter for RenderStats {
    fn step(&mut self) {
        self.worklist_steps += 1;
    }
    fn scan(&mut self, n: usize) {
        self.fits_chars += n;
    }
}

/// Render `doc` against `src` (the original source its leaf offsets index into)
/// at the target `width`. Total and linear in the document size.
///
/// # Panics
///
/// Panics if a leaf/verbatim span does not name a valid `char`-boundary slice of
/// `src`. By construction the builder records spans cut from this same `src`, so
/// the contract holds for any `Doc` built against the string passed here.
#[must_use]
pub fn render(doc: &Doc, src: &str, width: usize) -> String {
    // Production threads the no-op `()` counter: the idiomatic zero-cost strategy the
    // compiler is expected to inline away. The render tests verify the output is
    // byte-identical with it threaded (the counter is a side-channel that never touches the
    // emitted string). See the `Counter` trait.
    render_core(doc, src, width, &mut ())
}

/// Test-only twin of [`render`] returning the op-count tallies alongside the
/// output, so the deterministic linearity gate reads exact counts rather than a
/// flaky wall-clock timer.
#[cfg(test)]
#[must_use]
pub(crate) fn render_with_stats(doc: &Doc, src: &str, width: usize) -> (String, RenderStats) {
    let mut stats = RenderStats::default();
    let out = render_core(doc, src, width, &mut stats);
    (out, stats)
}

/// The renderer proper, generic over a [`Counter`] so the op-counts can be
/// tallied in tests at zero cost in production (`C = ()`). `counter.step()` fires
/// once per work-list pop (nodes visited); `counter.scan(n)` records the `n` columns
/// `fits` measures against its width budget.
fn render_core<C: Counter>(doc: &Doc, src: &str, width: usize, counter: &mut C) -> String {
    let mut out = String::with_capacity(src.len() + src.len() / 8);
    let mut col: usize = 0;
    // A DEFERRED line break (a "lazy newline"): a break decided but not yet written (its
    // `indent` + `blank` bit, [`Pending`]). The newline materializes only when real
    // content follows ([`flush`]); so consecutive breaks coalesce (last indent wins,
    // `blank` OR-s in), a break with NO following content is dropped (no trailing blank),
    // and a break with NO PRECEDING content is dropped (no leading blank). This
    // keeps a re-injected comment's forced `Hardline` from doubling with a following
    // structural break into an unwanted blank line, strips trailing whitespace off
    // broken lines for free, and realizes the `BlankLine` author blank.
    let mut pending: Option<Pending> = None;
    // Work-list of (indent, mode, node); the root starts in break mode.
    let mut stack: Vec<(i32, Mode, NodeId)> = vec![(0, Mode::Break, doc.root)];

    while let Some((indent, mode, id)) = stack.pop() {
        counter.step(); // one node visited (width-invariant: every node pops once)
        match doc.node(id) {
            DocNode::Nil => {}
            DocNode::Leaf(sp) => {
                flush(&mut out, &mut pending, &mut col);
                out.push_str(&src[sp.start as usize..(sp.start + sp.len) as usize]);
                col += sp.width as usize;
            }
            DocNode::Verbatim(sp) => {
                flush(&mut out, &mut pending, &mut col);
                let s = &src[sp.start as usize..(sp.start + sp.len) as usize];
                out.push_str(s);
                // The column advances to the display width of the span's LAST
                // line; internal newlines are emitted literally (no re-indent).
                col = match s.rsplit_once('\n') {
                    Some((_, last)) => last.width(),
                    None => col + s.width(),
                };
            }
            DocNode::Line => match mode {
                Mode::Flat => {
                    flush(&mut out, &mut pending, &mut col);
                    out.push(' ');
                    col += 1;
                }
                Mode::Break => col = defer_break(&mut pending, indent, false),
            },
            DocNode::Space => {
                // A space stranded at the START of a fresh line (a break is pending, so it
                // would be leading whitespace) is dropped — the break stays pending for the
                // next real content. Elsewhere it is an ordinary inter-token blank.
                if pending.is_none() {
                    out.push(' ');
                    col += 1;
                }
            }
            DocNode::SoftLine => match mode {
                Mode::Flat => {}
                Mode::Break => col = defer_break(&mut pending, indent, false),
            },
            DocNode::Hardline => col = defer_break(&mut pending, indent, false),
            // A non-forcing blank marker: when broken it promotes the adjacent
            // deferred break to a blank line (the `blank` bit); when flat it is nothing.
            DocNode::BlankLine => match mode {
                Mode::Flat => {}
                Mode::Break => col = defer_break(&mut pending, indent, true),
            },
            DocNode::Nest { delta, child } => {
                for &c in doc.children(child).iter().rev() {
                    stack.push((indent + delta, mode, c));
                }
            }
            DocNode::Seq(range) => {
                for &c in doc.children(range).iter().rev() {
                    stack.push((indent, mode, c));
                }
            }
            DocNode::Group {
                child,
                forced_break,
            } => {
                let kids = doc.children(child);
                let flat = !forced_break
                    && fits(
                        doc,
                        width.saturating_sub(col),
                        indent,
                        kids,
                        &stack,
                        counter,
                    );
                let m = if flat { Mode::Flat } else { Mode::Break };
                for &c in kids.iter().rev() {
                    stack.push((indent, m, c));
                }
            }
        }
    }
    // A break still pending at the end is intentionally dropped: a document never ends
    // with a forced trailing newline (the file's single terminating newline is a
    // separate concern).
    out
}

/// Emit a newline followed by `indent` (clamped at zero) spaces; return the new
/// column. Shared by `Line`/`SoftLine` in break mode and by `Hardline`.
fn break_line(out: &mut String, indent: i32) -> usize {
    out.push('\n');
    let pad = usize::try_from(indent.max(0)).unwrap_or(0);
    for _ in 0..pad {
        out.push(' ');
    }
    pad
}

/// Record a deferred break at `indent`, coalescing with any already-pending break: the
/// new `indent` wins (so a dedented closer lands correctly) while the `blank` bit OR-s in
/// (so a `blank_line` adjoining a structural break still yields one blank line). Returns
/// the column content will resume at — the indent — so a `Group`'s `fits` measures from
/// the right place even though the newline is not written until [`flush`].
fn defer_break(pending: &mut Option<Pending>, indent: i32, blank: bool) -> usize {
    let blank = blank || pending.is_some_and(|p| p.blank);
    *pending = Some(Pending { indent, blank });
    usize::try_from(indent.max(0)).unwrap_or(0)
}

/// Materialize a deferred break immediately before real content. A break with NO
/// preceding content is dropped (leading blank → 0). Otherwise the current line's
/// trailing horizontal whitespace is stripped (verbatim spans are exempt: their
/// internal newlines are pushed atomically and never reach here), an extra newline is
/// emitted for a blank break, then the newline + indent for the resuming line.
fn flush(out: &mut String, pending: &mut Option<Pending>, col: &mut usize) {
    let Some(p) = pending.take() else { return };
    if out.is_empty() {
        return; // a break before any content: no leading blank line
    }
    let kept = out.trim_end_matches([' ', '\t']).len();
    out.truncate(kept);
    if p.blank {
        out.push('\n'); // the (empty) blank line
    }
    *col = break_line(out, p.indent);
}

/// Early-exit `fits` (Lindig *Strictly Pretty*, option (i)): does the group's
/// flat content, followed by the continuation up to the next line break, fit in
/// `remaining` columns?
///
/// It scans forward only, stopping at the first of: an overflow (→ `false`), a
/// forced break (→ `false`, since a forced break can never be flat), or a
/// break-mode line in the continuation (→ `true`, since that line ends here).
/// The continuation is consulted in place, one item at a time, and never copied,
/// so a single call costs only the `O(w)` prefix it reads before the first overflow
/// or break — avoiding the `O(n)`-per-call rescan that would make rendering `O(n²)`.
/// Total work is `O(n·w)`: linear in the document at a fixed width. (The
/// once-per-group bound assumes a tree-shaped Doc — the builder never aliases a
/// node across parents.)
fn fits<C: Counter>(
    doc: &Doc,
    mut remaining: usize,
    group_indent: i32,
    group_children: &[NodeId],
    continuation: &[(i32, Mode, NodeId)],
    counter: &mut C,
) -> bool {
    #[cfg(test)]
    FITS_CALLS.with(|cell| cell.set(cell.get() + 1));

    // A local work-list for expanding subtrees. Seed it with the group's
    // children in flat mode, reversed so they pop in source order. When it
    // drains, the continuation is pulled one item at a time, also in source
    // order: the render stack stores items top-last, so `.rev()` walks forward.
    let mut work: Vec<(i32, Mode, NodeId)> = Vec::new();
    for &c in group_children.iter().rev() {
        work.push((group_indent, Mode::Flat, c));
    }
    let mut continuation = continuation.iter().rev();

    loop {
        while let Some((indent, mode, id)) = work.pop() {
            match doc.node(id) {
                DocNode::Nil => {}
                DocNode::Leaf(sp) => {
                    let w = sp.width as usize;
                    if w > remaining {
                        return false;
                    }
                    remaining -= w;
                    counter.scan(w); // w columns measured as fitting
                }
                // A hardline ends the current line: everything measured so far fit,
                // so the group lays flat. (A hardline in the group's OWN content
                // can't reach here — such a group carries `forced_break` and skips
                // `fits` entirely; so any hardline seen is in the continuation, where
                // it terminates the line exactly like a break-mode `Line`.)
                DocNode::Hardline => return true,
                // A verbatim span carries internal newlines and can never lay flat.
                DocNode::Verbatim(_) => return false,
                DocNode::Line => match mode {
                    Mode::Flat => {
                        if remaining == 0 {
                            return false;
                        }
                        remaining -= 1;
                        counter.scan(1); // the flat line's single space
                    }
                    // A break here ends the current line: everything so far fit.
                    Mode::Break => return true,
                },
                DocNode::Space => {
                    if remaining == 0 {
                        return false;
                    }
                    remaining -= 1;
                    counter.scan(1); // the mode-invariant blank
                }
                // `BlankLine` fits exactly like `SoftLine` — nothing when flat, ends the
                // line when broken; its blank-promotion is a render-time concern only.
                DocNode::SoftLine | DocNode::BlankLine => match mode {
                    Mode::Flat => {}
                    Mode::Break => return true,
                },
                DocNode::Nest { delta, child } => {
                    for &c in doc.children(child).iter().rev() {
                        work.push((indent + delta, mode, c));
                    }
                }
                DocNode::Seq(range) => {
                    for &c in doc.children(range).iter().rev() {
                        work.push((indent, mode, c));
                    }
                }
                // Groups inside a fits-test are tried flat.
                DocNode::Group { child, .. } => {
                    for &c in doc.children(child).iter().rev() {
                        work.push((indent, Mode::Flat, c));
                    }
                }
            }
        }
        match continuation.next() {
            Some(&item) => work.push(item),
            None => return true,
        }
    }
}

/// Shared linearity-gate fixture. A deliberate twin of the `wide_doc` in
/// `benches/render_linearity.rs`: a `#[cfg(test)]` item is invisible across the
/// bench/lib compile boundary (a bench links the library built *without*
/// `cfg(test)`), so the deterministic gate and the wall-clock bench cannot
/// literally share one definition — the same constraint that forces `builder.rs`'s
/// local `run_on_tiny_stack` twin. Keep the two `wide_doc`s in lockstep.
#[cfg(test)]
mod tests_fixtures {
    use crate::builder::{Doc, DocBuilder};

    /// A flat wide `group(seq(leaf, line, leaf, line, …))` of `n` leaves over one
    /// backing source — the size-sweep fixture. One group ⇒ exactly one `fits`
    /// call; `2n + 2` nodes ⇒ `2n + 2` work-list pops, identical at every width.
    pub(super) fn wide_doc(n: usize) -> (Doc, String) {
        let mut b = DocBuilder::new();
        let src = "a ".repeat(n);
        let mut items = Vec::with_capacity(n * 2);
        for i in 0..n {
            let leaf = b.leaf(u32::try_from(i * 2).expect("offset fits u32"), "a");
            items.push(leaf);
            let line = b.line();
            items.push(line);
        }
        let seq = b.seq(&items);
        let group = b.group(seq);
        (b.finish(group), src)
    }

    /// `m` independent small groups (`group(leaf line leaf)` ≈ "a a") joined by
    /// hardlines — the MANY-groups fixture. Each group calls `fits` once and, at any
    /// width, that call early-exits on its own content then the following hardline, so
    /// total `fits_chars` is `O(m)` even at unbounded width. A `fits` that failed to
    /// stop at a continuation break would, at large width, rescan the whole tail per
    /// group → `O(m²)` (the forbidden O(n·w) trap). The single-group `wide_doc`
    /// cannot express this — it has only one `fits` call — so this is the fixture that
    /// makes the multi-group linearity claim deterministically testable.
    pub(super) fn many_groups(m: usize) -> (Doc, String) {
        let mut b = DocBuilder::new();
        let src = "a a".to_string(); // every leaf slices this shared, real source
        let mut items = Vec::with_capacity(m * 2);
        for i in 0..m {
            let head = b.leaf(0, "a");
            let gap = b.line();
            let tail = b.leaf(2, "a");
            let inner = b.seq(&[head, gap, tail]);
            items.push(b.group(inner));
            if i + 1 < m {
                items.push(b.hardline());
            }
        }
        let seq = b.seq(&items);
        (b.finish(seq), src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::NodeId;
    use crate::builder::DocBuilder;

    fn one_line(src: &str, build: impl FnOnce(&mut DocBuilder) -> NodeId, w: usize) -> String {
        let mut builder = DocBuilder::new();
        let root = build(&mut builder);
        render(&builder.finish(root), src, w)
    }

    #[test]
    fn group_breaks_when_it_overflows() {
        // "aaaa" + Line + "BBBB" at width 5 → must break: Line becomes newline.
        let src = "aaaaBBBB";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "aaaa");
                let l = builder.line();
                let bb = builder.leaf(4, "BBBB");
                let inner = builder.seq(&[a, l, bb]);
                builder.group(inner)
            },
            5,
        );
        assert_eq!(out, "aaaa\nBBBB");
    }

    #[test]
    fn nest_indents_broken_lines_relatively() {
        let src = "aaaaBBBB";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "aaaa");
                let l = builder.line();
                let bb = builder.leaf(4, "BBBB");
                let inner = builder.seq(&[a, l, bb]);
                let nested = builder.nest(2, inner);
                builder.group(nested)
            },
            5,
        );
        assert_eq!(out, "aaaa\n  BBBB"); // continuation indented by the relative delta 2
    }

    #[test]
    fn softline_is_nothing_when_flat_newline_when_broken() {
        let src = "ab";
        let flat = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let s = builder.softline();
                let c = builder.leaf(1, "b");
                let inner = builder.seq(&[a, s, c]);
                builder.group(inner)
            },
            80,
        );
        assert_eq!(flat, "ab"); // softline flat = nothing

        let broken = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let s = builder.softline();
                let c = builder.leaf(1, "b");
                let inner = builder.seq(&[a, s, c]);
                builder.group(inner)
            },
            1,
        );
        assert_eq!(broken, "a\nb"); // softline broken = newline (no space)
    }

    #[test]
    fn hardline_forces_the_enclosing_group_broken() {
        let src = "ab";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let h = builder.hardline();
                let c = builder.leaf(1, "b");
                let inner = builder.seq(&[a, h, c]);
                builder.group(inner) // fits trivially, but the hardline forces broken
            },
            80,
        );
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn group_stays_flat_when_its_continuation_begins_with_a_hardline() {
        // A hardline AFTER a group (its continuation, e.g. between two statements)
        // ends the line — it must NOT force the group itself to break. The group
        // lays flat iff its own content fits; the hardline then breaks after it.
        // Regression: `fits` treated a continuation hardline as "cannot fit".
        let src = "abc";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let gap = builder.line();
                let b = builder.leaf(1, "b");
                let inner = builder.seq(&[a, gap, b]);
                let grp = builder.group(inner); // "a b" — fits at width 80
                let brk = builder.hardline();
                let c = builder.leaf(2, "c");
                builder.seq(&[grp, brk, c]) // group, then a sibling hardline, then "c"
            },
            80,
        );
        assert_eq!(out, "a b\nc"); // group flat; the hardline breaks AFTER it
    }

    #[test]
    fn verbatim_keeps_internal_newlines_literal_no_reindent() {
        // A verbatim span at indent 4 must NOT have its 2nd line re-indented.
        let src = "X\nY";
        let out = one_line(
            src,
            |builder| {
                let v = builder.verbatim(0, "X\nY");
                builder.nest(4, v)
            },
            80,
        );
        assert_eq!(out, "X\nY"); // NOT "X\n    Y"
    }

    #[test]
    fn fits_invoked_at_most_once_per_group() {
        // Build N nested groups, each of which must break at the target width,
        // and assert `fits` ran exactly once per group: no group is
        // ever re-scanned, the checkable form of the renderer's linearity.
        const GROUPS: usize = 8;
        let src = "wwwwwwwwww"; // width-1 leaves; we slice "wwwww" (width 5)
        let mut builder = DocBuilder::new();

        let mut current = {
            let a = builder.leaf(0, "wwwww");
            let l = builder.line();
            let c = builder.leaf(0, "wwwww");
            let s = builder.seq(&[a, l, c]);
            builder.group(s)
        };
        for _ in 1..GROUPS {
            let l = builder.line();
            let extra = builder.leaf(0, "wwwww");
            let s = builder.seq(&[current, l, extra]);
            current = builder.group(s);
        }
        let doc = builder.finish(current);

        FITS_CALLS.with(|c| c.set(0));
        let _ = render(&doc, src, 3); // width 3 → every group overflows → all break
        let calls = FITS_CALLS.with(std::cell::Cell::get);

        assert_eq!(
            calls, GROUPS,
            "fits must be invoked exactly once per group entered in break context"
        );
    }

    #[test]
    fn space_is_a_blank_when_flat() {
        let src = "ab";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let sp = builder.space();
                let b = builder.leaf(1, "b");
                let inner = builder.seq(&[a, sp, b]);
                builder.group(inner)
            },
            80,
        );
        assert_eq!(out, "a b");
    }

    #[test]
    fn space_stays_a_blank_when_the_group_is_forced_broken() {
        // A hardline forces the group broken; the space before it must remain a
        // literal ' ' (it is NOT a line), so the head-side join survives a break.
        let src = "abc";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let sp = builder.space();
                let b = builder.leaf(1, "b");
                let h = builder.hardline();
                let c = builder.leaf(2, "c");
                let inner = builder.seq(&[a, sp, b, h, c]);
                builder.group(inner)
            },
            80,
        );
        assert_eq!(out, "a b\nc");
    }

    #[test]
    fn space_counts_one_column_in_the_fit_decision() {
        // "a b cc" is 6 columns; at width 3 the group must break. The space's one
        // column is part of that flat measure (proving the `fits` arm decrements).
        let src = "abcc";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let sp = builder.space();
                let b = builder.leaf(1, "b");
                let l = builder.line();
                let cc = builder.leaf(2, "cc");
                let inner = builder.seq(&[a, sp, b, l, cc]);
                builder.group(inner)
            },
            3,
        );
        assert_eq!(out, "a b\ncc");
    }

    #[test]
    fn space_does_not_itself_force_a_break() {
        // A group whose only gaps are a space and a line stays flat when it fits:
        // the space is not a forced break (unlike hardline/verbatim).
        let src = "abc";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let sp = builder.space();
                let b = builder.leaf(1, "b");
                let l = builder.line();
                let c = builder.leaf(2, "c");
                let inner = builder.seq(&[a, sp, b, l, c]);
                builder.group(inner)
            },
            80,
        );
        assert_eq!(out, "a b c");
    }

    #[test]
    fn consecutive_breaks_coalesce_to_one_newline() {
        // Two adjacent breaks (the shape a re-injected comment's forced `Hardline`
        // makes when it lands just before a structural break) collapse to ONE newline,
        // never a blank line.
        let src = "ab";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let h1 = builder.hardline();
                let h2 = builder.hardline();
                let b = builder.leaf(1, "b");
                builder.seq(&[a, h1, h2, b])
            },
            80,
        );
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn a_trailing_break_emits_no_newline_or_whitespace() {
        // A break with no following content is dropped: no trailing newline, no trailing
        // indent. The file's single terminating newline is a later concern.
        let src = "a";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let h = builder.hardline();
                builder.seq(&[a, h])
            },
            80,
        );
        assert_eq!(out, "a");
    }

    #[test]
    fn coalesced_breaks_adopt_the_last_indent() {
        // When breaks coalesce, the LAST break's indent wins, so a dedented closer
        // following a comment's (deeper) forced break still lands at the closer column.
        let src = "ab";
        let out = one_line(
            src,
            |builder| {
                let a = builder.leaf(0, "a");
                let deep = builder.hardline();
                let deep_nested = builder.nest(4, deep); // a break deferred at indent 4
                let shallow = builder.hardline(); // then a break at indent 0 (wins)
                let b = builder.leaf(1, "b");
                builder.seq(&[a, deep_nested, shallow, b])
            },
            80,
        );
        assert_eq!(out, "a\nb"); // b at column 0, not 4
    }

    #[test]
    fn a_space_stranded_at_the_start_of_a_broken_line_is_dropped() {
        // A neck / operator space that would land at the START of a fresh line
        // (a break is pending before it) is leading whitespace — dropped; the break
        // materializes for the next real content instead. A space NOT after a break is
        // an ordinary inter-token blank and is kept (covered by `space_is_a_blank…`).
        let mut bd = DocBuilder::new();
        let a = bd.leaf(0, "a");
        let h = bd.hardline();
        let sp = bd.space(); // would land at line start → dropped
        let b = bd.leaf(1, "b");
        let s = bd.seq(&[a, h, sp, b]);
        assert_eq!(render(&bd.finish(s), "ab", 80), "a\nb"); // not "a\n b"
    }

    #[test]
    fn blank_line_is_a_blank_when_broken_nothing_when_flat() {
        // `a ⊕ line ⊕ blank_line ⊕ b` in ONE group: flat → "a b" — the
        // blank_line contributes nothing AND does not force the group; broken → "a\n\nb"
        // — the line becomes a newline and the blank_line promotes it to a blank line.
        let build = |width| {
            let mut bd = DocBuilder::new();
            let a = bd.leaf(0, "a");
            let gap = bd.line();
            let bl = bd.blank_line();
            let b = bd.leaf(1, "b");
            let body = bd.seq(&[a, gap, bl, b]);
            let grp = bd.group(body);
            render(&bd.finish(grp), "ab", width)
        };
        assert_eq!(build(80), "a b"); // non-forcing: fits → flat → no blank
        assert_eq!(build(1), "a\n\nb"); // broken → one blank line preserved
    }

    #[test]
    fn leading_break_is_dropped_and_trailing_ws_trimmed_before_a_break() {
        // A break BEFORE any content is dropped (no leading blank line); a literal
        // space immediately before a break is stripped (no trailing whitespace).
        let mut bd = DocBuilder::new();
        let h0 = bd.hardline(); // a LEADING break (nothing emitted yet) → dropped
        let a = bd.leaf(0, "a");
        let sp = bd.space(); // a trailing space …
        let h1 = bd.hardline(); // … immediately before a break → trimmed
        let b = bd.leaf(1, "b");
        let s = bd.seq(&[h0, a, sp, h1, b]);
        assert_eq!(render(&bd.finish(s), "ab", 80), "a\nb"); // not "\na \nb"
    }

    // ---- deterministic linearity gate (op-counts, not wall-clock) ----

    #[test]
    fn worklist_steps_are_width_invariant() {
        // worklist_steps = nodes visited; every node is popped exactly once whatever
        // the mode, so the count is IDENTICAL across widths — the strong statement
        // (flat vs broken changes the OUTPUT, never the number of nodes traversed).
        // Observed: 2n+2 = 402 at every one of the five widths (n=200).
        let (doc, src) = super::tests_fixtures::wide_doc(200);
        let counts: Vec<usize> = [1usize, 20, 100, 1000, usize::MAX]
            .iter()
            .map(|&w| super::render_with_stats(&doc, &src, w).1.worklist_steps)
            .collect();
        assert!(
            counts.windows(2).all(|w| w[0] == w[1]),
            "worklist_steps vary by width: {counts:?}"
        );
    }

    #[test]
    fn worklist_steps_are_linear_in_size() {
        // Doubling the document doubles the work-list steps (linear). A quadratic
        // renderer would ~quadruple; the [1.8, 2.2] band rules that out. Checked in
        // integer arithmetic (1.8·c1 ≤ c2 ≤ 2.2·c1) to avoid a lossy f64 cast.
        // Observed: 1002 → 2002 (ratio 1.998; the +2 is the seq+group overhead).
        let (d1, s1) = super::tests_fixtures::wide_doc(500);
        let (d2, s2) = super::tests_fixtures::wide_doc(1000);
        let c1 = super::render_with_stats(&d1, &s1, 100).1.worklist_steps;
        let c2 = super::render_with_stats(&d2, &s2, 100).1.worklist_steps;
        assert!(
            c2 * 10 >= c1 * 18 && c2 * 10 <= c1 * 22,
            "worklist_steps non-linear: {c1}→{c2} (want c2 in [1.8·c1, 2.2·c1])"
        );
    }

    #[test]
    fn fits_chars_are_bounded_linear_at_every_width() {
        // `fits`'s early-exit caps the columns it scans at the lone group's full
        // flat width (2n) — `O(n)` at EVERY width, the property an `O(n·w)` renderer
        // violates. Observed: fits_chars = min(w, 2n) = 1, 20, 100, 1000, then it
        // SATURATES at 2000 (= 2n) by width=usize::MAX; it never tracks w upward past
        // 2n. `K = 3` sits just above the structural constant 2 (1.5× headroom): tight
        // enough that an O(n·w) blow-up or a ≥1.5× rescan trips it, not brittle (the
        // count is deterministic, no wall-clock).
        const K: usize = 3;
        let n = 1000usize;
        let (doc, src) = super::tests_fixtures::wide_doc(n);
        for &w in &[1usize, 20, 100, 1000, usize::MAX] {
            let fits_chars = super::render_with_stats(&doc, &src, w).1.fits_chars;
            assert!(
                fits_chars <= K * n,
                "fits_chars={fits_chars} exceeds {K}·n at width {w} (O(n·w) regression?)"
            );
        }
    }

    #[test]
    fn fits_chars_stay_linear_across_many_groups_at_unbounded_width() {
        // The multi-group O(n·w) guard the single-group `wide_doc` cannot give (it has
        // exactly one `fits` call, so it can never exhibit the rescan trap). With m
        // independent groups `fits` runs m times; the continuation-break early-exit
        // stops each call at the next hardline, so total fits_chars is O(m) — linear —
        // and does NOT grow once the width exceeds a group: the early-exit, not the
        // width, bounds the scan. A `fits` that rescanned the tail would be O(m²).
        //
        // Two facets, both at the dangerous (unbounded) width:
        //  (1) saturation — width 100 and usize::MAX scan the SAME 3m columns (3000 at
        //      m=1000): unbounded width buys no extra work;
        //  (2) linearity — doubling m doubles fits_chars (1500 → 3000, ratio 2.0); a
        //      quadratic renderer would ~quadruple it.
        let (doc, src) = super::tests_fixtures::many_groups(1000);
        let bounded = super::render_with_stats(&doc, &src, 100).1.fits_chars;
        let unbounded = super::render_with_stats(&doc, &src, usize::MAX)
            .1
            .fits_chars;
        assert_eq!(
            bounded, unbounded,
            "fits_chars grew from width 100 to usize::MAX ({bounded}→{unbounded}): the \
             continuation-break early-exit is broken (O(n·w) at large width)"
        );

        let (d1, s1) = super::tests_fixtures::many_groups(500);
        let (d2, s2) = super::tests_fixtures::many_groups(1000);
        let c1 = super::render_with_stats(&d1, &s1, usize::MAX).1.fits_chars;
        let c2 = super::render_with_stats(&d2, &s2, usize::MAX).1.fits_chars;
        assert!(
            c2 * 10 >= c1 * 18 && c2 * 10 <= c1 * 22,
            "fits_chars quadratic in group count at unbounded width: {c1}→{c2} \
             (want c2 in [1.8·c1, 2.2·c1])"
        );
    }
}
