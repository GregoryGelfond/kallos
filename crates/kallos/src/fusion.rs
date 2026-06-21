//! No-token-merge. `fuses(left, right)` is true iff emitting `left` and
//! `right` with NO space between them would re-lex into a DIFFERENT token
//! sequence. The emitter must keep a space wherever `fuses` is true. Centralized
//! here as the by-construction rule and the seam-test target.
//!
//! Semantics: `true` = "must keep spaced" (would re-lex differently); `false` =
//! "positively certified safe to abut". The predicate is *reachable-honest*:
//! `false` is returned only where a seam is positively certified non-fusing for
//! adjacencies the grammar can actually produce; grammar-impossible same-class
//! pairs take the safe value. Safety is asymmetric: `false` (allow-tighten)
//! requires certification, `true` is the default.
//!
//! # Preconditions and proof obligations (the load-bearing hypotheses)
//!
//! Recorded here so the obligations a correctness argument must discharge are
//! visible in one place rather than scattered through the arm comments.
//! - **Kind-driven classification (load-bearing).** The caller must class each token
//!   by its tree-sitter *node kind*, never by raw glyph. In theory context an operator
//!   run such as `:=` arrives pre-grouped as one greedy `theory_operator` node
//!   (grammar.js:420-430) → [`Class::TheoryOp`] → arm [1]; so the `Colon`/`Relation`
//!   arms never see theory glyphs, and there is no non-theory `:=`/`==` token (relations
//!   are exactly `> < >= <= = !=`, 501). Soundness rests on this — the emitter must honor it.
//! - **Theory-operator munch boundary (lifted).** Arm [1] is char-class-precise: a theory
//!   operator fuses with a neighbor iff the neighbor's FACING char is in the greedy munch
//!   set [`is_theory_op_char`] (the union of the four `theory_operator` alternatives,
//!   grammar.js:420-430). The bound on the munch is load-bearing IN the predicate, not
//!   folded into a blanket `true` — so the non-operator hard punctuation `( ) [ ] { } ,`
//!   abutments (the `#theory` guard set `{<=, >=}`) are positively certified safe, while
//!   the operator-class punctuation `; | @ . :` correctly stays fusing.
//! - **Unreachable pairs.** `*`+`*` and `*`+`**` (arm [3] → `true`) cannot occur: two
//!   binary operators never abut (only binary + unary `-`/`~` do). `.`+`.`→`..`,
//!   `<`+`=`→`<=`, and relation×relation are likewise grammar-unreachable, so their
//!   verdicts hold vacuously (had `.`+`.` been reachable, `...`→`[.., .]` is a real fusion).
//! - **String and neck tokens.** A `string`/`fstring` is self-delimiting by `"`, and the
//!   necks `:-`/`:~` are fixed CST tokens (692,697,715), not `colon`+op; all class as
//!   [`Class::HardPunct`] → arm [5] → `false`, which is sound.

/// The minimal facet of an emitted token `fuses` needs: its lexical class plus the
/// literal text (so seam-specific fusions like `:`+`-`=`:-` and `*`+`*`=`**` are
/// decidable).
///
/// `Tok` itself needs no `dead_code` guard: production builds it inline in
/// `tok_of` and the term-op seam check. The ergonomic constructors below are
/// test-only (`#[cfg(test)]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tok<'a> {
    pub(crate) class: Class,
    pub(crate) text: &'a str,
}

/// Lexical classes, lifted from the grammar's fixed-vs-greedy token structure:
/// binary/unary operators are fixed literals (grammar.js:284-348); `relation` is a
/// fixed token (501); `theory_operator` is a greedy `[…]+` run (420-430); `colon`
/// is external (33). Kind-driven dispatch (not a raw token-char switch) is what lets
/// the emitter assign these classes correctly despite aliasing.
///
/// `fuses` is a *total* oracle over these classes.
/// The production classifier `tok_of` is intentionally partial (it builds only the
/// classes the formatter's tightening seams can meet), so `Keyword`, `TheoryOp`, and
/// `Relation` are constructed only by the seam tests that verify the oracle over its
/// whole domain. Their `cfg_attr(not(test), expect(dead_code))` self-removes the
/// moment a future seam classifies that kind, so the model cannot silently rot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Ident,  // identifier / variable — maximal-munch word token (grammar.js:255+)
    Number, // maximal-munch word token, incl. 0x/0o/0b (grammar.js:255-261)
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tok_of is partial; see type doc")
    )]
    Keyword, // `not` (509) / `#show` / … — lexes into identifier chars
    TermOp, // fixed-length term operator: + - * / \ ** .. ^ ? & ~ (284-348)
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tok_of is partial; see type doc")
    )]
    TheoryOp, // greedy maximal-munch operator run (420-430) — NEVER abut an operator char
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tok_of is partial; see type doc")
    )]
    Relation, // = != < > <= >= — fixed token (501); sits between terms (504)
    Colon,  // the external `:` token (33) — forms the necks :- / :~
    HardPunct, // ( ) { } [ ] , ; | @ and the statement `.` — non-extensible
}

