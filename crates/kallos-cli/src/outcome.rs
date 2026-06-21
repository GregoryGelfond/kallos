//! The per-file action taxonomy and the process-wide exit-code reduction.
//!
//! Exit codes mirror `black`: `0` success, `1` a no-write mode found a needed
//! change, `123` a file could not be safely processed. A usage error (`2`) is
//! emitted by clap before the runner is reached and never flows through here.
//! The process exit code is the **max severity** over all files.

/// The severity tiers, ordered `Ok < Changed < Failed`. The derived `Ord` ranks
/// by declaration order — the variant order IS the severity order; do not
/// reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Formatted, written, diffed, or already well-formatted.
    Ok,
    /// A no-write mode (`--check`) found a file that would change.
    Changed,
    /// A file could not be safely processed: a syntax error (`has_error`), a
    /// `--safe` mismatch, a non-UTF-8 body, or an I/O failure.
    Failed,
}

impl Severity {
    /// The process exit code for this severity (`black` parity).
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            Severity::Ok => 0,
            Severity::Changed => 1,
            Severity::Failed => 123,
        }
    }
}

/// What happened to one file or stream — drives both the stderr summary tally
/// and the severity reduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Default mode: the file changed and was written back.
    Reformatted,
    /// The file was already formatted; nothing to do.
    Unchanged,
    /// `--check`: the file would change (not written).
    WouldReformat,
    /// `--diff`: a diff was emitted (informational).
    Diffed,
    /// The file was skipped — see [`Severity::Failed`].
    Skipped,
}

impl Action {
    /// The severity this action contributes to the process exit code.
    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            Action::Reformatted | Action::Unchanged | Action::Diffed => Severity::Ok,
            Action::WouldReformat => Severity::Changed,
            Action::Skipped => Severity::Failed,
        }
    }
}

/// The process-wide exit severity: the max over every file's action.
#[must_use]
pub fn reduce<I: IntoIterator<Item = Action>>(actions: I) -> Severity {
    actions
        .into_iter()
        .map(Action::severity)
        .max()
        .unwrap_or(Severity::Ok)
}

#[cfg(test)]
mod tests {
    use super::{reduce, Action, Severity};

    #[test]
    fn severity_orders_ok_lt_changed_lt_failed() {
        assert!(Severity::Ok < Severity::Changed);
        assert!(Severity::Changed < Severity::Failed);
    }

    #[test]
    fn exit_codes_mirror_black() {
        assert_eq!(Severity::Ok.exit_code(), 0);
        assert_eq!(Severity::Changed.exit_code(), 1);
        assert_eq!(Severity::Failed.exit_code(), 123);
    }

    #[test]
    fn reduce_takes_the_max_severity() {
        assert_eq!(reduce([]), Severity::Ok);
        assert_eq!(
            reduce([Action::Unchanged, Action::Reformatted]),
            Severity::Ok
        );
        assert_eq!(
            reduce([Action::Unchanged, Action::WouldReformat]),
            Severity::Changed
        );
        assert_eq!(
            reduce([Action::WouldReformat, Action::Skipped]),
            Severity::Failed
        );
    }
}
