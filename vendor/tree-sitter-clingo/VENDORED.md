# Vendored: potassco/tree-sitter-clingo

- **Version:** 1.0.3
- **Revision:** `b701815f985ca72befd13774e5d602ae80375e41`
- **Source:** https://github.com/potassco/tree-sitter-clingo
- **License:** MIT (see `LICENSE` in this directory)
- **Vendored:** 2026-06-15, for clingo-fmt (design spec §14 — the grammar of record).

This is clingo-fmt's **grammar of record**: every grammar-grounded surface-syntax
claim in the spec and the engineering-session contributions is auditable against
the files here.

Do **not** hand-edit `src/parser.c`, `src/scanner.c`, `src/grammar.json`, or
`src/node-types.json` — they are generated artifacts. `grammar.js` is the source
the others are generated from; if the grammar ever needs regeneration, do it
upstream and re-vendor at a new pinned revision, updating this file.
