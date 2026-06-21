//! The opener taxonomy lifted into the emitter. Per construct, tiny code
//! classifies each child into a small [`Role`] set; then ONE generic
//! [`assemble`] realizes the two invariants — (1) a connective neck is never
//! stranded on a bare line (it trails its head, or leads with the first element
//! riding it), and (2) every hang is a *relative* `nest`, so indents compose
//! additively (`:-`-hang 3 nesting a block 4 → 7, no absolute-column arithmetic).
//! The shape logic lives here once; constructs only
//! assign roles.

use super::spacing::Sep;
use doc::{DocBuilder, NodeId};

/// Whether an opener is the light connective neck (`:-`/`:~`/conditional `:`),
/// which leads or trails, or a heavy bracket (`(`/`{`/`[`/`#agg{`/`&p{`), which
/// abuts its name/bound and opens an indented block with a dedented closer. A
/// bracket carries its interior [`Pad`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpenerKind {
    Connective,
    Bracket(Pad),
}

/// Whether a bracket pads its interior with a space when laid flat. Set/aggregate
/// braces breathe (`{ S : … }`); term parens hug (`(…)`) — spaces
/// inside set/aggregate braces; none inside term parens. When the bracket
/// explodes the pad is a newline either way; `Pad` governs only the flat form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pad {
    /// `{ … }` — a space inside each brace when flat.
    Spaced,
    /// `(…)` — content hugs the parens when flat. The sole Tight bracket; consumed
    /// by the applicative term parens (`lower_application`) and abs `|…|`.
    Tight,
}

/// The role a child plays in its construct's layout. The construct rule
/// assigns these; [`assemble`] turns the sequence into one derived shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    /// Lowered content: a head atom, a body literal, a term, an argument.
    Element,
    /// An opener — a connective neck or a bracket (see [`OpenerKind`]).
    Opener(OpenerKind),
    /// The bracket closer matching an `Opener(Bracket)` (`)`/`}`/`]`).
    Closer,
    /// A separator (`,`/`;`/`|`): trails its element; the break point follows it. The
    /// `Sep` is the flat-spacing — `Spaced → line`, `Tight → softline` — while the
    /// break-when-broken is identical either way (fit-or-explode binds nested lists
    /// too, so a Tight separator is a `SoftLine`, never a non-break-point).
    Separator(Sep),
    /// Glued terminal content (`.`, a weak-constraint `[w@p]`): abuts the
    /// preceding token, with no break before it.
    Tail,
}

/// A construct child: its assigned [`Role`] and its already-lowered Doc node.
pub(super) struct Part {
    pub(super) role: Role,
    pub(super) doc: NodeId,
}

/// The four inhabited construct→assemble shapes, as a BORROWED view over the
/// construct's `Vec<Part>`. [`classify`] recovers the shape; [`assemble`] matches it
/// exhaustively — no `debug_assert`, no silent-release "pick the first opener".
/// Borrowed (not owned make-unrepresentable) because the ≤1-opener violation is
/// provably cosmetic (Invariant W, the proptest below): a malformed spine is at worst
/// mis-laid-out, never an `≈` violation — so the lift costs the construct rules
/// nothing; they keep emitting `Vec<Part>`.
enum Spine<'a> {
    /// `:- body .` — a HEADLESS connective neck leads; the body's first element rides
    /// the neck line (Invariant 1, headless case).
    LeadingNeck {
        neck: &'a Part,
        body: &'a [Part],
        tail: &'a [Part],
    },
    /// `head :- body .` — a HEADED connective neck trails the head; the body drops to
    /// a relative `indent` block (Invariant 1, headed case).
    TrailingNeck {
        head: &'a [Part],
        neck: &'a Part,
        body: &'a [Part],
        tail: &'a [Part],
    },
    /// `name{ body } tail` — a heavy bracket opens an `indent` block with a dedented
    /// closer.
    Bracketed {
        name: &'a [Part],
        opener: &'a Part,
        body: &'a [Part],
        closer: Option<&'a Part>,
        tail: &'a [Part],
    },
    /// `e0 sep e1 … en` — a pure separated sequence wrapped in its own group.
    Separated { items: &'a [Part] },
}

