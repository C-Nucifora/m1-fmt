# m1-fmt

An auto-formatter (pretty-printer) for the MoTeC M1 script language (`.m1scr`).
It reparses the formatted output to guarantee it never changes a script's
meaning. It is both a **library** (consumed by `m1-lsp` for editor formatting)
and a **CLI**.

## Install

Prebuilt binaries for Linux, macOS, and Windows are attached to each
[release](https://github.com/C-Nucifora/m1-fmt/releases). Or build from
source:

```sh
cargo install --git https://github.com/C-Nucifora/m1-fmt.git --tag <latest>
```

## Usage

```sh
m1-fmt file.m1scr            # print formatted output to stdout
m1-fmt --check .             # CI mode: exit non-zero if anything would change
m1-fmt -i src/               # format a directory in place
m1-fmt --diff file.m1scr     # show what would change as a unified diff
m1-fmt --range 10:14 file    # format only those lines, leave the rest untouched
```

See `m1-fmt --help` for the full flag list. Directory runs are parallel by
default (`--jobs` to control). A file with syntax errors is left byte-for-byte
unchanged — formatting never corrupts unparseable input — and `--check`
reports it and exits non-zero so CI doesn't mistake it for a clean file.

## Guarantees

Every format is verified to uphold three invariants — also exercised by a
property-based fuzz suite:

1. **Idempotency** — formatting already-formatted output is a no-op.
2. **Output reparses** — the result parses without new syntax errors.
3. **Semantic-token preservation** — the sequence of meaningful tokens is
   unchanged; only whitespace and layout move.

## Style and configuration

The defaults follow the M1 Development Manual: **tab indentation and Allman
braces**. Teams with a different house style can override in config — the
manual is the default, deviation is a choice:

```toml
# .m1fmt.toml
indent_style     = "tab"    # or "spaces"
brace_style      = "allman" # or "kr"
max_line_length  = 88
```

The unified workspace config (`m1-tools.toml [format]`) uses `line_width` instead of
`max_line_length`; `.m1fmt.toml` always uses `max_line_length`. Unknown keys are
silently ignored, so a mismatched key name is never an error — it just has no effect.

The same knobs are also available as CLI flags for a one-off run without
committing a config file (handy for previewing a house style or in an ad-hoc CI
invocation):

```sh
m1-fmt --indent-style spaces --brace-style kr file.m1scr
m1-fmt --indent-width 2 --line-width 100 --continuation-indent 2 file.m1scr
```

Precedence: built-in defaults < `m1-tools.toml` `[format]` < `.m1fmt.toml` <
CLI flags. The workspace-level `m1-tools.toml` is shared with `m1-lint`,
`m1-lsp`, and the editor integrations — see the
[m1-tools configuration docs](https://github.com/C-Nucifora/m1-tools#configuration)
for the full set of knobs.

Beyond indentation and braces, the formatter handles operator spacing,
line-wrapping at the width budget, and blank-line policy, with a few opt-in
extras (assignment alignment, comment reflow). For hand-aligned tables and
other deliberate layout, `// @m1:fmt(off)` / `// @m1:fmt(on)` comments mark a
region the formatter passes through untouched.

## Development

The CI gate is `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --all -- --check`. The corpus test formats every `.m1scr` under
`$M1_CORPUS_PATH` (falling back to a sibling `m1-example/` checkout) and
checks the invariants; it skips if no corpus is present. Example identifiers
in docs and fixtures are synthetic placeholders, not drawn from any real
project.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
