//! Acceptance tests for the CLI's argument model and the stdin→stdout
//! contract, exercised in-process through the public `run` with injected
//! streams — no subprocess, no real stdin.

use std::fs;
use std::path::Path;

use clap::Parser;
use kallos_cli::{run, Cli, Io, Severity};
use tempfile::tempdir;

/// Parse args the way `main` does (but without exiting the process).
fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("kallos").chain(args.iter().copied()))
}

/// Run on a stdin body, returning (severity, stdout, stderr).
fn run_stdin(args: &[&str], stdin: &str) -> (Severity, String, String) {
    let cli = parse(args).expect("args parse");
    let mut input = stdin.as_bytes();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let sev = {
        let mut io = Io {
            input: &mut input,
            out: &mut out,
            err: &mut err,
        };
        run(&cli, &mut io)
    };
    (
        sev,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn line_width_zero_is_a_usage_error() {
    let err = parse(&["--line-width", "0", "-"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn line_width_negative_is_a_usage_error() {
    // Any clap rejection path (missing value / unknown flag / value validation)
    // is exit 2 — the required property is "non-positive → usage error".
    assert!(parse(&["--line-width", "-3", "-"]).is_err());
}

#[test]
fn stdin_formats_to_stdout() {
    let (sev, out, _err) = run_stdin(&[], "a:-b.\n");
    assert_eq!(out, "a :- b.\n");
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn stdin_dash_path_also_formats() {
    let (sev, out, _err) = run_stdin(&["-"], "a:-b.\n");
    assert_eq!(out, "a :- b.\n");
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn stdin_honors_line_width() {
    let src = "a :- bbbbbbbbbb, cccccccccc, dddddddddd, eeeeeeeeee, ffffffffff.\n";
    let (_s, narrow, _e) = run_stdin(&["--line-width", "8"], src);
    let (_s, wide, _e) = run_stdin(&["--line-width", "100"], src);
    assert_eq!(wide.lines().count(), 1, "fits at 100:\n{wide}");
    assert!(narrow.lines().count() > 1, "wraps at 8:\n{narrow}");
}

#[test]
fn empty_stdin_is_ok_and_empty() {
    let (sev, out, _err) = run_stdin(&[], "");
    assert_eq!(out, "");
    assert_eq!(sev, Severity::Ok);
}

// --- explicit files, in-place write, has_error skip, severity reduction ---

/// Run on file targets with empty stdin, returning (severity, stdout, stderr).
fn run_files_io(args: &[&str], paths: &[&Path]) -> (Severity, String, String) {
    let mut cmdline: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    cmdline.extend(paths.iter().map(|p| p.display().to_string()));
    let cli = Cli::try_parse_from(std::iter::once("kallos".to_owned()).chain(cmdline))
        .expect("args parse");
    let mut input: &[u8] = b"";
    let mut out = Vec::new();
    let mut err = Vec::new();
    let sev = {
        let mut io = Io {
            input: &mut input,
            out: &mut out,
            err: &mut err,
        };
        run(&cli, &mut io)
    };
    (
        sev,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

/// Run on file targets, returning (severity, stderr) — for cases that ignore stdout.
fn run_files(args: &[&str], paths: &[&Path]) -> (Severity, String) {
    let (sev, _out, err) = run_files_io(args, paths);
    (sev, err)
}

#[test]
fn formats_a_file_in_place() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    let (sev, _err) = run_files(&[], &[&f]);
    assert_eq!(fs::read_to_string(&f).unwrap(), "a :- b.\n");
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn already_formatted_file_is_unchanged_and_ok() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a :- b.\n").unwrap();
    let (sev, _err) = run_files(&[], &[&f]);
    assert_eq!(fs::read_to_string(&f).unwrap(), "a :- b.\n");
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn syntax_error_file_is_skipped_with_exit_123() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("bad.lp");
    fs::write(&f, "a :- :- .\n").unwrap();
    let before = fs::read_to_string(&f).unwrap();
    let (sev, err) = run_files(&[], &[&f]);
    assert_eq!(fs::read_to_string(&f).unwrap(), before, "untouched");
    assert_eq!(sev, Severity::Failed);
    assert_eq!(sev.exit_code(), 123);
    assert!(err.contains("skipped"), "stderr: {err}");
}

#[test]
fn mixed_good_and_bad_takes_max_severity() {
    let dir = tempdir().unwrap();
    let good = dir.path().join("good.lp");
    let bad = dir.path().join("bad.lp");
    fs::write(&good, "a:-b.\n").unwrap();
    fs::write(&bad, "p(\n").unwrap();
    let (sev, _err) = run_files(&[], &[&good, &bad]);
    assert_eq!(
        fs::read_to_string(&good).unwrap(),
        "a :- b.\n",
        "good still formatted"
    );
    assert_eq!(sev, Severity::Failed, "the bad file dominates");
}

#[test]
fn nonexistent_path_fails() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("nope.lp");
    let (sev, err) = run_files(&[], &[&missing]);
    assert_eq!(sev, Severity::Failed);
    assert!(err.contains("error"), "stderr: {err}");
}

#[test]
fn non_utf8_file_is_skipped_not_panicked() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("binary.lp");
    fs::write(&f, [0xff, 0xfe, 0x00, 0x01]).unwrap();
    let (sev, _err) = run_files(&[], &[&f]);
    assert_eq!(sev, Severity::Failed);
}

// --- --check (gate, exit 1) and --diff (informational, exit 0) ---

#[test]
fn check_on_would_change_exits_1_without_writing() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    let (sev, err) = run_files(&["--check"], &[&f]);
    assert_eq!(fs::read_to_string(&f).unwrap(), "a:-b.\n", "NOT written");
    assert_eq!(sev, Severity::Changed);
    assert_eq!(sev.exit_code(), 1);
    assert!(err.contains("would reformat"), "stderr: {err}");
}

#[test]
fn check_on_clean_file_exits_0() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a :- b.\n").unwrap();
    let (sev, _err) = run_files(&["--check"], &[&f]);
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn diff_prints_a_unified_diff_to_stdout_exit_0() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    let (sev, out, _err) = run_files_io(&["--diff"], &[&f]);
    assert_eq!(fs::read_to_string(&f).unwrap(), "a:-b.\n", "NOT written");
    assert_eq!(sev, Severity::Ok, "--diff is informational");
    assert!(out.contains("-a:-b."), "removed old line:\n{out}");
    assert!(out.contains("+a :- b."), "added new line:\n{out}");
}

#[test]
fn check_and_diff_together_prints_diff_and_exits_1() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    let (sev, out, _err) = run_files_io(&["--check", "--diff"], &[&f]);
    assert_eq!(sev, Severity::Changed, "--check governs exit");
    assert!(out.contains("+a :- b."), "diff still printed:\n{out}");
}

#[test]
fn stdin_check_exits_1_on_change_no_stdout() {
    let (sev, out, _err) = run_stdin(&["--check"], "a:-b.\n");
    assert_eq!(sev, Severity::Changed);
    assert_eq!(out, "", "--check writes nothing to stdout");
}

// --- directory discovery (ignore): .lp auto-discovery, gitignore, globs ---

/// Write `body` to `dir/rel`, creating parent directories. Returns the path.
fn write_file(dir: &Path, rel: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&p, body).unwrap();
    p
}

#[test]
fn directory_discovers_only_lp_files() {
    let dir = tempdir().unwrap();
    let lp = write_file(dir.path(), "a.lp", "a:-b.\n");
    let txt = write_file(dir.path(), "notes.txt", "a:-b.\n");
    let (sev, _err) = run_files(&[], &[dir.path()]);
    assert_eq!(
        fs::read_to_string(&lp).unwrap(),
        "a :- b.\n",
        "lp formatted"
    );
    assert_eq!(
        fs::read_to_string(&txt).unwrap(),
        "a:-b.\n",
        "txt untouched"
    );
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn explicit_non_lp_file_is_formatted_unix_posture() {
    let dir = tempdir().unwrap();
    let txt = write_file(dir.path(), "prog.txt", "a:-b.\n");
    let (_sev, _err) = run_files(&[], &[&txt]); // explicit FILE, not a dir
    assert_eq!(fs::read_to_string(&txt).unwrap(), "a :- b.\n");
}

#[test]
fn gitignored_lp_is_respected() {
    let dir = tempdir().unwrap();
    write_file(dir.path(), ".gitignore", "ignored.lp\n");
    let kept = write_file(dir.path(), "kept.lp", "a:-b.\n");
    let ignored = write_file(dir.path(), "ignored.lp", "a:-b.\n");
    let (_sev, _err) = run_files(&[], &[dir.path()]);
    assert_eq!(
        fs::read_to_string(&kept).unwrap(),
        "a :- b.\n",
        "kept formatted"
    );
    assert_eq!(
        fs::read_to_string(&ignored).unwrap(),
        "a:-b.\n",
        "gitignored skipped"
    );
}

#[test]
fn dot_git_directory_is_excluded() {
    let dir = tempdir().unwrap();
    let inside_git = write_file(dir.path(), ".git/hooks/x.lp", "a:-b.\n");
    let normal = write_file(dir.path(), "n.lp", "a:-b.\n");
    let (_sev, _err) = run_files(&[], &[dir.path()]);
    assert_eq!(
        fs::read_to_string(&inside_git).unwrap(),
        "a:-b.\n",
        ".git/ skipped"
    );
    assert_eq!(fs::read_to_string(&normal).unwrap(), "a :- b.\n");
}

#[test]
fn exclude_glob_skips_matching_lp() {
    let dir = tempdir().unwrap();
    let kept = write_file(dir.path(), "keep.lp", "a:-b.\n");
    let skipped = write_file(dir.path(), "gen.lp", "a:-b.\n");
    let (_sev, _err) = run_files(&["--exclude", "gen.lp"], &[dir.path()]);
    assert_eq!(fs::read_to_string(&kept).unwrap(), "a :- b.\n");
    assert_eq!(fs::read_to_string(&skipped).unwrap(), "a:-b.\n", "excluded");
}

#[test]
fn include_glob_overrides_the_lp_default() {
    let dir = tempdir().unwrap();
    let asp = write_file(dir.path(), "p.asp", "a:-b.\n");
    let lp = write_file(dir.path(), "q.lp", "a:-b.\n");
    let (_sev, _err) = run_files(&["--include", "*.asp"], &[dir.path()]);
    assert_eq!(
        fs::read_to_string(&asp).unwrap(),
        "a :- b.\n",
        "asp now included"
    );
    assert_eq!(
        fs::read_to_string(&lp).unwrap(),
        "a:-b.\n",
        "lp no longer the default"
    );
}

// --- --safe (verify gate, defense in depth) ---
//
// A --safe MISMATCH on error-free input is unreachable by construction
// (`≈` holds for error-free input, so `verify` is Ok), so there is no honest test
// that forces a mismatch on clean input without a fabricated formatter bug. The
// `SkipReason::Unsafe` → Display path is covered by the library's Mismatch
// Display tests (`equiv.rs`). These two pin the reachable --safe behavior.

#[test]
fn safe_passes_error_free_input_through() {
    // On error-free input, verify is Ok by construction → --safe writes.
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    let (sev, _err) = run_files(&["--safe"], &[&f]);
    assert_eq!(fs::read_to_string(&f).unwrap(), "a :- b.\n");
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn safe_does_not_resurrect_a_syntax_error_file() {
    // has_error skips error files BEFORE --safe; --safe does not change that.
    let dir = tempdir().unwrap();
    let f = dir.path().join("bad.lp");
    fs::write(&f, "a :- :- .\n").unwrap();
    let before = fs::read_to_string(&f).unwrap();
    let (sev, err) = run_files(&["--safe"], &[&f]);
    assert_eq!(fs::read_to_string(&f).unwrap(), before, "untouched");
    assert_eq!(sev, Severity::Failed);
    assert!(err.contains("skipped"), "stderr: {err}");
}

// --- discovery robustness ---

#[test]
fn parent_gitignore_is_not_honored() {
    // only the in-tree .gitignore (at/below the walk root) is respected, not a
    // PARENT's (nor the global) — else `--check` silently passes examining nothing.
    let dir = tempdir().unwrap();
    let sub = dir.path().join("proj/sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(dir.path().join("proj/.gitignore"), "*.lp\n").unwrap();
    let f = sub.join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    let (sev, _err) = run_files(&[], &[&sub]);
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        "a :- b.\n",
        "a parent .gitignore must NOT skip the file"
    );
    assert_eq!(sev, Severity::Ok);
}

#[cfg(unix)]
#[test]
fn unreadable_entry_does_not_abort_the_walk() {
    use std::os::unix::fs::PermissionsExt;
    // one unreadable subdir is reported + skipped; the rest still format.
    let dir = tempdir().unwrap();
    let good = dir.path().join("good");
    let bad = dir.path().join("bad");
    fs::create_dir_all(&good).unwrap();
    fs::create_dir_all(&bad).unwrap();
    let g = good.join("g.lp");
    fs::write(&g, "a:-b.\n").unwrap();
    fs::write(bad.join("b.lp"), "a:-b.\n").unwrap();
    fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (sev, err) = run_files(&[], &[dir.path()]);
    fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap(); // for cleanup
    assert_eq!(
        fs::read_to_string(&g).unwrap(),
        "a :- b.\n",
        "the good file still formats despite the unreadable sibling"
    );
    assert_eq!(
        sev,
        Severity::Failed,
        "the unreadable entry makes the run Failed"
    );
    assert!(
        err.contains("error"),
        "the unreadable entry is reported: {err}"
    );
}

#[cfg(unix)]
#[test]
fn nonregular_explicit_path_reports_accurately() {
    // /dev/null EXISTS (a char device) — the message must not claim otherwise.
    let (sev, err) = run_files(&[], &[Path::new("/dev/null")]);
    assert_eq!(sev, Severity::Failed);
    assert!(
        err.contains("not a regular file") && !err.contains("does not exist"),
        "accurate message for a non-regular path: {err}"
    );
}

// --- write safety (atomic write, stdin echo) ---

#[test]
fn stdin_skip_echoes_input_verbatim() {
    // a stdin syntax error echoes the input unchanged (so an editor that
    // replaces the buffer from stdout cannot blank the file), still exit 123.
    let (sev, out, err) = run_stdin(&["-"], "a :- :- .\n");
    assert_eq!(out, "a :- :- .\n", "input echoed verbatim, not blanked");
    assert_eq!(sev, Severity::Failed);
    assert!(err.contains("skipped"), "stderr: {err}");
}

#[cfg(unix)]
#[test]
fn in_place_write_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;
    // An in-place write preserves the file's mode (truncate-and-write keeps the
    // existing permissions), so reformatting never silently chmods.
    let dir = tempdir().unwrap();
    let f = dir.path().join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    fs::set_permissions(&f, std::fs::Permissions::from_mode(0o640)).unwrap();
    let (sev, _err) = run_files(&[], &[&f]);
    assert_eq!(sev, Severity::Ok);
    assert_eq!(fs::read_to_string(&f).unwrap(), "a :- b.\n");
    let mode = fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640, "in-place write preserves the original mode");
}

#[cfg(unix)]
#[test]
fn writable_file_in_readonly_dir_is_formatted() {
    use std::os::unix::fs::PermissionsExt;
    // Aligned with rustfmt's direct write: a writable file formats even when its
    // parent directory is read-only (an atomic temp+rename would fail here, since
    // the temp can't be created in the dir).
    let dir = tempdir().unwrap();
    let sub = dir.path().join("ro");
    fs::create_dir(&sub).unwrap();
    let f = sub.join("a.lp");
    fs::write(&f, "a:-b.\n").unwrap();
    fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    let (sev, _err) = run_files(&[], &[&f]);
    fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap(); // for cleanup
    assert_eq!(sev, Severity::Ok);
    assert_eq!(fs::read_to_string(&f).unwrap(), "a :- b.\n");
}

#[cfg(unix)]
#[test]
fn in_place_write_through_symlink_preserves_the_link() {
    use std::os::unix::fs::symlink;
    // A direct write (rustfmt-aligned) follows the symlink and rewrites the
    // TARGET, preserving the link — not replacing it with a regular file.
    let dir = tempdir().unwrap();
    let real = dir.path().join("real.lp");
    let link = dir.path().join("link.lp");
    fs::write(&real, "a:-b.\n").unwrap();
    symlink(&real, &link).unwrap();
    let (sev, _err) = run_files(&[], &[&link]);
    assert_eq!(sev, Severity::Ok);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "link.lp must remain a symlink"
    );
    assert_eq!(
        fs::read_to_string(&real).unwrap(),
        "a :- b.\n",
        "the target was formatted through the link"
    );
}

// --- BOM preservation, hidden-skip escape hatch ---

#[test]
fn leading_bom_is_preserved() {
    // Encoding is a CLI concern: a leading UTF-8 BOM is preserved across
    // formatting, not silently stripped as leading whitespace.
    let dir = tempdir().unwrap();
    let f = dir.path().join("bom.lp");
    fs::write(&f, "\u{feff}a:-b.\n").unwrap();
    let (sev, _err) = run_files(&[], &[&f]);
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        "\u{feff}a :- b.\n",
        "BOM kept, body formatted"
    );
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn bom_with_formatted_body_is_unchanged_under_check() {
    // A BOM file whose body is already formatted is Unchanged — the BOM alone must
    // not flag it as would-reformat.
    let dir = tempdir().unwrap();
    let f = dir.path().join("bom.lp");
    fs::write(&f, "\u{feff}a :- b.\n").unwrap();
    let (sev, _err) = run_files(&["--check"], &[&f]);
    assert_eq!(
        sev,
        Severity::Ok,
        "BOM + already-formatted body = unchanged"
    );
}

#[test]
fn stdin_preserves_leading_bom() {
    let (sev, out, _err) = run_stdin(&[], "\u{feff}a:-b.\n");
    assert_eq!(out, "\u{feff}a :- b.\n", "stdin BOM preserved");
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn explicit_hidden_file_is_formatted() {
    // discovery skips hidden files (black/ripgrep standard), but an EXPLICIT
    // path is the escape hatch — it formats regardless of the leading dot.
    let dir = tempdir().unwrap();
    let hidden = dir.path().join(".secret.lp");
    fs::write(&hidden, "a:-b.\n").unwrap();
    let (sev, _err) = run_files(&[], &[&hidden]);
    assert_eq!(fs::read_to_string(&hidden).unwrap(), "a :- b.\n");
    assert_eq!(sev, Severity::Ok);
}

#[test]
fn directory_walk_skips_hidden_lp() {
    // a hidden `.lp` inside a walked dir is skipped (matches black/ripgrep);
    // the visible sibling formats.
    let dir = tempdir().unwrap();
    let hidden = dir.path().join(".secret.lp");
    let visible = dir.path().join("v.lp");
    fs::write(&hidden, "a:-b.\n").unwrap();
    fs::write(&visible, "a:-b.\n").unwrap();
    let (_sev, _err) = run_files(&[], &[dir.path()]);
    assert_eq!(
        fs::read_to_string(&hidden).unwrap(),
        "a:-b.\n",
        "hidden skipped in walk"
    );
    assert_eq!(
        fs::read_to_string(&visible).unwrap(),
        "a :- b.\n",
        "visible formatted"
    );
}