// Ergonomic constructors used only by the seam unit tests below; production builds
// `Tok` values inline (`tok_of` and the term-op seam check in `emit::constructs`).
#[cfg(test)]
impl<'a> Tok<'a> {
    pub(crate) fn ident(t: &'a str) -> Self {
        Self {
            class: Class::Ident,
            text: t,
        }
    }
    pub(crate) fn number(t: &'a str) -> Self {
        Self {
            class: Class::Number,
            text: t,
        }
    }
    pub(crate) fn keyword(t: &'a str) -> Self {
        Self {
            class: Class::Keyword,
            text: t,
        }
    }
    pub(crate) fn op(t: &'a str) -> Self {
        Self {
            class: Class::TermOp,
            text: t,
        }
    }
    pub(crate) fn theory_op(t: &'a str) -> Self {
        Self {
            class: Class::TheoryOp,
            text: t,
        }
    }
    pub(crate) fn relation(t: &'a str) -> Self {
        Self {
            class: Class::Relation,
            text: t,
        }
    }
    pub(crate) fn colon() -> Self {
        Self {
            class: Class::Colon,
            text: ":",
        }
    }
    pub(crate) fn lparen() -> Self {
        Self {
            class: Class::HardPunct,
            text: "(",
        }
    }
    pub(crate) fn comma() -> Self {
        Self {
            class: Class::HardPunct,
            text: ",",
        }
    }
}

/// True iff `left` and `right` abutted (no space) would re-lex into a token sequence
/// other than `[left, right]` — i.e. "must keep spaced". Reachable-honest: `false`
/// is returned ONLY where the seam is positively certified non-fusing for adjacencies
/// the grammar can actually produce; grammar-impossible same-class operator/relation/
/// dot pairs take the safe value and are documented as unreachable below. Safety is
/// asymmetric: `false` (allow-tighten) requires certification; `true` is the default.
#[must_use]
pub(crate) fn fuses(left: Tok, right: Tok) -> bool {
    use Class::{Colon, Ident, Keyword, Number, TermOp, TheoryOp};
    match (left.class, right.class) {
        // [1] A theory operator is a greedy maximal-munch run over the operator-char class
        //     S (`is_theory_op_char`; grammar.js:420-430). It fuses with a neighbor IFF the
        //     neighbor's FACING character is also in S — then abutting extends the munch
        //     across the seam into a different token (`- +`→`-+`, `-`+`:`→`-:`, `;`+`-`→`;-`).
        //     If the facing char is NOT in S — a word/number (letter/digit/`_`) or the
        //     non-operator hard punctuation `( ) [ ] { } ,` — the munch stops at the seam
        //     and both tokens survive: positively certified safe to abut. This is the
        //     precise refinement of the former blanket `true`:
        //     the structural fact that bounds the greedy munch is now load-bearing here,
        //     so the `#theory` guard-set abutments (`{<=, >=}`) are oracle-certified, not a
        //     carve-out. NOTE S includes `; | @ . : ~ ^`, so a theory op DOES fuse with
        //     those (a `false` here would be unsound). An empty token (impossible for a
        //     lexed token) takes the safe `true` via `is_none_or`.
        (TheoryOp, _) => right.text.chars().next().is_none_or(is_theory_op_char),
        (_, TheoryOp) => left.text.chars().next_back().is_none_or(is_theory_op_char),
        // [2] two word tokens merge by maximal munch (`not`+`p`=`notp`, `a`+`1`=`a1`,
        //     `1`+`2`=`12`). Also the safe value for the unreachable hex case `0`+`xa`.
        (Ident | Number | Keyword, Ident | Number | Keyword) => true,
        // [3] fixed term-ops: the only longer fixed op formable across a seam is `**`
        //     (292/342), and only from a leading bare `*`; `+-`, `**-`, `/-` etc. do not
        //     fuse (`1+-2*2**-3` re-lexes identically). No term-op text
        //     is `.`, so `..` cannot form here.
        (TermOp, TermOp) => left.text == "*" && right.text.starts_with('*'),
        // [4] a conditional/aggregate colon (525) abutting a leading `-`/`~` would
        //     ACCIDENTALLY form the rule-neck `:-` (692,697) or weak-neck `:~` (715),
        //     re-lexing the seam. The real necks are their own CST tokens, never
        //     `colon`+op; this arm guards the accidental case, e.g. `{p : -q}`.
        (Colon, TermOp) => right.text == "-" || right.text == "~",
        // [5] everything else is positively certified non-fusing for reachable seams:
        //     hard punctuation on either side (non-extensible; `.`+`.`→`..` is
        //     unreachable — statement dots are never adjacent, the interval `..` is a
        //     single token); relations sit between terms and never abut a fusing partner
        //     (501/504; `<`+`=`→`<=` unreachable); and a word meeting an operator/punct
        //     cannot extend into one token (`f(`, `p/`, `X,`, `not`+`-`). The
        //     always-space-`not` house rule is a spacing obligation enforced in the emitter,
        //     not a fusion — `fuses` stays exactly "would re-lex differently".
        _ => false,
    }
}

