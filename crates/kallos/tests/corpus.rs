//! Real-program corpus property fixtures: idempotence + `≈` over real `.lp` programs.
//! Committed: the clingofmt test-case INPUTS (MIT, inputs only — see
//! `tests/corpus/clingofmt/NOTICE`). Local depth: any directories named in
//! `KALLOS_EXTRA_CORPUS` (colon-separated) are additionally exercised — point it at
//! KRBOOK / kr-domains for a deeper off-repo run; those are not committed
//! (provenance). Error-bearing inputs (old lparse/DLV syntax, or constructs outside the
//! vendored grammar) are reported and skipped, never silently dropped (no silent caps;
//! the safe direction).
//!
//! The `.lp` discovery + corpus-root helpers are shared with the differential test via
//! `tests/common/` (DRY). Run: `cargo test -p kallos --test corpus`.

mod common;

use common::{corpus_dirs, lp_files};
use kallos::{format, has_error, verify, Style};

#[test]
fn corpus_is_idempotent_and_equivalent() {
    let style = Style::default(); // width 100
    let (mut checked, mut skipped) = (0usize, 0usize);
    for dir in corpus_dirs() {
        for path in lp_files(&dir) {
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            // The safe direction: a program the vendored grammar cannot parse is
            // reported and skipped, never a failure. Each skip names the file + reason.
            if has_error(&src) {
                eprintln!(
                    "SKIP (parse error / unsupported syntax): {}",
                    path.display()
                );
                skipped += 1;
                continue;
            }
            // Idempotence: a formatted program is a formatting fixpoint.
            let once = format(&src, &style);
            let twice = format(&once, &style);
            assert_eq!(once, twice, "not idempotent: {}", path.display());
            // `≈` safety: the reformatting preserves the code-token stream.
            assert!(
                verify(&src, &style).is_ok(),
                "not ≈-safe: {}",
                path.display()
            );
            checked += 1;
        }
    }
    eprintln!("corpus: {checked} checked, {skipped} skipped");
    assert!(
        checked > 0,
        "no corpus programs checked — fixtures missing?"
    );
}
