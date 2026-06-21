//! The command-line argument model (clap derive) and the validated views the
//! runner consumes. Parsing/usage errors — including a non-positive
//! `--line-width` — exit `2` via clap before `run` is reached; `run`
//! sees only a valid [`Cli`].

use std::path::PathBuf;

use clap::Parser;
use kallos::Style;

/// `kallos` — a pure-layout source formatter for ASP/clingo (`.lp`).
///
/// With no `PATHS` (or a `-`) it reads stdin and writes the formatted result
/// to stdout — the editor / Zed external-formatter contract. With `PATHS` it
/// formats files in place; directories are searched for `.lp` files.
#[derive(Debug, Parser)]
#[command(name = "kallos", version, about)]
pub struct Cli {
    /// Files or directories to format. A `-`, or no paths with piped stdin,
    /// reads from stdin and writes to stdout.
    #[arg(value_name = "PATHS")]
    pub paths: Vec<PathBuf>,

    /// Maximum line width in columns (must be at least 1).
    #[arg(long, value_name = "N", default_value_t = 100, value_parser = parse_line_width)]
    pub line_width: usize,

    /// Do not write files back; exit non-zero if any file would change (CI gate).
    #[arg(long)]
    pub check: bool,

    /// Do not write files back; print a unified diff of each change to stdout.
    #[arg(long)]
    pub diff: bool,

    /// Run the `verify` self-check on each file and refuse to write any whose
    /// result is not equivalent to its input (defense in depth).
    #[arg(long)]
    pub safe: bool,

    /// Glob of files to format when searching a directory (default `*.lp`).
    /// Repeatable.
    #[arg(long, value_name = "GLOB")]
    pub include: Vec<String>,

    /// Glob of files to skip when searching a directory. Repeatable.
    #[arg(long, value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Additional globs to skip (added to `--exclude`). Repeatable.
    #[arg(long, value_name = "GLOB")]
    pub extend_exclude: Vec<String>,
}

/// clap value-parser for `--line-width`: a positive integer. A non-positive or
/// non-numeric value is a usage error → clap exits `2`.
fn parse_line_width(s: &str) -> Result<usize, String> {
    let n: usize = s
        .parse()
        .map_err(|_| format!("`{s}` is not a non-negative integer"))?;
    if n == 0 {
        return Err("must be at least 1".to_owned());
    }
    Ok(n)
}

impl Cli {
    /// The layout [`Style`] carrying the validated `--line-width`.
    #[must_use]
    pub fn style(&self) -> Style {
        Style::default().with_line_width(self.line_width)
    }
}
