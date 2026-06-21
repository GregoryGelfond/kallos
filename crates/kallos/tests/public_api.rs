//! Public-API acceptance: the public self-check surface is reachable from OUTSIDE the
//! crate and has the exact shape consumers depend on — `verify(&str, &Style) ->
//! Result<(), Mismatch>`, callable on a bare `&str`, with `Mismatch` a `#[non_exhaustive]`
//! `std::error::Error`. An in-crate test cannot catch a missing `pub use`; this can.

use kallos::{verify, CommentMismatch, Mismatch, StructuralMismatch, Style};

#[test]
fn verify_is_public_and_ok_on_error_free_input() {
    // The full guarantee: error-free input formats `≈` to itself → Ok(()).
    assert_eq!(verify("a :- b, c.\n", &Style::default()), Ok(()));
    // Explosion at a narrow width does not break `≈`:
    assert_eq!(
        verify(":- d(X), e(X).\n", &Style::default().with_line_width(8)),
        Ok(()),
    );
}

#[test]
fn verify_is_total_on_error_bearing_input() {
    // `verify` does NOT short-circuit on errors — it runs the full check and
    // RETURNS (Ok or a Mismatch). Totality here means "no panic", not a verdict.
    let _ = verify("a :- :- .\n", &Style::default());
    let _ = verify("p(", &Style::default());
}

#[test]
fn mismatch_is_a_public_std_error() {
    fn assert_error<E: std::error::Error>() {}
    assert_error::<Mismatch>();
}

// The witness payload types are public (they appear in `Mismatch`'s variants), even
// though their fields are private in v0.
fn _payloads_are_nameable(_s: Option<StructuralMismatch>, _c: Option<CommentMismatch>) {}

#[test]
fn format_is_public_and_normalizes_layout() {
    // `format` is the public entry; a missing `pub use` would fail to COMPILE here.
    // Pure layout: re-spacing only, `≈` the input.
    assert_eq!(kallos::format("a:-b.\n", &Style::default()), "a :- b.\n");
    // document bottom cases: a missing final newline is added; empty stays empty.
    assert_eq!(kallos::format("a.", &Style::default()), "a.\n");
    assert_eq!(kallos::format("", &Style::default()), "");
}

#[test]
fn format_threads_the_style_line_width() {
    // The core contract: `format` honors the passed `style.line_width()` (it threads the
    // REAL `&Style` to the renderer, not a hardcoded default). A depth-0 body conjunction
    // that fits at 100 must explode into more lines at width 8.
    let src = "a :- bbbbbbbbbb, cccccccccc, dddddddddd, eeeeeeeeee, ffffffffff.\n";
    let narrow = kallos::format(src, &Style::default().with_line_width(8));
    let wide = kallos::format(src, &Style::default().with_line_width(100));
    assert_eq!(
        wide.lines().count(),
        1,
        "the rule fits at 100 — one line:\n{wide}"
    );
    assert!(
        narrow.lines().count() > wide.lines().count(),
        "a narrow width must wrap into more lines:\n--- narrow ---\n{narrow}--- wide ---\n{wide}"
    );
}