/// The character class a `theory_operator` greedily munches: the union of the four
/// `theory_operator` alternatives' character sets (grammar.js:420-430). A theory operator
/// abutting a token whose facing character is in this set would re-lex into a different
/// (longer) operator token — the hazard arm [1] of [`fuses`] guards. Characters OUTSIDE
/// the set — word/number characters and the hard punctuation `( ) [ ] { } ,` — stop the
/// munch, so a theory operator is safe to abut them. (The set is a superset of the fixed
/// term operators; it additionally contains `: ; |` and excludes nothing a theory operator
/// can contain. `@` and `.` are in it, so `&name`/intervals never abut a theory op tightly.)
fn is_theory_op_char(c: char) -> bool {
    matches!(
        c,
        '/' | '!'
            | '<'
            | '='
            | '>'
            | '+'
            | '-'
            | '*'
            | '\\'
            | '?'
            | '&'
            | '@'
            | '|'
            | ':'
            | ';'
            | '~'
            | '^'
            | '.'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // KEEP-SPACED: seams that would re-lex into a different token sequence if abutted.
    #[test]
    fn fusing_seams_must_stay_spaced() {
        // [2] word × word — maximal munch merges them into one token.
        assert!(fuses(Tok::keyword("not"), Tok::ident("p"))); // not+p = notp
        assert!(fuses(Tok::ident("a"), Tok::number("1"))); // a+1 = a1
        assert!(fuses(Tok::number("1"), Tok::number("2"))); // 1+2 = 12

        // [1] theory operators are greedy — any adjacent operator char munches across.
        assert!(fuses(Tok::theory_op("-"), Tok::theory_op("+"))); // -+ = one theory_operator
        assert!(fuses(Tok::theory_op("<"), Tok::op("="))); // theory op swallows '='

        // [3] the only fusing fixed term-op pair: * + * = ** (grammar.js:292,342).
        assert!(fuses(Tok::op("*"), Tok::op("*")));

        // [4] external colon before a sign forms the necks :- / :~.
        assert!(fuses(Tok::colon(), Tok::op("-"))); // :- (grammar.js:692,697)
        assert!(fuses(Tok::colon(), Tok::op("~"))); // :~ (grammar.js:715)
    }

    // CERTIFIED SAFE TO ABUT: positively non-fusing seams (`fuses` returns false).
    #[test]
    fn certified_safe_seams_may_tighten() {
        // hard punctuation on either side is non-extensible.
        assert!(!fuses(Tok::ident("f"), Tok::lparen())); // f(
        assert!(!fuses(Tok::ident("X"), Tok::comma())); // X,
        assert!(!fuses(Tok::comma(), Tok::ident("Y"))); // ,Y

        // signature p/2 — '/' is a fixed punctuation/term-op token (grammar.js:742).
        assert!(!fuses(Tok::ident("p"), Tok::op("/")));
        assert!(!fuses(Tok::op("/"), Tok::number("2")));

        // fixed term-op pairs that form no longer token tighten (1+-2*2**-3..).
        assert!(!fuses(Tok::op("+"), Tok::op("-"))); // +- is [+,-]
        assert!(!fuses(Tok::op("**"), Tok::op("-"))); // **- is [**,-]

        // `not` + `-` does NOT lexically fuse (not-p = not,-,p); the always-space-`not`
        // house rule lives in the emitter's spacing table, NOT in `fuses` (the predicate stays
        // exactly "would re-lex differently").
        assert!(!fuses(Tok::keyword("not"), Tok::op("-")));

        // relations sit between terms; never adjacent to a fusing partner (grammar.js:501,504).
        assert!(!fuses(Tok::ident("X"), Tok::relation("<=")));
        assert!(!fuses(Tok::relation("<="), Tok::number("1")));
    }

    // BOUNDARY WITNESSES: pin the exact edges of the model so a future "simplification"
    // that widens an arm is caught by a red test (this predicate is the axiom).
    #[test]
    fn model_boundary_witnesses() {
        // [3] the `**` rule keys on a LEADING BARE `*` (`left.text == "*"`), not on
        //     "either side is a `*`": only `*`+`*` fuses across the seam. The reachable
        //     `2*-3` seam (`*`+`-`) has `left.text == "*"` true, so it pins the `&&`
        //     guard's RIGHT conjunct (`right.starts_with('*')`) against over-firing.
        assert!(fuses(Tok::op("*"), Tok::op("**"))); // * ** : right starts with '*'
        assert!(!fuses(Tok::op("**"), Tok::op("*"))); // ** * : left is "**", not "*"
        assert!(!fuses(Tok::op("**"), Tok::op("**"))); // ** ** : left is "**", not "*"
        assert!(!fuses(Tok::op("*"), Tok::op("/"))); // * / : [*, /], no `*/` token
        assert!(!fuses(Tok::op("*"), Tok::op("-"))); // * - : reachable 2*-3 seam, [*, -]

        // [1] theory-op fusion is PRECISE (not blanket): a theory operator fuses with a
        //     neighbor IFF the neighbor's FACING char is itself a theory-operator char
        //     (`is_theory_op_char`). Against the NON-operator hard punctuation `( ) [ ] { } ,`
        //     or a word/number it does NOT fuse — the greedy munch stops at the seam. This
        //     is the structural fact the `#theory` guard set relies on (`{<=, >=}` abuts the
        //     braces and the comma safely — `lower_theory_atom_definition`):
        assert!(!fuses(Tok::theory_op("+"), Tok::lparen())); // + ( : `(` ∉ S, [+ , (]
        assert!(!fuses(Tok::theory_op("<="), Tok::comma())); // <=, : `,` ∉ S (guard set)
        assert!(!fuses(Tok::comma(), Tok::theory_op("+"))); // , + : `,` ∉ S, [, , +]
        assert!(!fuses(Tok::ident("p"), Tok::theory_op("+"))); // p + : word, `p` ∉ S
        assert!(!fuses(Tok::theory_op("+"), Tok::number("1"))); // + 1 : `1` ∉ S
                                                                //     Against an operator-class char it DOES fuse — another operator, OR the
                                                                //     operator-class punctuation `; | @ . : ~ ^` (all in S, all extend the munch):
        assert!(fuses(Tok::theory_op("+"), Tok::theory_op("-"))); // +- munches
        assert!(fuses(
            Tok {
                class: Class::HardPunct,
                text: ";"
            },
            Tok::theory_op("-")
        )); // ;- munches (the element-separator-then-op seam — kept spaced by spacing)
        assert!(fuses(
            Tok::theory_op("<="),
            Tok {
                class: Class::HardPunct,
                text: "."
            }
        )); // <=. munches (`.` ∈ S)

        // [4] the colon neck is sign-specific: `:`+`*` is `[:, *]`, not a neck;
        //     `:`+`=` is `[:, =]` (there is no non-theory `:=` token, grammar.js:501).
        assert!(!fuses(Tok::colon(), Tok::op("*")));
        assert!(!fuses(Tok::colon(), Tok::relation("=")));
        // arm [1] takes precedence over [4]: a colon meeting a theory-op keeps spaced.
        assert!(fuses(Tok::colon(), Tok::theory_op("+")));

        // [5] the interval `1..3` tightens — `..` is a single fixed term-op token —
        //     and so does `1..-2` (interval then a unary sign: [.., -], no `..-` token).
        assert!(!fuses(Tok::number("1"), Tok::op(".."))); // 1..
        assert!(!fuses(Tok::op(".."), Tok::number("3"))); // ..3
        assert!(!fuses(Tok::op(".."), Tok::op("-"))); // ..- : interval then unary sign

        // [5] hard punctuation never fuses with hard punctuation (e.g. `,` then `,`).
        assert!(!fuses(Tok::comma(), Tok::comma()));
    }
}
