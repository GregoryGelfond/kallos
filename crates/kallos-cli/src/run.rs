//! The runner: expand targets, process each, tally a summary to stderr, and
//! reduce per-file actions to the process exit severity. I/O is injected
//! ([`Io`]) so the whole pipeline is testable in-process — `main` supplies the
//! real stdin/stdout/stderr.

use std::io::{Read, Write};

use crate::cli::Cli;
use crate::discover::{collect_targets, Target};
use crate::outcome::{reduce, Action, Severity};
use crate::process::process;

/// Injected standard streams. `main` passes the process handles; tests pass
/// in-memory buffers, so `run` needs no real stdin/stdout to exercise.
pub struct Io<'a> {
    pub input: &'a mut dyn Read,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

impl std::fmt::Debug for Io<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Io").finish_non_exhaustive()
    }
}

/// Run the formatter over `cli`'s targets, returning the process exit severity.
///
/// Total over its inputs: per-file I/O errors, syntax errors, and `--safe`
/// mismatches are reported to `io.err` and recorded as [`Severity::Failed`]
/// rather than aborting the run. A failure to even enumerate the targets
/// (e.g. a non-existent path) is reported and returns [`Severity::Failed`].
#[must_use]
pub fn run(cli: &Cli, io: &mut Io<'_>) -> Severity {
    let found = match collect_targets(cli) {
        Ok(found) => found,
        Err(e) => {
            let _ = writeln!(io.err, "error: {e:#}");
            return Severity::Failed;
        }
    };
    // Non-fatal walk errors (e.g. an unreadable subdirectory): report each
    // and floor the run to `Failed`, but still process every file we did find.
    for msg in &found.errors {
        let _ = writeln!(io.err, "error: {msg}");
    }
    let has_file = found.targets.iter().any(|t| matches!(t, Target::File(_)));
    let style = cli.style();
    let actions: Vec<Action> = found
        .targets
        .iter()
        .map(|t| process(t, cli, &style, io))
        .collect();
    summarize(cli, has_file, &actions, io);
    let mut severity = reduce(actions);
    if !found.errors.is_empty() {
        severity = severity.max(Severity::Failed);
    }
    severity
}

/// Emit a one-line stderr summary when at least one target was a file. A pure
/// stdin run is a filter — stay quiet except for per-file errors (the Zed case).
fn summarize(cli: &Cli, has_file: bool, actions: &[Action], io: &mut Io<'_>) {
    if !has_file {
        return;
    }
    let count = |a: Action| actions.iter().filter(|&&x| x == a).count();
    let unchanged = count(Action::Unchanged) + count(Action::Diffed);
    let skipped = count(Action::Skipped);
    let (verb, changed_n) = if cli.check {
        ("would be reformatted", count(Action::WouldReformat))
    } else {
        ("reformatted", count(Action::Reformatted))
    };
    let _ = writeln!(
        io.err,
        "{changed_n} {verb}, {unchanged} unchanged, {skipped} skipped"
    );
}
