//! Frozen-layout golden corpus: the one thing `verify` cannot check —
//! that the output matches the *house style*. Each case is pinned at an exact width
//! and triple-checked: the exact layout (the golden's point), `≈` token-stream safety
//! (can never silently freeze an unsafe reformatting), and idempotence (the frozen
//! output is a formatting fixpoint).
//!
//! This file holds the worked gallery, pinned at its stated widths. For cases 1/2/5/9
//! the formatter's output is the gallery's hand-drawn form verbatim (the hand-drawn
//! form is exact here). For 3/4/6/7/8 the gallery's *drawing* is illustrative, not
//! exact — its element/body lines (25-28 cols) overflow the stated narrow widths, so
//! the formatter correctly explodes them further under recursive-minimal; the oracle
//! there is the recursive-minimal principle, verified per case. The fresh cases
//! (weak/theory/disjunctive/comment/bottom edges) live alongside, each verified
//! against its layout rule before freezing.
//!
//! Run: `cargo test -p kallos --test golden`.

use kallos::{format, verify, Style};

/// Assert `input` formats at `width` to exactly `expected`, AND that the reformatting
/// is `≈`-safe, AND that `expected` is a formatting fixpoint. The three are
/// independent: layout fidelity (the golden's point), safety (lifted into the
/// harness so a golden can never encode a token-stream change), and stability.
fn golden(input: &str, width: usize, expected: &str) {
    let style = Style::default().with_line_width(width);
    assert_eq!(
        format(input, &style),
        expected,
        "layout mismatch @ width {width}"
    );
    assert!(
        verify(input, &style).is_ok(),
        "golden input is not ≈-safe @ width {width}: {input:?}"
    );
    assert_eq!(
        format(expected, &style),
        expected,
        "golden output is not idempotent @ width {width}"
    );
}

// ----- worked gallery, pinned at the gallery widths -----

#[test]
fn gallery_headed_rule_neck_trails_head_body_indent_4() {
    // [width 20] neck trails the head; body drops to a relative indent-4 block.
    golden(
        "reachable(X,Y) :- edge(X,Z), reachable(Z,Y).",
        20,
        "reachable(X, Y) :-\n    edge(X, Z),\n    reachable(Z, Y).\n",
    );
}

#[test]
fn gallery_constraint_neck_leads_first_literal_rides_hang_3() {
    // [width 20] neck leads, the first literal rides it, the rest hang at the neck
    // width (3).
    golden(
        ":- assign(T,S1), assign(T,S2), S1 < S2.",
        20,
        ":- assign(T, S1),\n   assign(T, S2),\n   S1 < S2.\n",
    );
}

#[test]
fn gallery_weak_constraint_tail_on_last_body_line() {
    // [width 20] same shape as the constraint, the `[w@p, …]` tail glued to the last
    // body line. The gallery DRAWS `assign(T, S). [C@1, T, S]` flat (28 cols), which
    // overflows width 20 — so the formatter recursive-minimally explodes `assign(…)`'s
    // args to fit; the tail stays glued to the closer's line.
    golden(
        ":~ cost(T,C), assign(T,S). [C@1, T, S]",
        20,
        ":~ cost(T, C),\n   assign(\n       T, S\n   ). [C@1, T, S]\n",
    );
}

#[test]
fn gallery_bounded_choice_head_explodes_body_drops() {
    // [width 24] bounds flank the brace spaced (`1 {`, `} 1`); the brace explodes one
    // element per line; the neck trails `} 1`; the body drops to indent 4. The
    // gallery DRAWS the elements flat (`assign(T,S) : slot(S);`, 25-26 cols), which
    // overflow width 24 — so each conditional element correctly hangs its condition
    // (recursive-minimal).
    golden(
        "1 { assign(T,S) : slot(S); waive(T) : optional(T) } 1 :- task(T).",
        24,
        "1 {\n    assign(T,S) :\n        slot(S);\n    waive(T) :\n        optional(T)\n} 1 :-\n    task(T).\n",
    );
}

#[test]
fn gallery_recursive_minimal_only_overflowing_condition_breaks() {
    // [width 28] the first element's 2-literal condition overflows and explodes one per
    // line; the short second element stays flat; the exploded head still drops the body.
    // The formatter's output is the gallery's hand-drawn form verbatim (width 28 IS
    // exact for this shape).
    golden(
        "1 { assign(T,S) : slot(S), available(S); waive(T) : optional(T) } 1 :- task(T).",
        28,
        "1 {\n    assign(T,S) :\n        slot(S),\n        available(S);\n    waive(T) : optional(T)\n} 1 :-\n    task(T).\n",
    );
}

