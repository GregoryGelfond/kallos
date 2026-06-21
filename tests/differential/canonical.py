"""Differential-test oracle.

Read an ASP/clingo program on stdin; emit clingo's own canonical rendering of its
AST — one statement per line, in source order. The Rust differential test compares
this for `s` against `format(s)`: equal sequences ⇒ clingo parses both identically
⇒ `format` preserved the code-token stream. (clingo drops comments, so the comment
conjunct is tree-sitter's separate job.)

This is the AUTHORITY against which the tree-sitter `≈` proxy is validated. It is
TEST-ONLY: invoked solely by the feature-gated Rust differential test via pixi, never
compiled into or shipped with the Rust crates. It sits behind that test's `canonical()`
seam, so it is replaceable by a native Rust clingo oracle with no change to the test
bodies.

SECURITY: clingo's `parse_string` is NOT stdin-isolated. It RESOLVES `#include`
directives by OPENING the named files from disk (relative to the process cwd, which the
Rust caller sets to the repo root), so a program fed here can make clingo read files off
the filesystem — the 6 corpus `#include` fixtures demonstrate this (clingo rejects them
only because their absent targets fail to open). This oracle is therefore safe ONLY for
TRUSTED inputs: the committed `tests/corpus` and the user's own `KALLOS_EXTRA_CORPUS`.
Do NOT feed an untrusted program on the assumption that stdin is a sandbox. (The helper
itself stays read-only: it parses and prints — no `eval` / `exec` / file write.)

Exit 0 + canonical lines on success; exit 2 + `PARSE-ERROR: …` on a clingo parse
failure (the caller triages: skip that program, never fail the suite). The
implicit `#program base.` clingo injects appears identically for `s` and `format(s)`,
so it cancels in the comparison and is left in rather than special-cased.
"""

import sys

from clingo.ast import parse_string


def canonical(program: str) -> list[str]:
    """Clingo's canonical rendering of each AST statement, in source order."""
    out: list[str] = []
    parse_string(program, lambda stmt: out.append(str(stmt)))
    return out


def main() -> int:
    try:
        for line in canonical(sys.stdin.read()):
            print(line)
    except RuntimeError as exc:  # clingo raises RuntimeError on a parse error
        print(f"PARSE-ERROR: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
