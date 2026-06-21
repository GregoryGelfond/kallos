//! Thin binary shim over the testable [`kallos_cli::run`]: parse args (clap
//! exits `2` on a usage error), wire the real standard streams, and exit with
//! the reduced severity's code.

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;
use kallos_cli::{run, Cli, Io};

fn main() -> ExitCode {
    let cli = Cli::parse();

    // No paths AND an interactive stdin → nothing to read. Fail with a usage
    // error rather than hang waiting on the terminal.
    if cli.paths.is_empty() && io::stdin().is_terminal() {
        eprintln!("error: no input — provide PATHS or pipe source on stdin");
        return ExitCode::from(2);
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut input = stdin.lock();
    let mut out = stdout.lock();
    let mut err = stderr.lock();

    let severity = {
        let mut io = Io {
            input: &mut input,
            out: &mut out,
            err: &mut err,
        };
        run(&cli, &mut io)
    };
    let _ = out.flush();
    let _ = err.flush();
    ExitCode::from(severity.exit_code())
}
