# AGENTS.md — m1-fmt

Guidance for coding agents working in this repository.

## Purpose

The formatter for MoTeC M1 scripts. It is a library first (m1-lsp calls it
for editor formatting and range-formatting) and a CLI second; behaviour
changes here surface in every editor, not just the command line.

## Things that are deliberate (don't "fix" them)

- **The three invariants are the contract.** Idempotency, output-reparses,
  and semantic-token preservation are checked on every format and fuzzed.
  Any change that trades one away is wrong, however nice the output looks.
- **Syntax errors pass through.** Unparseable input is returned byte-for-byte
  unchanged. Never "best-effort format" a broken file — the formatter must
  never corrupt code.
- **Manual defaults, configurable deviation.** Tabs + Allman braces are the
  M1 Development Manual's mandate and therefore the defaults. House-style
  preferences (K&R, spaces) are config options, never the default. When
  unsure what "correct" output is, the manual wins over current behaviour.
- **Config precedence** is defaults < `m1-tools.toml` `[format]` <
  `.m1fmt.toml` < CLI flags. The shared schema lives in `m1-workspace`;
  adding a knob usually means a workspace release first, then wiring it
  through the resolve layer here.

## Gotchas

- Config discovery walks up from the file being formatted, so a stray
  `.m1fmt.toml` in an ancestor directory (e.g. left in `/tmp` by an earlier
  run) can change CLI test behaviour locally.
- The corpus test reads `$M1_CORPUS_PATH` (or a sibling `m1-example/`) and
  skips when absent — a green run on a corpus-less clone proves less than it
  appears to.

## Build / test gate

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI also runs rustdoc with `-D warnings`, a security audit, and an MSRV job.
The MSRV pin in CI (`dtolnay/rust-toolchain@<version>`) must stay in sync with
`rust-version` in `Cargo.toml` — never bump one without the other.

## Dependencies and releases

Depends on `m1-core` and `m1-workspace` via **versioned git tags** — never
`branch`/`path`/`[patch]`; the repo must build exactly like a public clone,
and everything in one lockfile must pin the same m1-core tag. This is a
binary repo: a version bump on `main` makes `release.yml` tag it and upload
prebuilt binaries. After releasing, open the consumer bump PR in `m1-lsp`
immediately rather than waiting for Dependabot.
