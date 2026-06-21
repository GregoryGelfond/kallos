//! The embedded-code formatting hook. A `#script (lang) … #end`
//! body is opaque to the ASP formatter; this is the single seam through which a
//! language sub-formatter can be invoked (`ruff` for Python,
//! `stylua` for Lua — never a reimplementation). It is currently **the identity**.
//!
//! Contract: `format_embedded` is **total** — on any failure
//! (unknown language, sub-formatter error, missing binary) it returns the original
//! bytes unchanged, keeping whole-program `format` total and the region an honest
//! implicit `fmt: off`. Composition (once delegation is added): whole-program `≈` ∧ the
//! sub-formatter's AST-equivalence, with the side condition that the spliced output
//! re-lex to a SINGLE `code` token. The identity trivially satisfies the contract.
//!
//! Return type is `Cow` so a delegating version can return owned formatted text without
//! changing this signature; the identity always borrows. (Emitting owned, non-source text as a
//! Doc leaf needs a new owned-text Doc primitive — a reserved seam, out of scope:
//! the current `doc` builder leaf is an offset+length into `src` by construction, the
//! mechanism that keeps every leaf's text a verbatim source slice.)

use std::borrow::Cow;

/// Format an embedded `#script` body. Currently the identity (`Cow::Borrowed`).
// reason: the `code` body flows through the verbatim source path (the builder
// cannot emit non-source text); this hook is the named seam, called by
// `lower_script` to assert the identity contract and to host future delegation.
pub(super) fn format_embedded<'a>(_lang: &str, code: &'a str) -> Cow<'a, str> {
    Cow::Borrowed(code)
}

#[cfg(test)]
mod tests {
    use super::format_embedded;

    #[test]
    fn v0_is_the_identity_for_any_language() {
        let code = "\ndef main():\n    return 1\n";
        assert_eq!(format_embedded("python", code), code);
        assert_eq!(format_embedded("lua", code), code);
        assert_eq!(format_embedded("unknown", code), code);
    }
}
