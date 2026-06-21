//! Spacing — one bracket-depth count for separators, plus a per-token table
//! for operators and delimiters. Every "tight"/"abut" join is ALSO a
//! correctness obligation: it is permitted only because `fusion::fuses` is false
//! for that seam (the construct rules assert this where real tokens meet).
//! Directive-keyword spacing — a predicate/function NAME abuts its
//! applicative opener (`f(`), but a DIRECTIVE KEYWORD is spaced from its operand
//! (`#edge (` does not abut) — is realized by the CONSTRUCT, not a table here: an
//! explicit `b.space()` is baked into the keyword `Element` (the
//! `lower_theory_directive` / `directive_parts` pattern), so no per-token abut
//! table is needed.

/// A join decision between two adjacent tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sep {
    /// No space and no break point (an abutment).
    Tight,
    /// A space when flat, a break point when broken.
    Spaced,
}

/// Comma / separator spacing: tight only when at least two brackets deep, so a
/// top-level body conjunction (`b1, b2`) and a single-bracket argument list
/// (`conflict(T1, T2)`) stay spaced, but a nested tuple (`assign(T,S)` inside a
/// brace) tightens.
pub(super) fn comma_spacing(bracket_depth: usize) -> Sep {
    if bracket_depth >= 2 {
        Sep::Tight
    } else {
        Sep::Spaced
    }
}

/// Theory operators are NEVER tightened (greedy maximal munch: `+ +` ≠ `++`), so
/// every `theory_operator` renders with surrounding spaces. Consumed by
/// `lower_theory_unparsed_term` / `lower_theory_operators` (the theory operator layer);
/// the normal-term operators use `term_operator_spacing` instead.
pub(super) fn theory_operator_spacing() -> Sep {
    Sep::Spaced
}

/// Sign spacing: `not` / `not not` are always spaced (`not p` ≠ `notp`), while a
/// classical or unary `-`/`~` abuts its operand. Consumed by `lower_body_literal`
/// for the default / double negation; the classical `-` arm is handled by the term layer.
pub(super) fn sign_spacing(sign: &str) -> Sep {
    match sign {
        "not" | "not not" => Sep::Spaced,
        _ => Sep::Tight,
    }
}

/// Arithmetic / term operators tighten one bracket-level earlier than commas:
/// spaced at the top body level (`X = Y + Z`), tight inside any bracket
/// (`foo(X, Y+Z)`). Consumed by `lower_binary_operation`.
pub(super) fn term_operator_spacing(bracket_depth: usize) -> Sep {
    if bracket_depth >= 1 {
        Sep::Tight
    } else {
        Sep::Spaced
    }
}

#[cfg(test)]
mod tests {
    use super::{comma_spacing, sign_spacing, term_operator_spacing, theory_operator_spacing, Sep};

    #[test]
    fn comma_spacing_is_bracket_depth_based() {
        assert_eq!(comma_spacing(0), Sep::Spaced); // body conjunction b1, b2
        assert_eq!(comma_spacing(1), Sep::Spaced); // conflict(T1, T2)
        assert_eq!(comma_spacing(2), Sep::Tight); // assign(T,S) inside a brace
    }

    #[test]
    fn theory_operators_are_never_tightened() {
        assert_eq!(theory_operator_spacing(), Sep::Spaced);
    }

    #[test]
    fn not_is_spaced_but_classical_minus_is_tight() {
        assert_eq!(sign_spacing("not"), Sep::Spaced); // lest `not p` → `notp`
        assert_eq!(sign_spacing("not not"), Sep::Spaced);
        assert_eq!(sign_spacing("-"), Sep::Tight); // classical -p / unary -X
    }

    #[test]
    fn term_operators_tighten_one_level_earlier_than_commas() {
        assert_eq!(term_operator_spacing(0), Sep::Spaced); // X = Y + Z
        assert_eq!(term_operator_spacing(1), Sep::Tight); // foo(X, Y+Z)
    }
}
