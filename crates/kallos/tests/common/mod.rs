//! Shared `.lp` corpus-discovery helpers, DRY'd out of the corpus and differential test
//! binaries. This is a MODULE in a subdirectory, not a `tests/*.rs` integration
//! binary, so cargo does not compile it on its own — each consumer pulls it in with
//! `mod common;`. Both helpers below are used by BOTH consumers (`tests/corpus.rs` and
//! `tests/differential.rs`), so neither binary emits a dead-code warning.

use std::path::{Path, PathBuf};

/// All `.lp` files under `dir`, recursively, via an iterative work-list walk (no
/// recursion — defensive against pathological directory depth). A missing or unreadable
/// directory contributes nothing; the caller's `checked > 0` guard catches an empty union.
pub fn lp_files(dir: &Path) -> Vec<PathBuf> {
    let (mut files, mut stack) = (Vec::new(), vec![dir.to_path_buf()]);
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "lp") {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// Corpus roots: the committed `tests/corpus/` tree (the recursive drop-in extension
/// point) plus any directories named in `KALLOS_EXTRA_CORPUS` (colon-separated) for a
/// deeper local probe — point it at KRBOOK / kr-domains for an off-repo run.
/// The committed root is resolved from `CARGO_MANIFEST_DIR`, so it is identical whichever
/// binary links this module.
pub fn corpus_dirs() -> Vec<PathBuf> {
    let committed = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut dirs = vec![committed];
    if let Ok(extra) = std::env::var("KALLOS_EXTRA_CORPUS") {
        dirs.extend(
            extra
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        );
    }
    dirs
}