#[test]
fn gallery_boundless_choice_brace_leads_its_own_line() {
    // [width 24] a boundless `{` (no bound/name) is a heavy opener, so it leads its own
    // line. As with the bounded choice, the gallery's flat elements overflow width 24,
    // so the conditions correctly explode.
    golden(
        "{ assign(T,S) : slot(S); waive(T) : optional(T) } :- task(T).",
        24,
        "{\n    assign(T,S) :\n        slot(S);\n    waive(T) :\n        optional(T)\n} :-\n    task(T).\n",
    );
}

#[test]
fn gallery_optimize_weight_tuple_breathes() {
    // [width 24] `#minimize` abuts its brace; weight tuples explode one per line; `}.`
    // glued. The gallery DRAWS both elements flat; at width 24 the first
    // (`C@1, T : cost(T,C);`, 23 cols) fits but the second (`P@2, T : penalty(T,P)`,
    // 25 cols) overflows, so only the second's condition hangs (recursive-minimal).
    golden(
        "#minimize{ C@1, T : cost(T,C); P@2, T : penalty(T,P) }.",
        24,
        "#minimize{\n    C@1, T : cost(T,C);\n    P@2, T :\n        penalty(T,P)\n}.\n",
    );
}

#[test]
fn gallery_constraint_nesting_aggregate_hangs_compose() {
    // [width 16] hang increments compose (the neck's 3, then the aggregate's 3+4=7).
    // Width 16 is pathologically narrow: the gallery DRAWS `S : assign(T,S)` flat
    // (22 cols at hang 7), which overflows, so the element fully explodes — condition
    // hung, then `assign(T,S)`'s args one per line. The residual `assign(` at indent 11
    // is unbreakable (indent + token), the minimum overflow recursive-minimal can reach.
    golden(
        ":- task(T), #count{ S : assign(T,S) } >= 2.",
        16,
        ":- task(T),\n   #count{\n       S :\n           assign(\n               T,\n               S\n           )\n   } >= 2.\n",
    );
}

#[test]
fn gallery_disjunctive_head_disjuncts_at_base_column() {
    // [width 22] a separated `;`-disjunction; disjuncts at the base column, separators
    // trailing, the body drops. The formatter's output is the gallery's form verbatim.
    golden(
        "color(N,r); color(N,g); color(N,b) :- node(N).",
        22,
        "color(N, r);\ncolor(N, g);\ncolor(N, b) :-\n    node(N).\n",
    );
}

// ----- fresh cases (weak / theory / disjunctive / comments / bottom edges) -----

#[test]
fn fresh_weak_constraint_overflowing_tail() {
    // [width 20] the compose-additively invariant: an overflowing weak-constraint
    // `[w@p, terms]` tail is a real breakable TIGHT bracket. The body atom
    // `p` rides the leading neck; the `.` glues; the `[ … ]` tail, when it overflows, explodes
    // one element per line at body-hang + indent = 3 + 4 = 7 (commas trailing) and dedents its
    // closer `]` to body-hang = 3 — never the old column-0 drop. Pinned at the single-atom body
    // so the tail's explosion is the SOLE break: it isolates the 3→7 tail shape with
    // no body-element interaction (see the NB below for the multi-literal-body residual).
    golden(
        ":~ p. [1@2, aaaa, bbbb, cccc]",
        20,
        ":~ p. [\n       1@2,\n       aaaa,\n       bbbb,\n       cccc\n   ]\n",
    );
}

#[test]
fn fresh_weak_constraint_overflowing_tail_explodes_glued_body() {
    // [width 24] a MULTI-literal body whose last literal is glued to an overflowing tail —
    // BOTH explode, and that is correct. There is no fill: under the greedy (Lindig) renderer
    // the tail is glued to `assign(T, S). [` with no break point between them, so `assign`'s
    // fit-check measures the flat tail, overflows, and `assign` breaks its args — while the
    // tail itself explodes compose-additively (interior 3→7, closer dedented to 3). This is the
    // SAME body-explodes-under-a-glued-tail behavior the
    // `gallery_weak_constraint_tail_on_last_body_line` (width 20) pins for the short-tail case;
    // here the tail is long, so it explodes 3→7 as well. Keeping `assign` flat would require a
    // fill renderer, which the no-fill rule forbids.
    golden(
        ":~ cost(T,C), assign(T,S). [C@1,T,S,extra1,extra2]",
        24,
        ":~ cost(T, C),\n   assign(\n       T, S\n   ). [\n       C@1,\n       T,\n       S,\n       extra1,\n       extra2\n   ]\n",
    );
}