/// Recover the spine shape from a construct's parts. TOTAL and order-preserving
/// over ANY `&[Part]` (Invariant W): it slices at the first structural opener, so every
/// part lands in exactly one borrowed field in source order — a malformed spine (the
/// provably cosmetic case) is at worst mis-laid-out, never dropped, never a panic.
///
/// The neck dispatch encodes Invariant 1 DIRECTLY: a connective neck *leads* iff
/// it is HEADLESS — `head.is_empty()` — else it *trails*. This is the generative
/// invariant from which the per-construct table is derived (a constraint leads because
/// it is headless; a rule trails because it has a head), lifted into the selection
/// criterion rather than re-stipulated per construct kind — the
/// structural fact is load-bearing in the representation, not operational folklore.
fn classify(parts: &[Part]) -> Spine<'_> {
    if let Some(i) = position(parts, |r| matches!(r, Role::Opener(OpenerKind::Connective))) {
        let head = &parts[..i];
        let neck = &parts[i];
        let (body, tail) = split_tail(&parts[i + 1..]);
        if head.is_empty() {
            Spine::LeadingNeck { neck, body, tail }
        } else {
            Spine::TrailingNeck {
                head,
                neck,
                body,
                tail,
            }
        }
    } else if let Some(i) = position(parts, |r| matches!(r, Role::Opener(OpenerKind::Bracket(_)))) {
        let name = &parts[..i];
        let opener = &parts[i];
        let after = &parts[i + 1..];
        let (body, closer, tail) =
            if let Some(c) = after.iter().position(|p| p.role == Role::Closer) {
                (&after[..c], Some(&after[c]), &after[c + 1..])
            } else {
                let (body, tail) = split_tail(after);
                (body, None, tail)
            };
        Spine::Bracketed {
            name,
            opener,
            body,
            closer,
            tail,
        }
    } else {
        Spine::Separated { items: parts }
    }
}

/// Realize the shape of `parts`. `indent` (the block drop) and `neck_width` (the
/// `:- ` hang) come from `Style`; both are relative `nest` deltas (Invariant 2). Returns
/// one grouped Doc node: flat iff it fits, else exploded (fit-or-explode).
/// Total — [`classify`] maps every `&[Part]` to exactly one shape, so this is an
/// exhaustive `match` with no precondition and no silent mis-format.
///
/// `neck_tail_fuses` is the construct's fusion fact: whether the neck's last token would
/// fuse with the tail's first. It is consulted ONLY in the trailing-neck empty-body
/// branch, where `assemble` abuts them — the construct owns the tokens, `assemble` owns
/// the abutment (see [`trailing_neck`]). Only a rule / integrity-constraint statement
/// computes it; every other caller passes `false` (the conditional literal can too,
/// because its tail is always empty — the `.` belongs to the enclosing rule).
pub(super) fn assemble(
    b: &mut DocBuilder,
    parts: &[Part],
    indent: i32,
    neck_width: i32,
    neck_tail_fuses: bool,
) -> NodeId {
    match classify(parts) {
        Spine::LeadingNeck { neck, body, tail } => leading_neck(b, neck, body, tail, neck_width),
        Spine::TrailingNeck {
            head,
            neck,
            body,
            tail,
        } => trailing_neck(b, head, neck, body, tail, indent, neck_tail_fuses),
        Spine::Bracketed {
            name,
            opener,
            body,
            closer,
            tail,
        } => bracketed(b, name, opener, body, closer, tail, indent),
        Spine::Separated { items } => {
            let body = separated(b, items);
            b.group(body)
        }
    }
}

/// Index of the first part whose role satisfies `pred`.
fn position(parts: &[Part], pred: impl Fn(Role) -> bool) -> Option<usize> {
    parts.iter().position(|p| pred(p.role))
}

/// Concatenate the parts' docs in source order (no separators inserted). An empty
/// slice yields `nil`.
fn concat(b: &mut DocBuilder, parts: &[Part]) -> NodeId {
    let items: Vec<NodeId> = parts.iter().map(|p| p.doc).collect();
    b.seq(&items)
}

/// A separated sequence `e0 ⊕ sep0 ⊕ brk ⊕ e1 ⊕ … ⊕ en`: each `Separator(sep)` trails
/// its element and is followed by the break point its `Sep` selects — a `line`
/// (`Spaced`) or a `softline` (`Tight`). Returns the bare concatenation — the caller
/// wraps it in the group/nest that scopes the break. PRECONDITION: separators are infix
/// (between two elements); a trailing separator would leave a dangling break point.
fn separated(b: &mut DocBuilder, parts: &[Part]) -> NodeId {
    let mut items = Vec::with_capacity(parts.len() * 2);
    for part in parts {
        items.push(part.doc);
        if let Role::Separator(sep) = part.role {
            let brk = match sep {
                Sep::Spaced => b.line(),
                Sep::Tight => b.softline(),
            };
            items.push(brk);
        }
    }
    b.seq(&items)
}

