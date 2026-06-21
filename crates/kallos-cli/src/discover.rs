//! Path selection: explicit files are formatted as-is (any extension —
//! the Unix posture); a `-` (or no paths) reads stdin; a directory is searched
//! for `.lp` files via the `ignore` crate, respecting `.gitignore` and skipping
//! hidden entries (so `.git/` is excluded), with `--include`/`--exclude` glob
//! refinement applied as a post-filter so `.gitignore` stays authoritative. A
//! non-existent path is an error.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;

use crate::cli::Cli;

/// One unit of work for the runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Read stdin, write stdout (the editor contract).
    Stdin,
    /// A concrete file on disk.
    File(PathBuf),
}

/// The result of expanding CLI paths: the ordered work list plus any non-fatal
/// directory-walk errors. An unreadable entry within a walked tree is reported
/// and skipped — not fatal to the whole run; only a bad EXPLICIT path or a
/// malformed glob is fatal (an `Err` return).
#[derive(Debug, Default)]
pub struct Discovered {
    pub targets: Vec<Target>,
    pub errors: Vec<String>,
}

/// Expand the CLI paths into a work list. No paths → stdin.
///
/// # Errors
/// Returns an error if an EXPLICIT path does not exist or is not a regular file
/// or directory, or if an `--include`/`--exclude` glob is malformed. A per-entry
/// failure DURING a directory walk is collected into [`Discovered::errors`]
/// instead (reported and skipped, not fatal).
pub fn collect_targets(cli: &Cli) -> Result<Discovered> {
    if cli.paths.is_empty() {
        return Ok(Discovered {
            targets: vec![Target::Stdin],
            errors: Vec::new(),
        });
    }
    let mut found = Discovered::default();
    for path in &cli.paths {
        if path.as_os_str() == "-" {
            found.targets.push(Target::Stdin);
        } else if path.is_dir() {
            walk_dir(path, cli, &mut found)
                .with_context(|| format!("searching directory `{}`", path.display()))?;
        } else if path.is_file() {
            found.targets.push(Target::File(path.clone()));
        } else if path.exists() {
            anyhow::bail!("not a regular file or directory: `{}`", path.display());
        } else {
            anyhow::bail!("path does not exist: `{}`", path.display());
        }
    }
    Ok(found)
}

/// Walk `dir` for `.lp` files (or the `--include` globs), honoring the in-tree
/// `.gitignore`, skipping hidden entries (so `.git/` is excluded) and not
/// following symlinks (the `black`/ripgrep posture), and applying
/// `--exclude`/`--extend-exclude`. An explicit path bypasses these discovery
/// filters — a hidden or symlinked file passed by name is formatted (the escape
/// hatch, the Unix posture).
///
/// The include/exclude globs are built into a standalone [`ignore::overrides::Override`]
/// matcher applied AFTER the walk — never plugged into the walker — so a `*.lp`
/// whitelist cannot un-ignore a gitignored file; `.gitignore` stays authoritative.
///
/// Only the IN-TREE `.gitignore` (at or below `dir`) is honored — parent, global
/// (`~/.config/git/ignore`), `.ignore`, and `.git/info/exclude` rules are disabled,
/// so pointing at a directory never silently skips everything because of an
/// ancestor's or the machine's ignore rules. `require_git(false)` keeps the in-tree
/// `.gitignore` effective even outside a git repository.
///
/// A per-entry walk failure (e.g. an unreadable subdirectory) is collected into
/// `found.errors` and skipped, NOT propagated — one bad entry must not discard the
/// rest of the tree.
fn walk_dir(dir: &Path, cli: &Cli, found: &mut Discovered) -> Result<()> {
    let includes: Vec<String> = if cli.include.is_empty() {
        vec!["*.lp".to_owned()]
    } else {
        cli.include.clone()
    };
    let mut ob = OverrideBuilder::new(dir);
    for inc in &includes {
        ob.add(inc)
            .with_context(|| format!("bad --include glob `{inc}`"))?;
    }
    for exc in cli.exclude.iter().chain(&cli.extend_exclude) {
        ob.add(&format!("!{exc}"))
            .with_context(|| format!("bad --exclude glob `{exc}`"))?;
    }
    let selector = ob.build().context("building --include/--exclude globs")?;

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in WalkBuilder::new(dir)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .require_git(false)
        .build()
    {
        match entry {
            Ok(e) => {
                if e.file_type().is_some_and(|ft| ft.is_file())
                    && selector.matched(e.path(), false).is_whitelist()
                {
                    files.push(e.into_path());
                }
            }
            // an unreadable entry is reported and skipped, never fatal.
            Err(e) => found.errors.push(e.to_string()),
        }
    }
    files.sort(); // deterministic order for reproducible output and tests
    found.targets.extend(files.into_iter().map(Target::File));
    Ok(())
}