#[test]
fn fresh_directive_overflowing_tail() {
    // [width 20] the compose-additively invariant, the DIRECTIVE-tail analogue of
    // `fresh_weak_constraint_overflowing_tail`. A `#heuristic atom : body. [weight, sign]`
    // routes through the trailing-neck spine (the `:` is a `Connective`): the head rides the
    // neck line, the body `cccc` drops to indent 4, the `.` glues, and the `[ … ]` tail — now
    // nested at the body hang (the trailing_neck fix mirroring `leading_neck`) — explodes
    // one element per line at body-indent + indent = 4 + 4 = 8 and dedents its closer `]` to
    // body-indent = 4. Before the fix the tail sat OUTSIDE the body's nest, so its interior
    // hung at 4 and its closer dropped to column 0 (the old "4→0" shape).
    golden(
        "#heuristic aaaa : cccc. [verylongweight@verylongprio, sign]",
        20,
        "#heuristic aaaa :\n    cccc. [\n        verylongweight@verylongprio,\n        sign\n    ]\n",
    );
}

#[test]
fn fresh_directive_no_colon_tail_composes_at_base_zero() {
    // [width 20] the no-`:`-neck directive path (whole-class consistency witness for the
    // trailing_neck fix). A `#external atom. [type]` carries NO connective neck, so it classifies
    // to the Separated spine, not trailing_neck: the whole `#external … [` stays on the base line
    // (column 0). Its `[ … ]` tail is the SAME breakable bracket, so when it overflows it explodes
    // its interior to base + indent = 0 + 4 = 4 and dedents its closer `]` to the construct base
    // (0). That is already compose-additive — the closer aligns with the directive's own column —
    // so this path needed NO change; it is frozen here to lock the whole-class shape. (`#const`'s
    // `[default]` tail can't take an arbitrary identifier — `[longname]` is a parse error — so
    // weak constraints, `#heuristic`, and this single-long-element `#external` are the only tails
    // that reach the breakable path.)
    golden(
        "#external aaaaaaaaaaaa. [verylongtypename]",
        20,
        "#external aaaaaaaaaaaa. [\n    verylongtypename\n]\n",
    );
}

#[test]
fn fresh_theory_atom_with_upper() {
    // [width 100] theory operators are never tightened — the `-` in `x(1) - x(2)`
    // and the `<=` upper guard keep surrounding spaces (the greedy-munch carve-out).
    golden(
        "&diff{ x(1)-x(2) } <= 3.",
        100,
        "&diff{ x(1) - x(2) } <= 3.\n",
    );
}

#[test]
fn fresh_theory_definition() {
    // [width 100] `#theory` definition: fits on one line, so the separated-sequence /
    // bracket machinery leaves it flat; `#theory` directive spaced, theory ops never
    // tightened. (Matches the equiv.rs round-trip fixture verbatim.)
    golden(
        "#theory t { d { - : 0, binary, left }; &a/0 : d, any }.",
        100,
        "#theory t { d { - : 0, binary, left }; &a/0 : d, any }.\n",
    );
}

#[test]
fn fresh_disjunctive_head() {
    // [width 100] disjunctive head, the FLAT complement to the exploded gallery case
    // (`gallery_disjunctive_head_disjuncts_at_base_column` @ 22): when the head fits, the
    // `;`-separated disjunction round-trips on one line with its `;` separators preserved as
    // authored. The de-housed input proves the formatter NORMALIZES to the house form — a
    // space after each `;` and around `:-` — without rewriting the `;` disjunction separator.
    golden(
        "win(P);lose(P);draw(P):-player(P).",
        100,
        "win(P); lose(P); draw(P) :- player(P).\n",
    );
}

#[test]
fn fresh_doc_comment() {
    // [width 100] a `doc_comment` (`%*! … *%`) is a first-class statement preserved
    // verbatim, positioned above the following statement under the blank-line rule.
    golden(
        "%*! p/1 a predicate *%\np(1).",
        100,
        "%*! p/1 a predicate *%\np(1).\n",
    );
}

#[test]
fn fresh_script_block() {
    // [width 100] the `#script (lang)` wrapper is formatted (directive spaced) and
    // the embedded body is preserved byte-for-byte (a verbatim span); `#end.` closes.
    golden(
        "#script (python)\nx = 1\n#end.",
        100,
        "#script (python)\nx = 1\n#end.\n",
    );
}

