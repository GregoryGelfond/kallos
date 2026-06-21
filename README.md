# kallos

A pure-layout source formatter for Answer Set Programming (clingo `.lp`).

**Answer Set Programming (ASP)** is a declarative approach to knowledge representation and
combinatorial search: you write a logic program — facts, rules, and constraints — and a solver
computes its *answer sets*, the stable models that are its solutions. [clingo](https://potassco.org)
is the dominant system. kallos reformats clingo source the way `rustfmt` reformats Rust or `gofmt`
reformats Go: it re-spaces and re-indents a program to one consistent house style, and changes
nothing else.

**Pure layout.** kallos touches whitespace only. It never adds, removes, reorders, or rewrites a
token, and it never "improves" your encoding. Three properties hold on every run:

- **Meaning-preserving.** The formatted program lexes to the *same token stream* as the input —
  every atom, operator, and number is exactly where it was. kallos proves this to itself before it
  writes (see [How it works](#how-it-works)); `--safe` turns the check into a hard gate.
- **Comments verbatim.** Line (`% …`) and block (`%* … *%`) comments are carried through unchanged
  and kept with the statement they belong to, so the layout pass never loses or rewrites a note.
- **Idempotent.** Formatting an already-formatted file is a no-op. `format(format(x)) == format(x)`.

What you get back is your program, meaning for meaning and comment for comment, in a uniform shape.

**Who it's for.** If you write and maintain clingo encodings and want them to read consistently —
across a file, a project, or a team — kallos is the autoformatter. Point your editor at it for
format-on-save, or run it in CI with `--check`.

## The name

*kallos* (Greek κάλλος) means **beauty** — beauty of form and proportion, the quality the Greeks
heard in a well-ordered arrangement. That is exactly a formatter's one job: to give a program a
clear, orderly form without changing what it says. The name also nods to Dijkstra's motto for the
craft, *"beauty is our business"* — legible structure is not decoration, it is the substance of good
engineering. kallos belongs to a small family of Greek-named ASP tools, alongside *aspis* (a host
library) and [elenctic](https://github.com/GregoryGelfond/elenctic) (a declarative test harness);
the Greek is the through-line.

It is distinct from Potassco's `clingofmt`. kallos reuses that project's MIT-licensed test programs
as formatter *inputs* (re-formatted in kallos's own house style and validated against clingo, never
compared to `clingofmt`'s output); the attribution is in
[`crates/kallos/tests/corpus/clingofmt/NOTICE`](crates/kallos/tests/corpus/clingofmt/NOTICE).

## A first example

Give kallos a program with inconsistent spacing, run-together statements, and comments:

```clingo
%reachability
edge(1,2).edge(2,3). edge(3,4).
reach(X,Y):-edge(X,Y).
reach(X,Z):-reach(X,Y),edge(Y,Z). %transitive
#show reach/2.
```

```console
$ kallos reach.lp        # formats the file in place
```

```clingo
%reachability
edge(1, 2).
edge(2, 3).
edge(3, 4).
reach(X, Y) :- edge(X, Y).
reach(X, Z) :- reach(X, Y), edge(Y, Z). %transitive
#show reach/2.
```

One statement per line, a space after every comma, the `:-` neck spaced, and both comments kept
exactly where they were (kallos does not even insert a space after `%` — the comment text is yours).

## Wrapping

A statement that fits stays on one line. When a rule is wider than the target (default **100**
columns), kallos breaks the body one literal per line and indents the continuation, so the structure
stays legible instead of running off the screen:

```console
$ kallos --line-width 40 game.lp
```

```clingo
win(X); lose(X) :-
    player(X),
    move(X, Y),
    not win(Y),
    reachable(X).
```

The same rule under the default width stays on a single line. Aggregates, conditional literals,
pools, and disjunctions break the same way, descending one level at a time.

## Choice rules and aggregates

Cardinality, choice, and aggregate constructs are everywhere in ASP, and kallos gives them a
consistent house style: a bound relation is spaced, a bare choice `{ … }` is spaced, an aggregate
keyword hugs its brace (`#minimize{ … }`), and a conditional's `:` is spaced. A task-assignment
encoding (exactly one compatible agent per task, minimizing total cost) reads the same however it
was typed:

```clingo
% input
1{assigned_to(A,T):agent(A),compatible_with(A,T)}1:-task(T).
#minimize{C,A,T:assigned_to(A,T),cost_of(A,T,C)}.
```

```console
$ kallos assign.lp
```

```clingo
1 { assigned_to(A,T) : agent(A), compatible_with(A,T) } 1 :- task(T).
#minimize{ C, A, T : assigned_to(A,T), cost_of(A,T,C) }.
```

Past the target width, each construct breaks the same way: its elements hang inside the braces, with
the choice rule's condition dropping a further level (here at `--line-width 50`):

```clingo
1 {
    assigned_to(A,T) :
        agent(A), compatible_with(A,T)
} 1 :-
    task(T).
#minimize{
    C, A, T : assigned_to(A,T), cost_of(A,T,C)
}.
```

## Using kallos

### Command line

kallos is a Unix filter and an in-place formatter in one binary.

```console
$ kallos                       # read stdin, write formatted stdout (editor / format-on-save)
$ cat prog.lp | kallos         # same, explicitly
$ kallos a.lp b.lp             # format these files in place
$ kallos encodings/            # format every .lp file under a directory
```

Useful flags:

| flag | effect |
|---|---|
| `--line-width N` | target width in columns (default `100`, minimum `1`) |
| `--check` | don't write; exit non-zero if any file would change — the CI gate |
| `--diff` | don't write; print a unified diff of each change |
| `--safe` | run the meaning-preserving self-check and refuse to write any file whose result is not equivalent to its input |
| `--include GLOB` / `--exclude GLOB` | choose which files a directory search formats (default `*.lp`) |

With no paths (or a `-`) kallos reads stdin and writes stdout — the editor / external-formatter
contract (e.g. a Zed or VS Code "format on save" hook). With paths it rewrites files in place and
prints a one-line summary (`N reformatted, M unchanged, K skipped`) to stderr. In CI:

```console
$ kallos --check encodings/    # exits 1 if anything is unformatted
```

### As a library

The formatter is a library first; the CLI is a thin shell over it.

```rust
use kallos::{format, verify, Style};

let src = "a:-b,c.\n";

// Format to the house style; `with_line_width` sets the one layout knob.
assert_eq!(format(src, &Style::default()), "a :- b, c.\n");
assert_eq!(format(src, &Style::default().with_line_width(40)), "a :- b, c.\n");

// Defense in depth: confirm formatting `src` is layout-only — same tokens, same comments.
assert!(verify(src, &Style::default()).is_ok());
```

`format` is total: it returns a `String` for any input, including a syntactically invalid program
(an error-bearing file is re-spaced as best the parser allows and is never panicked on).
`has_error(src)` reports whether the input parses cleanly, and `verify(src, style)` formats `src`
and returns the precise `Mismatch` if (and only if) the reflow failed to preserve the program — the
fact `--safe` keys on.

## The house style

kallos has one configurable knob, the line width; everything else is fixed so that any two kallos
users produce the same layout. The rules, in brief:

- **Indentation** is 4 spaces; continuations nest one level per structural depth.
- **One statement per line**; a run-together `a.b.` becomes two lines.
- The rule neck `:-` and weak-constraint neck `:~` are spaced (`head :- body`); `,` and `;`
  separators get a trailing space; `|` disjunction is spaced both sides.
- Function and atom parentheses are tight (`p(X, Y)`), aggregate braces are spaced (`{ a; b }`), and
  an aggregate keyword hugs its brace (`#count{ … }`).
- **Blank lines** are normalized: a run of blank lines between statements collapses to at most one,
  and a single author blank line is preserved as a deliberate grouping signal.
- **Comments** keep their position relative to the code they annotate (leading, trailing, or
  detached by a blank line), and their text is emitted byte-for-byte.

See [How it works](#how-it-works) for why these are safe to apply mechanically.

## How it works

kallos is a small pipeline, and each stage has a single job:

1. **Parse.** A vendored [tree-sitter](https://tree-sitter.github.io) grammar for clingo turns the
   source into a concrete syntax tree that keeps *every* token — including the anonymous ones
   (`,` `;` `:-` `{`) a meaning-only AST would drop — and every comment.
2. **Lower to a `Doc`.** The tree is lowered to a Wadler/Lindig pretty-printing document (the
   `kallos-doc` crate, a small strict pretty-printer): a structure of groups, nests, and soft
   breaks that encodes *where* the layout may break and by how much it indents.
3. **Render** the document at the target width.
4. **Re-inject comments.** A separate pre-pass attaches each comment to an anchor node; the renderer
   weaves them back in at their anchors, on their own line or trailing, as the source had them.

Two correctness mechanisms make "pure layout" a guarantee rather than a hope:

- **A token-fusion oracle.** Removing a space can be dangerous — `a` next to `b` would re-lex as the
  single token `ab`. kallos centralizes the lexical theory of *when whitespace may be removed* into
  one total, independently-tested predicate, and tightens a seam only where that predicate certifies
  it safe. The default is always to keep the space.
- **An equivalence self-check (`≈`).** Before writing, kallos confirms the formatted text re-parses
  to the same token structure as the input. The `differential/` test goes further and cross-checks
  the tree-sitter proxy against clingo *itself* — clingo's own parse is the authority — so the
  guarantee rests on the real solver's lexer, not only on the grammar.

The shipped formatter links no solver and bundles no Python; the clingo cross-check is a
development-time test, enabled separately.

## Building and developing

kallos is a Rust workspace (MSRV **1.96**). Three crates: `kallos` (the formatter library),
`kallos-cli` (the `kallos` binary), and `kallos-doc` (the pretty-printing engine).

```console
$ cargo build --release        # binary at target/release/kallos
$ cargo test --workspace       # the full suite
$ cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
```

The repository ships [pixi](https://pixi.sh) for the optional clingo differential test, which pins
clingo and runs the cross-check:

```console
$ pixi run differential        # validates the layout against clingo's own parser
```

Point `KALLOS_EXTRA_CORPUS` (colon-separated directories) at your own `.lp` files to exercise the
idempotence and equivalence property tests over them.

## Status

**v0.1.0.** kallos formats the full clingo language — rules and constraints, the optimization and
aggregate constructs, conditional literals, pools and disjunction, theory atoms and `#theory`
definitions, and the directives — with the meaning-preserving, comment-verbatim, and idempotence
guarantees above enforced by a corpus and property-based test suite. It is a young project; please
report any program kallos reshapes incorrectly.

## License

MIT — see [LICENSE](LICENSE).