/// Split a trailing run of `Tail` parts off the end: `(body, tail)`.
fn split_tail(parts: &[Part]) -> (&[Part], &[Part]) {
    let body_len = parts
        .iter()
        .rposition(|p| p.role != Role::Tail)
        .map_or(0, |i| i + 1);
    parts.split_at(body_len)
}

/// Trailing connective neck (a headed rule `head :- body`, or a conditional
/// literal `literal : condition`): `head :- ` then the body drops to a relative
/// `indent` block. The non-breaking `space` keeps the neck on the head's line, so
/// it is never stranded (Invariant 1); the whole is one group, so it either fits
/// flat or explodes (fit-or-explode). An EMPTY body — the degenerate empty-
/// condition conditional literal `p :`, a valid error-free parse — hangs nothing:
/// `head : ` with no drop-line, mirroring [`leading_neck`]'s bodyless `:- .`.
/// (Without the empty-body branch the unconditional drop-line would leave a
/// trailing space after the neck.)
fn trailing_neck(
    b: &mut DocBuilder,
    head: &[Part],
    neck: &Part,
    body: &[Part],
    tail: &[Part],
    indent: i32,
    neck_tail_fuses: bool,
) -> NodeId {
    // The head is its OWN group over a `separated` sequence. `separated` (not `concat`)
    // so a multi-part head — an `optimize_element`'s `weight, terms` prefix — gets its
    // separator spacing; the wrapping `group` so that prefix breaks ONLY if the head
    // itself overflows, independent of the condition hanging (recursive-minimal). For a
    // single-part head (a rule's head atom, an element's content node, a directive's
    // keyword+operand) `separated` is just the lone doc and the extra group is inert.
    let head_seq = separated(b, head);
    let head_doc = b.group(head_seq);
    let neck_doc = neck.doc;
    let space = b.space();
    let tail_doc = concat(b, tail);
    let whole = if body.is_empty() {
        // Seam self-heal: the empty body abuts `neck_doc` and `tail_doc`. If the
        // neck's last token would fuse with the tail's first (a future `:`-neck before a
        // `-`/`~` tail → `:-`), interpose a space — never an `≈` violation. Dormant
        // today (every empty-body trailing neck has an empty tail); a defensive belt.
        if neck_tail_fuses {
            let gap = b.space();
            b.seq(&[head_doc, space, neck_doc, gap, tail_doc])
        } else {
            b.seq(&[head_doc, space, neck_doc, tail_doc])
        }
    } else {
        let body_seq = separated(b, body);
        let drop_line = b.line();
        let dropped = b.seq(&[drop_line, body_seq]);
        let nested = b.nest(indent, dropped);
        // The tail is nested at the body's drop too (mirroring [`leading_neck`]'s tail
        // nest), so a breakable `[ … ]` directive tail composes additively (Invariant
        // 2): a `#heuristic a : c. [w, t]` whose tail overflows explodes its interior to
        // indent + indent and dedents its closer `]` to the body level (indent), never to
        // the ambient (column 0 — the rejected "4→0" shape). The glued `.` carries no break
        // point, so the nest only relocates the tail bracket's own breaks — a rule's lone
        // `.` tail stays glued to the last body line, unaffected.
        let nested_tail = b.nest(indent, tail_doc);
        b.seq(&[head_doc, space, neck_doc, nested, nested_tail])
    };
    b.group(whole)
}

