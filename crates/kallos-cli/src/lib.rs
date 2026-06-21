//! The `kallos` command-line interface over the pure `kallos` library.
//! Library-first: the entire CLI is [`run`] plus its helpers, exercised
//! in-process via an injected [`Io`]; `main.rs` is a thin shim.

mod cli;
mod discover;
mod outcome;
mod process;
mod run;

pub use cli::Cli;
pub use outcome::Severity;
pub use run::{run, Io};
