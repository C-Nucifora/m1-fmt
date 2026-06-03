# m1-fmt

An auto-formatter (pretty-printer) for the MoTeC M1 script language (`.m1scr`).
It reparses the formatted output to guarantee it never changes a script's
meaning. It is both a **library** (consumed by `m1-lsp` for
`textDocument/formatting`) and a **CLI**.

## Workspace layout

The M1 toolchain lives in **six separate repositories**. They are not published
to crates.io; instead each crate pins its upstreams as **versioned git-tag Cargo
dependencies** (e.g. `m1-core = { git = "…/m1-core.git", tag = "v0.3.1" }`), so
this crate **does** build from a standalone clone — Cargo fetches its upstreams
from their tagged releases. Checking the whole set out as siblings under one
parent directory is handy for cross-repo work, but is not required to build:

```
<parent>/
├── tree-sitter-m1/   # grammar (root)
├── m1-core/          # parse / CST / diagnostics
├── m1-lint/          # linter
├── m1-fmt/           # this crate
├── m1-typecheck/     # type checker
└── m1-lsp/           # language server; depends on the four above
```

**`m1-fmt` depends on `m1-core`** (a git-tag dep that transitively pulls in
`tree-sitter-m1`), and is itself a git-tag dependency of `m1-lsp`. (The
`m1-example` corpus project, used by the corpus test, is an optional sibling
checkout.)

Because every dependency is pinned by tag, the coupling **is** visible on
GitHub — each `Cargo.toml` names its upstreams and their versions, and
Dependabot opens bump PRs as new upstream tags ship. Cutting a new upstream
release and bumping `tag = "vX.Y.Z"` in each consumer is what propagates a
change across the stack.

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

- **Manual-correct style by default:** Allman braces (each brace on its own line)
  and tab indentation, per the M1 Build Development Manual. Override in
  `.m1fmt.toml` for a different house style:
  ```toml
  brace_style  = "allman"   # or "kr"
  indent_style = "tab"      # or "spaces"
  indent_width = 4          # columns per level (tab display width / space count)
  ```
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
m1-fmt --range 10:14 <file>          # format only lines 10–14, leave the rest as-is
```

`--range START:END` (1-based, inclusive) formats only the top-level statements
overlapping that line range and leaves every other line byte-for-byte unchanged;
the range is snapped outward to whole statement boundaries. It composes with
`--check`, `--in-place`, and `--diff`, and backs editor format-on-selection (the
LSP calls the underlying `format_range` to build `textDocument/rangeFormatting`
edits). Expression fragments aren't independently parseable, so a range that
overlaps no complete statement is a no-op.

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
