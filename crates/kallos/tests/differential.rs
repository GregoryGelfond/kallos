//! Clingo differential test — the proxy-vs-authority closure.
//! Asserts clingo's canonical view of `s` equals its view of `format(s)` over the
//! probed edge seeds + the corpus. tree-sitter is a PROXY for clingo's lexer; clingo is
//! the authority. Equal canonical views ⇒ `format` preserved the code-token stream.
//!
//! Gated behind the `differential` feature (needs pixi + clingo); run via
//! `pixi run differential`. Programs clingo rejects are reported and skipped (triage) —
//! never a suite failure; only a genuine DIVERGENCE fails.
//!
//! The oracle sits behind the `canonical()` SEAM (it shells out to the pyclingo helper
//! `tests/differential/canonical.py`). Swapping it for a native Rust clingo oracle is a
//! one-function change here; the seeds, corpus wiring, and assertions do not move. The
//! shipped core links no libclingo and bundles no Python; this is test-only.

mod common;

use common::{corpus_dirs, lp_files};
use kallos::{format, has_error, Style};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The workspace root (`<root>/crates/kallos` → `<root>`), where `tests/differential`
/// and the corpus live.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves")
}

/// The result of asking the oracle for clingo's canonical view of a program.
#[derive(Debug)]
enum Canon {
    /// clingo accepted it; the canonical statement sequence (one per line).
    Ok(String),
    /// clingo rejected it (a parse error) — the caller triage-skips this program.
    Rejected,
    /// The oracle itself is unreachable (no pixi/python/pyclingo) — skip the whole test.
    Unavailable,
}

/// clingo's canonical statement sequence for `program`, via the pyclingo helper (the
/// `canonical()` seam — swappable to a native Rust oracle). Classifies the helper's exit: 0 → the
/// canonical text; 2 → clingo rejected the program; anything else (incl. a failed spawn
/// or a pyclingo import error) → the oracle is unavailable.
fn canonical(program: &str) -> Canon {
    let root = repo_root();
    let Ok(mut child) = Command::new("python")
        .arg(root.join("tests/differential/canonical.py"))
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return Canon::Unavailable;
    };
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(program.as_bytes())
        .expect("write program to the oracle");
    let out = child.wait_with_output().expect("await the oracle");
    match out.status.code() {
        Some(0) => Canon::Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Some(2) => Canon::Rejected,
        _ => Canon::Unavailable,
    }
}

/// Is the oracle reachable? Probes a trivial program; used to skip the whole test
/// gracefully when run outside the pixi env (rather than fail on every case).
fn oracle_available() -> bool {
    matches!(canonical("p.\n"), Canon::Ok(_))
}

/// The core assertion: where clingo accepts `src`, it must give `format(src)` the
/// identical canonical view. Returns whether the case was actually checked (false =
/// triage-skipped because tree-sitter or clingo rejected the input).
fn assert_differential(src: &str, label: &str) -> bool {
    if has_error(src) {
        eprintln!("SKIP (tree-sitter ERROR): {label}");
        return false;
    }
    let before = match canonical(src) {
        Canon::Ok(text) => text,
        Canon::Rejected => {
            eprintln!("SKIP (clingo rejects input): {label}");
            return false;
        }
        Canon::Unavailable => panic!("oracle became unavailable mid-run on {label}"),
    };
    let formatted = format(src, &Style::default());
    let after = match canonical(&formatted) {
        Canon::Ok(text) => text,
        other => panic!("format({label}) output is not clingo-parseable: {other:?}\n{formatted}"),
    };
    assert_eq!(
        before, after,
        "DIFFERENTIAL DIVERGENCE on {label}\n--- canonical(input) ---\n{before}\n--- canonical(format(input)) ---\n{after}"
    );
    true
}

#[test]
fn differential_edge_seeds() {
    if !oracle_available() {
        eprintln!(
            "SKIP differential_edge_seeds: oracle unavailable (run via `pixi run differential`)"
        );
        return;
    }
    // The 2026-06-14 probe seeds: octal, the greedy-munch witnesses, theory operators, and
    // double-negation. Each round-trips canonically where it parses. NOTE the two skip paths
    // fall to DIFFERENT inputs: `0o0` / `0o10` are rejected by TREE-SITTER (the `has_error`
    // gate → "SKIP (tree-sitter ERROR)"), so they never reach the oracle's `Canon::Rejected`
    // branch (`0o7` parses and IS checked). That clingo-exit-2 `Canon::Rejected` branch is
    // exercised instead by the 6 corpus `#include` files (`differential_over_corpus`), which
    // clingo rejects because the included files are absent.
    let seeds = [
        ("octal_0o7", "p(0o7).\n"),
        ("octal_0o10_ts_error", "p(0o10).\n"),
        ("octal_0o0_ts_error", "p(0o0).\n"),
        ("not_ab_neg", ":- not ab(X), not -flies(X), bird(X).\n"),
        ("not_neg_p", "q :- not -p.\n"),
        ("cond_neg_guard", "p :- q(X) : -r(X).\n"),
        ("theory_op_run", "&sum { x } >= 1 + 2.\n"),
        ("double_negation", "q :- not not p.\n"),
    ];
    let mut checked = 0;
    for (label, src) in seeds {
        if assert_differential(src, label) {
            checked += 1;
        }
    }
    eprintln!("edge seeds: {checked}/{} checked", seeds.len());
    assert!(checked > 0, "no edge seed survived to the differential");
}

#[test]
fn differential_over_corpus() {
    if !oracle_available() {
        eprintln!(
            "SKIP differential_over_corpus: oracle unavailable (run via `pixi run differential`)"
        );
        return;
    }
    // The committed corpus (`tests/corpus/`) plus any dirs in KALLOS_EXTRA_CORPUS
    // (colon-separated) for the local probe (KRBOOK, kr-domains) — both via the shared
    // `corpus_dirs()` helper. A missing/empty corpus is logged, not failed — the edge
    // seeds carry the always-on coverage; the corpus is additive.
    let (mut checked, mut skipped) = (0usize, 0usize);
    for dir in corpus_dirs() {
        for path in lp_files(&dir) {
            let src = std::fs::read_to_string(&path).expect("read corpus file");
            if assert_differential(&src, &path.display().to_string()) {
                checked += 1;
            } else {
                skipped += 1;
            }
        }
    }
    eprintln!("differential corpus: {checked} checked, {skipped} skipped");
    // Floor (mirroring `differential_edge_seeds`): the committed corpus has 17 `.lp` files, 11
    // of them clingo-checked (the 6 `#include` files are skipped — clingo rejects their absent
    // targets), so `checked` is ≥ 11 here. Asserting `checked > 0` keeps this test from passing
    // VACUOUSLY were the corpus ever emptied (fixtures removed).
    assert!(checked > 0, "differential corpus empty — fixtures missing?");
}
