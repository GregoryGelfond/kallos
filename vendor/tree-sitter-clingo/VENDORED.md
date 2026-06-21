# Vendored: potassco/tree-sitter-clingo

- **Version:** 1.0.3
- **Revision:** `b701815f985ca72befd13774e5d602ae80375e41`
- **Source:** https://github.com/potassco/tree-sitter-clingo
- **License:** MIT (see `LICENSE` in this directory)
- **Vendored:** 2026-06-15, for kallos.

This is kallos's **grammar of record**: every grammar-grounded surface-syntax
claim is auditable against the files here.

Do **not** hand-edit the generated artifacts (`src/parser.c`, `src/scanner.c`,
`src/grammar.json`, `src/node-types.json`). `grammar.js` is the source they are
generated from; after any change to it, regenerate with the pinned CLI:

```
npx -y -p tree-sitter-cli@0.25.10 tree-sitter generate
```

(0.25.10 is the exact version that produced the committed artifacts — regenerating the
*unmodified* grammar with it reproduces them byte-for-byte.) The preferred way to evolve
the grammar is **upstream**: contribute the change to potassco/tree-sitter-clingo and
re-vendor at a new pinned revision. The one current exception is the local patch below.

## Local modifications (downstream patch)

A minimal, documented divergence from the pinned upstream revision `b701815…`, to be
dropped once upstream incorporates the change and we re-vendor at the new revision.

- **`relation` rule — added the `==` and `<>` comparison operators.** Touches the
  `relation` token in `grammar.js`; `src/parser.c` and `src/grammar.json` were regenerated
  (`src/node-types.json` is unchanged — no new node kind, only the matched token set grows).
  clingo accepts both `==` (explicit equality, alongside `=`) and `<>` (angle not-equal,
  alongside `!=`); upstream `v1.0.3` omits both. The change is purely additive: it accepts
  inputs the pinned grammar rejected and alters no existing parse, so files using these
  operators format instead of being skipped as syntax errors.
  - **Upstream tracker:** [potassco/tree-sitter-clingo#58](https://github.com/potassco/tree-sitter-clingo/issues/58)
    (filed for `==`; upstream HEAD already carries `<>`, so re-vendoring at the resolved
    revision subsumes this patch).