/// Leading connective neck (an integrity / weak constraint): `:- ` then the first
/// body element rides the neck line and the rest hang at a relative `neck_width`.
/// A non-empty body is the normal case; the degenerate bodyless constraint `:- .`
/// (a valid, error-free parse with no body node) renders `:- .` — the empty
/// sequence is `nil`, so nothing hangs between the neck's space and the tail. When
/// BOTH body and tail are empty (a bare `:` theory element, `&a{ : }`), the neck hangs
/// nothing: just the neck, no trailing space — mirroring [`trailing_neck`]'s empty-body
/// guard (without it the unconditional space would dangle, e.g. `&a{ :  }`).
fn leading_neck(
    b: &mut DocBuilder,
    neck: &Part,
    body: &[Part],
    tail: &[Part],
    neck_width: i32,
) -> NodeId {
    let neck_doc = neck.doc;
    if body.is_empty() && tail.is_empty() {
        return b.group(neck_doc);
    }
    let body_seq = separated(b, body);
    let nested = b.nest(neck_width, body_seq);
    let space = b.space();
    // The tail is nested at the body's hang too, so a breakable `[ … ]` weak-constraint
    // tail composes additively (Invariant 2): its interior explodes to neck_width +
    // indent and its dedented closer lands at neck_width, never at the ambient (column 0).
    // The glued `.` and `]` carry no break point, so the nest only relocates the tail
    // bracket's own breaks — an integrity constraint's lone `.` tail is unaffected.
    let tail_doc = concat(b, tail);
    let nested_tail = b.nest(neck_width, tail_doc);
    let whole = b.seq(&[neck_doc, space, nested, nested_tail]);
    b.group(whole)
}

/// A bracketed block: `name{` opens an `indent`-deep body that explodes one
/// element per line with a dedented closer (`name{\n    e0;\n    e1\n}`), or stays
/// flat (`name{ e0; e1 }`) when it fits. The interior pad (`{ … }` vs `(…)`) is
/// the opener's [`Pad`]. The closerless branch is the unreachable totality
/// backstop: every bracket is matched, and a MISSING closer degrades the whole
/// construct to verbatim upstream (`has_unformattable_child`) before this runs.
fn bracketed(
    b: &mut DocBuilder,
    name: &[Part],
    opener: &Part,
    body: &[Part],
    closer: Option<&Part>,
    tail: &[Part],
    indent: i32,
) -> NodeId {
    // `bracketed` is dispatched only at a `Bracket` opener (see `classify`).
    let Role::Opener(OpenerKind::Bracket(pad)) = opener.role else {
        unreachable!("bracketed runs only at a Bracket opener")
    };
    let name_doc = concat(b, name);
    let opener_doc = opener.doc;
    let tail_doc = concat(b, tail);
    // An empty bracket hugs (`{}` / `()`), never a doubled pad from the open and
    // close breaks collapsing onto one line.
    if body.is_empty() {
        let whole = match closer {
            Some(c) => b.seq(&[name_doc, opener_doc, c.doc, tail_doc]),
            None => b.seq(&[name_doc, opener_doc, tail_doc]),
        };
        return b.group(whole);
    }
    let body_seq = separated(b, body);
    let open_break = pad_break(b, pad);
    let inner = b.seq(&[open_break, body_seq]);
    let nested = b.nest(indent, inner);
    let whole = match closer {
        Some(c) => {
            let close_break = pad_break(b, pad);
            b.seq(&[name_doc, opener_doc, nested, close_break, c.doc, tail_doc])
        }
        None => b.seq(&[name_doc, opener_doc, nested, tail_doc]),
    };
    b.group(whole)
}

/// The break padding a bracket's interior: a `line` (flat → space) for a spaced
/// brace, a `softline` (flat → nothing) for a tight paren. Broken, both render a
/// newline + indent, so the pad governs only the flat form.
fn pad_break(b: &mut DocBuilder, pad: Pad) -> NodeId {
    match pad {
        Pad::Spaced => b.line(),
        Pad::Tight => b.softline(),
    }
}

#[cfg(test)]
mod tests {
    use super::{assemble, OpenerKind, Pad, Part, Role, Sep};
    use doc::DocBuilder;
    use proptest::prelude::*;

