//! Per-target processing: read → conservative `has_error` skip → optional
//! `--safe` `verify` gate → `format` → mode dispatch. Each target yields an
//! [`Action`] driving the summary and the exit-code reduction.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use kallos::{format, has_error, verify, Mismatch, Style};

use crate::cli::Cli;
use crate::discover::Target;
use crate::outcome::Action;
use crate::run::Io;

/// Format one target per the mode flags in `cli`.
#[must_use]
pub fn process(target: &Target, cli: &Cli, style: &Style, io: &mut Io<'_>) -> Action {
    match target {
        Target::Stdin => process_stdin(cli, style, io),
        Target::File(path) => match process_file(path, cli, style, io) {
            Ok(action) => action,
            Err(e) => {
                let _ = writeln!(io.err, "error: {}: {e:#}", path.display());
                Action::Skipped
            }
        },
    }
}

/// The format/skip decision for one source body, independent of its origin.
enum Decision {
    Formatted { text: String, changed: bool },
    Skip(SkipReason),
}

/// Why a target was not formatted.
enum SkipReason {
    SyntaxError,
    Unsafe(Mismatch),
}

/// Decide what to do with `src`: conservative `has_error` skip, then the
/// optional `--safe` `verify` gate, then `format`.
///
/// Encoding is a CLI concern: a leading UTF-8 BOM is split off, the body is
/// the unit checked and formatted, and the BOM is re-attached to the output —
/// preserved across formatting, never silently stripped by the layout pass.
fn decide(src: &str, cli: &Cli, style: &Style) -> Decision {
    let (bom, body) = src
        .strip_prefix('\u{feff}')
        .map_or(("", src), |rest| ("\u{feff}", rest));
    if has_error(body) {
        return Decision::Skip(SkipReason::SyntaxError);
    }
    if cli.safe {
        if let Err(m) = verify(body, style) {
            return Decision::Skip(SkipReason::Unsafe(m));
        }
    }
    let formatted = format(body, style);
    let text = if bom.is_empty() {
        formatted
    } else {
        format!("{bom}{formatted}")
    };
    let changed = text != src;
    Decision::Formatted { text, changed }
}

fn process_stdin(cli: &Cli, style: &Style, io: &mut Io<'_>) -> Action {
    let mut src = String::new();
    if let Err(e) = io.input.read_to_string(&mut src) {
        let _ = writeln!(io.err, "error: reading stdin: {e}");
        return Action::Skipped;
    }
    match decide(&src, cli, style) {
        Decision::Skip(reason) => {
            // Editor-buffer safety (Zed contract): echo the input verbatim so
            // an editor replacing the buffer from stdout cannot blank the file on a
            // transient syntax error mid-edit. The exit stays 123 via `Skipped`.
            let _ = io.out.write_all(src.as_bytes());
            report_skip(io, "<stdin>", &reason);
            Action::Skipped
        }
        Decision::Formatted { text, changed } => {
            // No-write modes compose: --diff prints, --check governs the exit.
            if cli.check || cli.diff {
                if changed && cli.diff {
                    let _ = render_diff(io.out, &src, &text, "<stdin>");
                }
                return no_write_action(changed, cli.check);
            }
            // Default: stdin → stdout (the editor contract) — emit either way.
            let _ = io.out.write_all(text.as_bytes());
            if changed {
                Action::Reformatted
            } else {
                Action::Unchanged
            }
        }
    }
}

/// The action for a no-write mode (`--check`/`--diff`), given whether the file
/// changed and whether `--check` is set.
fn no_write_action(changed: bool, check: bool) -> Action {
    match (changed, check) {
        (true, true) => Action::WouldReformat, // exit 1
        (true, false) => Action::Diffed,       // --diff only: informational
        (false, _) => Action::Unchanged,
    }
}

/// Report a skipped target to stderr.
fn report_skip(io: &mut Io<'_>, label: &str, reason: &SkipReason) {
    match reason {
        SkipReason::SyntaxError => {
            let _ = writeln!(io.err, "skipped {label}: syntax errors");
        }
        SkipReason::Unsafe(m) => {
            let _ = writeln!(io.err, "skipped {label}: --safe check failed: {m}");
        }
    }
}

/// Write a unified diff of `old`→`new` to `out` (`--diff`).
fn render_diff(out: &mut dyn Write, old: &str, new: &str, label: &str) -> std::io::Result<()> {
    let diff = similar::TextDiff::from_lines(old, new);
    let mut ud = diff.unified_diff();
    ud.context_radius(3).header(label, label);
    write!(out, "{ud}")
}

/// Format a single file per the mode flags. Reads (UTF-8; a non-UTF-8 body is
/// an I/O error → `Err` → `Skipped`), decides, then writes in place / checks /
/// diffs.
///
/// # Errors
/// Returns an error if the file cannot be read or written.
fn process_file(path: &Path, cli: &Cli, style: &Style, io: &mut Io<'_>) -> Result<Action> {
    let src = fs::read_to_string(path).with_context(|| format!("reading `{}`", path.display()))?;
    let label = path.display().to_string();
    match decide(&src, cli, style) {
        Decision::Skip(reason) => {
            report_skip(io, &label, &reason);
            Ok(Action::Skipped)
        }
        Decision::Formatted { text, changed } => {
            // No-write modes compose: --diff prints, --check governs the exit.
            if cli.check || cli.diff {
                if changed && cli.diff {
                    let _ = render_diff(io.out, &src, &text, &label);
                }
                if changed && cli.check {
                    let _ = writeln!(io.err, "would reformat {label}");
                }
                return Ok(no_write_action(changed, cli.check));
            }
            if changed {
                // Direct write, aligned with rustfmt's `FilesEmitter` (plain
                // `fs::write`): it follows a symlink to rewrite the target
                // (preserving the link) and works even when the parent directory
                // is read-only. The (VCS-mitigated) non-atomicity is the same
                // posture rustfmt takes on source files.
                fs::write(path, &text).with_context(|| format!("writing `{}`", path.display()))?;
                let _ = writeln!(io.err, "reformatted {label}");
                Ok(Action::Reformatted)
            } else {
                Ok(Action::Unchanged)
            }
        }
    }
}
