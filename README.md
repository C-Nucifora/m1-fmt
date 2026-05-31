# m1-fmt

An auto-formatter (pretty-printer) for the MoTeC M1 script language (`.m1scr`).
It reparses the formatted output to guarantee it never changes a script's
meaning. It is both a **library** (consumed by `m1-lsp` for
`textDocument/formatting`) and a **CLI**.

## Workspace layout

The M1 toolchain lives in **six separate repositories** coupled through Cargo
**path** dependencies. They are not published to crates.io, so this crate does
**not** build from a standalone clone — check out the whole set as siblings under
one parent directory:

```
<parent>/
├── tree-sitter-m1/   # grammar (root)
├── m1-core/          # parse / CST / diagnostics
├── m1-lint/          # linter
├── m1-fmt/           # this crate
├── m1-typecheck/     # type checker
└── m1-lsp/           # language server; depends on the four above
```

**`m1-fmt` depends on `../m1-core`** (`m1-core = { path = "../m1-core" }`, which
in turn needs `../tree-sitter-m1`), so both must be checked out alongside it. It is
in turn depended on by `m1-lsp`. (The `m1-example` example project, used by the corpus
test, is an optional further sibling.)

Because the repos are independent on GitHub, this coupling is **not visible
there**: each repo's CI and PRs see only itself. Build/merge ordering across the
stack is a manual, local-workspace concern.

## Guarantees (invariants)

Every format is verified to uphold three invariants — also exercised by a
property-based fuzz suite:

1. **Idempotency** — formatting already-formatted output is a no-op.
2. **Output reparses** — the result parses without new syntax errors.
3. **Semantic-token preservation** — the sequence of meaningful tokens is
   unchanged; only trivia (whitespace/layout) moves.

If the input has syntax errors, `m1-fmt` returns it unchanged (pass-through
safety).

## Formatting behavior

- Consistent indentation, operator spacing, and brace/statement layout.
- **Line-wrapping** at the width budget: argument lists (greedy fill, no trailing
  comma), binary chains (break before operators), and `if` conditions, accounting
  for trailing end-of-line comments, `;`, and `,`.
- **`--max-blank-lines <n>`** collapses runs of blank lines and strips
  brace-adjacent blanks; author blank lines are otherwise preserved.

## CLI usage

```sh
m1-fmt <file.m1scr>                  # print formatted output to stdout
m1-fmt --max-blank-lines 1 <file>    # cap consecutive blank lines
```

## Build & test

```sh
cargo build --release      # binary at target/release/m1-fmt
cargo test                 # unit + snapshot + corpus + proptest-invariant tests
```

The corpus test formats every `.m1scr` under `$M1_CORPUS_PATH` (falling back to
the sibling `m1-example` example project), checking the invariants and that no
breakable line exceeds the width budget; it is skipped if the directory is absent.

## Note on examples

Example identifiers in the docs and fixtures are **synthetic placeholders**, not
drawn from any real project.

## License

Not yet chosen — decided by the repository owner. Treated as proprietary until
then.

## License

Licensed under the GNU General Public License v3.0 or later (GPL-3.0-or-later) — see [LICENSE](LICENSE).

Copyright (C) 2026 The M1 Tools authors.