#[test]
fn fresh_fstring() {
    // [width 100] an `fstring` is a composite span kept verbatim in v0 (no recursion
    // into `fstring_field`); the surrounding `p(…)` formats normally.
    golden("p(f\"a{X}b\").", 100, "p(f\"a{X}b\").\n");
}

#[test]
fn fresh_argument_pool() {
    // [width 100] an argument pool surfaces as the repeated `arguments` field; its `;`
    // separators are preserved and spaced; fits flat at width 100.
    golden("p(a; b; c).", 100, "p(a; b; c).\n");
}

#[test]
fn fresh_empty_body_constraint() {
    // [width 100] totality: an empty-body integrity constraint must not crash; the
    // headless `:-` neck leads, `.` follows after a space (the canonical form `:- .`).
    golden(":- .", 100, ":- .\n");
}

#[test]
fn fresh_empty_condition_literal() {
    // [width 100] an empty-condition conditional literal (`elem :`) must not crash; the
    // trailing `:` rides `q(X)` and abuts the statement `.` (`:.`) — safe, since `:` only
    // fuses with a following `-`/`~`, not `.`. (Matches the equiv.rs `p :.` fixture.)
    golden("p(X) :- q(X) : .", 100, "p(X) :- q(X) :.\n");
}

#[test]
fn fresh_empty_weak_constraint() {
    // [width 100] totality: an empty-body weak constraint must not crash; the `.`
    // precedes the `[…]` tail (the canonical form `:~ . [0]`).
    golden(":~ . [0]", 100, ":~ . [0]\n");
}

#[test]
fn fresh_bottom_empty() {
    // [width 100] bottom case: an empty document formats to the empty string (no
    // content to terminate).
    golden("", 100, "");
}

#[test]
fn fresh_bottom_whitespace_only() {
    // [width 100] a whitespace-only document has no statements, so leading/trailing
    // stripping collapses it to the empty string.
    golden("   \n\n  \n", 100, "");
}

#[test]
fn fresh_bottom_comment_only() {
    // [width 100] bottom case: a comment-only file dangles its comment on the
    // `source_file` root, preserved, ending in exactly one terminating newline.
    golden("% just a comment\n", 100, "% just a comment\n");
}

#[test]
fn fresh_bottom_bare_percent() {
    // [width 100] a bare `%` is an empty line comment, preserved verbatim (no
    // trailing whitespace to strip), one terminating newline.
    golden("%\n", 100, "%\n");
}

#[test]
fn fresh_comment_interior() {
    // [width 100] an interior line comment forces its group to explode — the body
    // drops to indent 4 though it would fit flat; `% mid` rides `q,` with a ONE-space gap —
    // canonical (the deliberate 2026-06-18 rustfmt-POLS calibration in
    // `emit/reinject.rs`; the dead `Style::trailing_gap = 2` was removed).
    golden("p :- q, % mid\n   r.", 100, "p :-\n    q, % mid\n    r.\n");
}

#[test]
fn fresh_comment_dangling() {
    // [width 100] dangling: a comment after the last aggregate element, before the `}`
    // closer, attaches to the enclosing aggregate and emits on its own line just before the
    // closer; it forces the bracket group to explode (the elements `p; q` still fit flat).
    golden(
        "a :- #count{ p; q\n% dangling\n}.",
        100,
        "a :-\n    #count{\n        p; q\n        % dangling\n    }.\n",
    );
}

#[test]
fn fresh_comment_trailing_on_exploding() {
    // [width 20] the body overflows (`fits` treats the trailing `% c` as
    // zero-width, so the comment never feeds the explode decision), so the body explodes
    // one-per-line; `% c` rides the last line `cccc.` with a ONE-space gap (canonical,
    // the `emit/reinject.rs` calibration; see `fresh_comment_interior`).
    golden(
        "p :- aaaa, bbbb, cccc. % c",
        20,
        "p :-\n    aaaa,\n    bbbb,\n    cccc. % c\n",
    );
}

#[test]
fn fresh_comment_inline_block() {
    // [width 100] a single-line `block_comment` between two code tokens
    // forces the break in v0 (the group explodes), so `a, %* b *%` and `c` land on
    // separate body lines; the true in-place `Inline` role is a reserved seam.
    golden("p :- a, %* b *% c.", 100, "p :-\n    a, %* b *%\n    c.\n");
}