    /// A faux-CST harness: build leaf `Part`s from literal source slices into a
    /// backing `src`, so the assembler is tested in isolation from real lowering.
    struct Harness {
        builder: DocBuilder,
        src: String,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                builder: DocBuilder::new(),
                src: String::new(),
            }
        }

        /// A leaf `Part` with `role`, appending `text` to the backing source.
        fn part(&mut self, role: Role, text: &str) -> Part {
            let start = u32::try_from(self.src.len()).expect("test source fits in u32");
            self.src.push_str(text);
            let doc = self.builder.leaf(start, text);
            Part { role, doc }
        }

        /// Assemble `parts` and render at `width` (no neck/tail seam — `false`).
        fn render(mut self, parts: &[Part], indent: i32, neck_width: i32, width: usize) -> String {
            let root = assemble(&mut self.builder, parts, indent, neck_width, false);
            doc::render(&self.builder.finish(root), &self.src, width)
        }

        /// Assemble with an explicit empty-body neck/tail seam flag, then render.
        fn render_seam(
            mut self,
            parts: &[Part],
            indent: i32,
            neck_width: i32,
            width: usize,
            neck_tail_fuses: bool,
        ) -> String {
            let root = assemble(
                &mut self.builder,
                parts,
                indent,
                neck_width,
                neck_tail_fuses,
            );
            doc::render(&self.builder.finish(root), &self.src, width)
        }
    }

    #[test]
    fn assembler_headed_rule_body_drops_at_indent_4() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "reachable(X, Y)"),
            h.part(Role::Opener(OpenerKind::Connective), ":-"),
            h.part(Role::Element, "edge(X, Z)"),
            h.part(Role::Separator(Sep::Spaced), ","),
            h.part(Role::Element, "reachable(Z, Y)"),
            h.part(Role::Tail, "."),
        ];
        let out = h.render(&parts, 4, 3, 20);
        assert_eq!(
            out,
            "reachable(X, Y) :-\n    edge(X, Z),\n    reachable(Z, Y)."
        );
    }

    #[test]
    fn assembler_headed_rule_stays_flat_when_it_fits() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "reachable(X, Y)"),
            h.part(Role::Opener(OpenerKind::Connective), ":-"),
            h.part(Role::Element, "edge(X, Z)"),
            h.part(Role::Separator(Sep::Spaced), ","),
            h.part(Role::Element, "reachable(Z, Y)"),
            h.part(Role::Tail, "."),
        ];
        let out = h.render(&parts, 4, 3, 100);
        assert_eq!(out, "reachable(X, Y) :- edge(X, Z), reachable(Z, Y).");
    }

    #[test]
    fn assembler_constraint_neck_leads_first_element_rides_hang_3() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Opener(OpenerKind::Connective), ":-"),
            h.part(Role::Element, "assign(T, S1)"),
            h.part(Role::Separator(Sep::Spaced), ","),
            h.part(Role::Element, "assign(T, S2)"),
            h.part(Role::Separator(Sep::Spaced), ","),
            h.part(Role::Element, "S1 < S2"),
            h.part(Role::Tail, "."),
        ];
        let out = h.render(&parts, 4, 3, 20);
        assert_eq!(out, ":- assign(T, S1),\n   assign(T, S2),\n   S1 < S2.");
    }

    #[test]
    fn assembler_constraint_stays_flat_when_it_fits() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Opener(OpenerKind::Connective), ":-"),
            h.part(Role::Element, "assign(T, S1)"),
            h.part(Role::Separator(Sep::Spaced), ","),
            h.part(Role::Element, "S1 < S2"),
            h.part(Role::Tail, "."),
        ];
        let out = h.render(&parts, 4, 3, 100);
        assert_eq!(out, ":- assign(T, S1), S1 < S2.");
    }

    #[test]
    fn assembler_tight_bracket_explodes_with_a_dedented_closer() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "f"),
            h.part(Role::Opener(OpenerKind::Bracket(Pad::Tight)), "("),
            h.part(Role::Element, "alpha"),
            h.part(Role::Separator(Sep::Spaced), ","),
            h.part(Role::Element, "beta"),
            h.part(Role::Closer, ")"),
        ];
        let out = h.render(&parts, 4, 3, 6);
        assert_eq!(out, "f(\n    alpha,\n    beta\n)");
    }

    #[test]
    fn assembler_tight_bracket_hugs_its_interior_when_flat() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "f"),
            h.part(Role::Opener(OpenerKind::Bracket(Pad::Tight)), "("),
            h.part(Role::Element, "a"),
            h.part(Role::Separator(Sep::Spaced), ","),
            h.part(Role::Element, "b"),
            h.part(Role::Closer, ")"),
        ];
        let out = h.render(&parts, 4, 3, 80);
        assert_eq!(out, "f(a, b)"); // tight: no interior pad
    }

    #[test]
    fn assembler_spaced_bracket_pads_its_interior_when_flat() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "#count"),
            h.part(Role::Opener(OpenerKind::Bracket(Pad::Spaced)), "{"),
            h.part(Role::Element, "a"),
            h.part(Role::Separator(Sep::Spaced), ";"),
            h.part(Role::Element, "b"),
            h.part(Role::Closer, "}"),
        ];
        let out = h.render(&parts, 4, 3, 80);
        assert_eq!(out, "#count{ a; b }"); // name abuts opener; interior padded
    }

    #[test]
    fn assembler_spaced_bracket_explodes_with_a_dedented_closer() {
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "#count"),
            h.part(Role::Opener(OpenerKind::Bracket(Pad::Spaced)), "{"),
            h.part(Role::Element, "alpha"),
            h.part(Role::Separator(Sep::Spaced), ";"),
            h.part(Role::Element, "beta"),
            h.part(Role::Closer, "}"),
        ];
        let out = h.render(&parts, 4, 3, 8);
        assert_eq!(out, "#count{\n    alpha;\n    beta\n}"); // exploded pad is a newline
    }

    #[test]
    fn assembler_empty_bracket_has_no_interior_pad() {
        // Both pads collapse an empty body to a hugged pair, never `(  )`/`{  }`.
        for (pad, open, close, want) in [
            (Pad::Tight, "(", ")", "f()"),
            (Pad::Spaced, "{", "}", "f{}"),
        ] {
            let mut h = Harness::new();
            let parts = vec![
                h.part(Role::Element, "f"),
                h.part(Role::Opener(OpenerKind::Bracket(pad)), open),
                h.part(Role::Closer, close),
            ];
            let out = h.render(&parts, 4, 3, 80);
            assert_eq!(out, want);
        }
    }

    #[test]
    fn classify_dispatches_the_four_shapes() {
        use super::{classify, Spine};
        let mut b = DocBuilder::new();
        let mut mk = |role| Part { role, doc: b.nil() };
        let leading = [mk(Role::Opener(OpenerKind::Connective)), mk(Role::Element)];
        assert!(matches!(classify(&leading), Spine::LeadingNeck { .. }));
        let trailing = [
            mk(Role::Element),
            mk(Role::Opener(OpenerKind::Connective)),
            mk(Role::Element),
        ];
        assert!(matches!(classify(&trailing), Spine::TrailingNeck { .. }));
        let bracket = [
            mk(Role::Element),
            mk(Role::Opener(OpenerKind::Bracket(Pad::Spaced))),
            mk(Role::Element),
            mk(Role::Closer),
        ];
        assert!(matches!(
            classify(&bracket),
            Spine::Bracketed {
                closer: Some(_),
                ..
            }
        ));
        let plain = [
            mk(Role::Element),
            mk(Role::Separator(Sep::Spaced)),
            mk(Role::Element),
        ];
        assert!(matches!(classify(&plain), Spine::Separated { .. }));
        assert!(matches!(classify(&[]), Spine::Separated { .. }));
    }

    #[test]
    fn tight_separator_is_a_softline_no_flat_pad_but_breaks() {
        // A Tight separator hugs when flat (SoftLine pads nothing) and breaks one-per-
        // line when its bracket explodes (SoftLine → newline) — never a non-break-point
        // (fit-or-explode binds nested lists). Contrast the Spaced separator, a
        // Line that pads a space when flat.
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "f"),
            h.part(Role::Opener(OpenerKind::Bracket(Pad::Tight)), "("),
            h.part(Role::Element, "a"),
            h.part(Role::Separator(Sep::Tight), ","),
            h.part(Role::Element, "b"),
            h.part(Role::Closer, ")"),
        ];
        assert_eq!(h.render(&parts, 4, 3, 80), "f(a,b)"); // flat: SoftLine pads nothing
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "f"),
            h.part(Role::Opener(OpenerKind::Bracket(Pad::Tight)), "("),
            h.part(Role::Element, "alpha"),
            h.part(Role::Separator(Sep::Tight), ","),
            h.part(Role::Element, "beta"),
            h.part(Role::Closer, ")"),
        ];
        assert_eq!(h.render(&parts, 4, 3, 6), "f(\n    alpha,\n    beta\n)"); // broken
    }

    #[test]
    fn empty_body_trailing_neck_self_heals_a_fusing_seam() {
        // An empty-body `:`-neck whose tail begins with `-` would fuse to `:-`; the
        // self-heal interposes a space iff the construct reports the seam fuses. No
        // grammar input produces this (the conditional `:` has an empty tail); the
        // guard is a defensive belt, exercised here via the faux Harness.
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "p"),
            h.part(Role::Opener(OpenerKind::Connective), ":"),
            h.part(Role::Tail, "-q"),
        ];
        assert_eq!(h.render_seam(&parts, 4, 3, 80, true), "p : -q"); // fuses → healed
        let mut h = Harness::new();
        let parts = vec![
            h.part(Role::Element, "p"),
            h.part(Role::Opener(OpenerKind::Connective), ":"),
            h.part(Role::Tail, "."),
        ];
        assert_eq!(h.render_seam(&parts, 4, 3, 80, false), "p :."); // no fuse → unchanged
    }

    /// A non-opener role: element, closer, separator, or tail — a building block for
    /// [`any_role`]. Exercises the malformations (a stray closer, a leading or trailing
    /// separator, a misplaced tail) that `assemble` must still handle by interposing
    /// whitespace only.
    fn non_opener_role() -> impl Strategy<Value = Role> {
        prop_oneof![
            Just(Role::Element),
            Just(Role::Closer),
            Just(Role::Separator(Sep::Spaced)),
            Just(Role::Separator(Sep::Tight)),
            Just(Role::Tail),
        ]
    }

    /// One structural opener — a connective neck or either bracket pad.
    fn any_opener() -> impl Strategy<Value = Role> {
        prop_oneof![
            Just(Role::Opener(OpenerKind::Connective)),
            Just(Role::Opener(OpenerKind::Bracket(Pad::Spaced))),
            Just(Role::Opener(OpenerKind::Bracket(Pad::Tight))),
        ]
    }

    /// Any role — element, closer, separator, tail, OR a structural opener. `classify`
    /// is total (the ≤1-opener `debug_assert` is gone), so the property below
    /// holds over ARBITRARY role sequences.
    fn any_role() -> impl Strategy<Value = Role> {
        prop_oneof![non_opener_role(), any_opener()]
    }

    /// An arbitrary `(Role, text)` spine — any roles, any length, no well-formedness
    /// constraint (totality is now structural). Exercises every `classify` dispatch path
    /// and the malformed cases (multiple openers, a stray closer, a leading or trailing
    /// separator) that `assemble` must still handle by interposing whitespace only.
    fn arbitrary_spec() -> impl Strategy<Value = Vec<(Role, String)>> {
        proptest::collection::vec((any_role(), "[a-zA-Z0-9]{1,3}"), 0..12)
    }

    /// Build `spec` into a fresh harness, assemble, and render at `width`. A fresh
    /// build per call because `Part`s hold `NodeId`s into one builder and
    /// `Harness::render_seam` consumes it.
    fn assemble_and_render(
        spec: &[(Role, String)],
        indent: i32,
        neck_width: i32,
        width: usize,
        neck_tail_fuses: bool,
    ) -> String {
        let mut h = Harness::new();
        let parts: Vec<Part> = spec
            .iter()
            .map(|(role, text)| h.part(*role, text))
            .collect();
        h.render_seam(&parts, indent, neck_width, width, neck_tail_fuses)
    }

    proptest! {
        /// Invariant W (the Hoare post-condition): `assemble` is doc-
        /// and order-preserving for ALL role assignments — it only interposes
        /// whitespace. Equivalently, stripping every whitespace char from the output
        /// reproduces the parts' texts concatenated in source order, at every width,
        /// and `assemble` never panics. This is the empirical proof that a malformed
        /// spine (multiple openers, a stray closer, a trailing separator, …) is at worst
        /// cosmetic mis-layout, never an `≈` violation. `classify` is total
        /// (the ≤1-opener `debug_assert` is gone), so the spec is now ARBITRARY role
        /// sequences — the multi-opener case is exercised here, not deferred. The
        /// empty-body neck/tail seam flag is varied too, so the self-heal branch is
        /// exercised (a healed space is still whitespace, so Invariant W is preserved).
        #[test]
        fn assemble_only_interposes_whitespace(
            spec in arbitrary_spec(),
            indent in 0i32..8,
            neck_width in 0i32..8,
            neck_tail_fuses in any::<bool>(),
        ) {
            let concatenated: String = spec.iter().map(|(_, t)| t.as_str()).collect();
            for width in [1usize, 4, 16, 80, 100_000] {
                let out = assemble_and_render(&spec, indent, neck_width, width, neck_tail_fuses);
                let stripped: String = out.chars().filter(|c| !c.is_whitespace()).collect();
                prop_assert_eq!(
                    &stripped,
                    &concatenated,
                    "width {}: assemble must only interpose whitespace (Invariant W)",
                    width
                );
            }
        }
    }
}
