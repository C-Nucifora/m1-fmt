# m1-fmt v2 — Design Specification

**Date:** 2026-05-31
**Status:** Approved for implementation
**Author:** Christian Nucifora
**Builds on:** `docs/superpowers/specs/2026-05-30-m1-fmt-v1-design.md`

> **Note:** Example identifiers in this document (e.g. `Vund Klee.Trilby Glonk`)
> are synthetic placeholders, not drawn from any real project. The corpus path is
> resolved via the `M1_CORPUS_PATH` env var (falling back to the sibling EV-M1
> example project).

---

## 1. Purpose

v1 shipped a deterministic, idempotent full-reprint formatter for the MoTeC M1
script language. It normalized indentation, operator spacing, brace placement,
comment spacing, trailing whitespace, the final newline, and collapsed runs of
3+ blank lines to 2. It explicitly did **not** wrap long lines: it only emitted
a `LineTooLong` warning for lines exceeding 88 characters and left them intact
(see v1 spec §3.4).

v2 closes that gap. The headline feature is **automatic line-wrapping of long
lines (>88 chars)**, the single item the v1 spec deferred as needing "a separate
design pass." v2 also makes blank-line collapsing configurable via
`--max-blank-lines` and adds a `proptest`-based fuzz suite that exercises the
three v1 invariants over a much wider input space than the 80-script corpus.

The goal is unchanged: eliminate formatting debate and enforce one consistent
style — now including a hard 88-column ceiling that the formatter *enforces*
rather than merely *reports*.

---

## 2. Scope

### 2.1 In-scope (v2)

- **Automatic line-wrapping at 88 columns.** When a fully-printed *logical line*
  (a statement, an `if (...)`/`when (...)`/`expand (...)` header, or an `is (...)`
  header) would exceed 88 display columns, the printer re-emits it broken across
  multiple physical lines using the break strategy in §3.
- **Argument-list wrapping.** A `CallExpression`'s `ArgumentList` that does not
  fit is broken one-argument-per-line with a trailing comma omitted (M1 has no
  trailing-comma grammar; see §3.5).
- **Binary-expression-chain wrapping.** A `BinaryExpression` chain that does not
  fit is broken *before* each operator at the top level of the chain.
- **Continuation indent.** Wrapped continuation lines are indented at the
  statement's block depth **+ 8 spaces** (two indent units). Justification in §3.3.
- **`--max-blank-lines N` flag** (default 2). Replaces v1's hard-coded "collapse
  3+ to 2" with a configurable ceiling, and additionally strips blank lines
  immediately after an opening `{` and immediately before a closing `}`.
- **`proptest` fuzz suite** (`tests/proptest_invariants.rs`) generating random
  valid M1 fragments and asserting the three v1 invariants.
- All v1 rules continue to hold unchanged.

### 2.2 Out-of-scope (v2) — YAGNI

| Feature | Rationale for deferral |
|---------|----------------------|
| Column-alignment of assignment groups | Contentious; not in CONTRIBUTING; deferred to v3 |
| Wrapping `when`/`is` *subject* expressions across lines | Subjects are short in the corpus (max 41 cols); wrapping them adds parser-position complexity for no observed benefit; v3 |
| Reflowing/re-wrapping comment prose to fit 88 cols | Comment bodies are opaque English; rewrapping changes author intent; v3 |
| Configuration file (`.m1fmt.toml`) | Only one new knob (`--max-blank-lines`); a CLI flag suffices; v3 if more accrue |
| Configurable column limit (`--line-width`) | 88 is the single agreed limit; no demand for a knob; v3 |
| Wrapping ternary expressions across lines | Ternaries in the corpus are short; binary-chain logic does not cover them and they need their own break model; v3 |
| Joining short lines the author hand-broke | The formatter never *removes* author line breaks inside expressions (idempotency risk); v3 if requested |
| LSP range-formatting / format-on-save | Lives in `m1-lsp`, not `m1-fmt` |

---

## 3. Critical Design Decisions

### 3.1 Wrapping is a Re-Print Decision, Not a Post-Pass

**Decision:** Line-wrapping is integrated into the existing full-reprint walk in
`printer.rs`, not bolted on as a string post-processing pass over the rendered
output.

**Justification:**

1. *Token boundaries are known only during the walk.* A post-pass operating on
   the rendered string cannot safely tell an operator inside an identifier-with-
   internal-spaces (`Vund Klee`) from a real binary operator, nor a `,` inside a
   string literal from an argument separator. The CST walk already has this
   information for free.

2. *Idempotency.* If wrapping were a string post-pass, a second run would see
   already-wrapped multi-line output and would have to *decide not to re-wrap* —
   a fragile "is this already wrapped?" check. By making wrapping a function of
   CST structure + a width budget, the second run sees the same structure and the
   same budget and produces identical output by construction (the same argument
   that gave v1 idempotency for free).

3. *Semantic-token preservation.* The walk emits exactly the same token sequence
   whether or not it wraps; only the whitespace *between* tokens changes (newline
   + continuation indent instead of a single space). The semantic-token invariant
   is therefore preserved automatically.

**Mechanism — measure-then-emit.** The printer gains a "trial render" capability:
it renders a node (or statement) into a scratch `String` using the existing
flat (single-line) rules, measures its display width *as it would land at the
current indent*, and only if that width exceeds 88 does it switch to the broken
form. Concretely, `Printer` grows two helpers:

```rust
/// Render `f` into a fresh buffer at the current indent and return it,
/// WITHOUT touching `self.output`. Trivia state is snapshotted and restored
/// so a trial render never consumes comments.
fn trial(&mut self, f: impl FnOnce(&mut Printer)) -> String;

/// True if `flat` placed at column `start_col` would exceed the 88-col limit.
fn exceeds_limit(&self, start_col: usize, flat: &str) -> bool;
```

`trial` snapshots `self.output.len()`, `self.indent`, and the front cursor of
the trivia `VecDeque` (by cloning the deque), runs the closure, captures the
appended slice, truncates `self.output` back, and restores `indent`/`trivia`. It
returns the captured slice. This keeps the existing flat emitters as the source
of truth for the single-line form — the wrapper only *measures* what they would
produce and decides to call them inline or to call a broken variant.

### 3.2 Where to Break — Priority Order

Given a logical line over budget, the printer applies the **first** applicable
strategy, in this order:

1. **Argument lists first (outermost call).** If the over-budget statement
   contains a `CallExpression` whose flat `ArgumentList` is itself the bulk of
   the width, break that argument list one-arg-per-line. This is the most common
   over-budget shape in the corpus (long `Func(a, b, c, d, ...)` calls) and the
   least ambiguous break point.

2. **Binary-operator chains.** If the over-budget node is (or its value/RHS is) a
   top-level `BinaryExpression` chain, break **before** each operator at the
   chain's top level: each operand after the first starts a new continuation
   line, with the operator leading the line.

3. **Otherwise, leave the line long.** If neither strategy applies (e.g. a single
   atom or an identifier-with-internal-spaces longer than 88 by itself), the line
   is emitted flat and the `LineTooLong` warning from v1 is still produced. The
   formatter never breaks *inside* a leaf token.

Only the **outermost** breakable construct on a logical line is broken in v2.
Recursively re-wrapping arguments that are *themselves* still over budget after a
one-per-line break is deferred to v3 (see §2.2 note on nested wrapping is
implicitly out by this rule). In practice a single level of breaking brings every
over-budget corpus line under 88.

**Break-before-operator, not after.** For binary chains we break *before* the
operator (`+`, `&&`, etc. start the continuation line), matching the convention
in `black`, `prettier` (for booleans), and the M1 CONTRIBUTING examples, which
keep the operator visually adjacent to its right operand. This makes the operator
the first non-whitespace token a reader sees on the continuation line.

### 3.3 Continuation Indent = +8 (Two Units)

**Decision:** Continuation lines use the statement's own indent **+ 8 spaces**
(two 4-space units), not +4.

**Justification:** A continuation indented +4 is visually indistinguishable from
a nested block body. Consider:

```
if (Long.Condition.One && Long.Condition.Two
    && Long.Condition.Three) {     // +4: looks like the block body!
    Body.Statement = 1;
}
```

versus the v2 form:

```
if (Long.Condition.One && Long.Condition.Two
        && Long.Condition.Three) {  // +8: clearly a continuation
    Body.Statement = 1;             // +4: the actual body
}
```

The +8 continuation makes the wrapped condition unambiguous against the +4 body
that follows. This matches `rustfmt`'s and `black`'s treatment of wrapped
control-flow conditions. Argument-list continuations also use +8 relative to the
line that opened the call:

```
Result = Some.Module.Compute(First Argument, Second Argument,
        Third Argument, Fourth Argument);
```

The closing `)` and trailing `;` ride on the last argument's line (no dedented
hang-`)` in v2 — that style is a v3 nicety).

### 3.4 Interaction with Comments

EOL comments are measured as part of the logical line's width. The 88-col budget
is computed on the **statement text including its two-space EOL comment**, so a
statement that only overflows *because of* its trailing comment will wrap its
expression to make room. However:

- The EOL comment is always emitted on the **last** physical line of the wrapped
  statement (after the final `;`/`)`), never on an interior continuation line.
- Own-line comments injected *before* a statement are emitted by the existing
  `inject_trivia_before` path and are never themselves wrapped (comment prose
  rewrapping is out of scope — §2.2).
- A `trial` render must not consume trivia. Because `trial` snapshots and
  restores the trivia `VecDeque` (§3.1), the real (non-trial) emit is the only
  one that pops comments, so each comment is emitted exactly once.

If wrapping the expression still cannot bring the line (comment included) under
88 — because the comment alone is very long — the statement is wrapped as far as
the expression allows and the residual over-length is reported as a
`LineTooLong` warning, as in v1.

### 3.5 Identifiers-With-Spaces and `$(SEG)` Interpolation Under Wrapping

The verbatim-preservation guarantee from v1 §3.1 is **strengthened, not
relaxed**, by wrapping. Wrapping only inserts `\n` + continuation indent at
*token boundaries* the CST exposes — between an argument and the following comma,
or between an operand and a binary operator. It never inspects the interior of an
`Identifier`, `Number`, `String`, or `Boolean` token. Therefore:

- `Vund Klee.Trilby Glonk` is never split across lines; if it alone exceeds 88
  it is emitted flat (§3.2 rule 3).
- `Channel.$(SEG).Value` is one identifier token and is emitted verbatim on
  whichever continuation line it lands.
- A `String` literal containing commas or operators is one token; wrapping never
  breaks inside it.

M1 has no trailing-comma grammar (the `argument_list` rule is
`( expr ( , expr )* )`), so the one-arg-per-line form **must not** emit a trailing
comma after the last argument — doing so would introduce a token the semantic
invariant forbids and would produce a parse error. The last argument is followed
directly by `)`.

### 3.6 Idempotency Under Wrapping

`fmt(fmt(src)) == fmt(src)` must continue to hold. The risk wrapping introduces:
on the second pass the input already contains the continuation newlines, and the
CST for the wrapped source has the *same* expression/argument structure (M1
treats intra-expression newlines as insignificant whitespace, confirmed against
the grammar). The printer discards all original whitespace (full reprint) and
re-derives wrapping purely from `(structure, width budget, current indent)`,
which are identical across both passes. Hence the broken form reproduces itself
exactly. This is verified explicitly by the idempotency invariant test, now run
against synthetically-widened inputs in the proptest suite (§7.3).

### 3.7 Blank-Line Normalization (`--max-blank-lines`)

v1 hard-coded "collapse runs of 3+ blank lines to 2" in
`printer::collapse_blank_lines`. v2 parameterizes the ceiling:

- `--max-blank-lines N` (default `2`) collapses any run of `> N` blank lines to
  exactly `N`.
- Additionally, blank lines **immediately after** an opening `{` and
  **immediately before** a closing `}` are always stripped (count 0), regardless
  of `N` — leading/trailing padding inside a block is never meaningful.
- A leading run of blank lines at the very top of the file is stripped to 0.

The ceiling is threaded from the CLI through `format_str` via a new
`FormatOptions` struct (§4). `collapse_blank_lines` takes the ceiling as a
parameter. The brace-adjacent stripping is a second cheap line-oriented pass over
the rendered output (it only looks at whether a trimmed line is `{`/`}` adjacent
to a blank line — no CST needed and no token ever moves).

---

## 4. Architecture Changes

The v1 module layout (§4.1 of the v1 spec) is unchanged; v2 modifies four files
and adds one test file. No new module is introduced — wrapping lives in
`printer.rs` because it is a printing concern, and the width-budget constant
lives there too.

### 4.1 `lib.rs` — `FormatOptions`

A new options struct carries the configurable knobs. `format_str` gains an
options-taking sibling while the old signature is kept as a thin wrapper for
backward compatibility (the test suite and any callers keep working).

```rust
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Maximum consecutive blank lines to keep (default 2).
    pub max_blank_lines: usize,
    /// Hard column ceiling for wrapping (fixed at 88 in v2).
    pub line_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions { max_blank_lines: 2, line_width: 88 }
    }
}

pub fn format_str(src: &str) -> Result<FormatResult, FormatError> {
    format_str_with(src, &FormatOptions::default())
}

pub fn format_str_with(src: &str, opts: &FormatOptions)
    -> Result<FormatResult, FormatError>
{
    // ... syntax-error pass-through unchanged ...
    let output = printer::print_with(&cst, opts);
    // ... warnings + changed unchanged ...
}
```

`line_width` is carried as a field (rather than a literal `88`) so the v3
`--line-width` flag is a one-line change, but it is **not** exposed on the CLI in
v2 (§2.2).

### 4.2 `printer.rs` — wrapping engine

`Printer` gains:

- `width: usize` and `max_blank_lines: usize` fields, populated from
  `FormatOptions`.
- `fn print_with(cst: &Cst, opts: &FormatOptions) -> String` entry point;
  `fn print(cst: &Cst) -> String` becomes `print_with(cst, &Default::default())`.
- `fn trial(&mut self, f: impl FnOnce(&mut Printer)) -> String` (§3.1).
- `fn current_col(&self) -> usize` — display column of the cursor on the current
  physical line (chars since the last `\n` in `self.output`).
- `fn emit_continuation_indent(&mut self)` — emits `(indent * 4) + 8` spaces.
- Wrapping-aware variants invoked only when the flat trial overflows:
  `emit_arg_list_wrapped`, `emit_binary_wrapped`.
- The statement printers (`print_assignment`, `print_expression_stmt`,
  `print_local_decl`) and the header printers (`print_if`, `print_when`,
  `print_expand`, `print_is_clause`) consult a new `fn wrap_value(&mut self, node)`
  that trial-renders the value/condition and dispatches flat-or-wrapped.

`collapse_blank_lines` gains a `max: usize` parameter; a new
`strip_brace_adjacent_blanks(output: &mut String)` runs after it.

### 4.3 `main.rs` — `--max-blank-lines`

```rust
/// Maximum consecutive blank lines to keep
#[arg(long, default_value_t = 2)]
max_blank_lines: usize,
```

`main` builds a `FormatOptions { max_blank_lines: args.max_blank_lines, ..Default::default() }`
and calls a new `format_file_with` / `format_str_with`.

### 4.4 `diagnostics.rs`

Unchanged. `WarningKind::LineTooLong` is still emitted for lines that remain over
budget after wrapping (the unbreakable-atom case, §3.2 rule 3).

### 4.5 Data Flow (delta from v1)

```
format_str_with(src, opts)
  ├─ m1_core::parse(src) → Cst                       (unchanged)
  ├─ syntax_diagnostics empty?  no → pass through      (unchanged)
  ├─ printer::print_with(&cst, opts)
  │     for each statement:
  │        flat = trial(|p| p.print_statement_flat(node))   ← NEW
  │        if exceeds_limit(col, flat) → print broken  ← NEW
  │        else                        → emit(flat)
  │     normalize_trailing(out, opts.max_blank_lines)  ← param NEW
  │     strip_brace_adjacent_blanks(out)               ← NEW
  └─ warnings for residual long lines                  (unchanged)
```

---

## 5. Wrapping Rules Reference

### 5.1 Argument lists

Flat (fits):
```
Result = Compute(First Arg, Second Arg);
```

Wrapped (over 88; continuation at +8 from the call's line indent):
```
Result = Some.Long.Module.Compute(First Argument Name, Second Argument Name,
        Third Argument Name, Fourth Argument Name);
```

Rules:
- The `(` stays on the line with the callee.
- As many arguments as fit (within budget) ride on the opening line; the rest
  flow onto continuation lines, each at indent +8.
- `, ` separates arguments on the same line; a `,` at a line break is emitted at
  the end of the line being closed (trailing on the closed line), then newline +
  continuation indent.
- **No trailing comma** before `)` (§3.5).
- `)` and the statement's `;` ride on the last argument's line.

> v2 uses a greedy fill (pack arguments until the next would overflow). One-arg-
> per-line "expanded" mode is a v3 refinement; greedy fill already brings the
> corpus under budget and is simpler to make idempotent.

### 5.2 Binary-operator chains

Flat (fits):
```
Mask = Flag A | Flag B | Flag C;
```

Wrapped (break before each top-level operator, continuation at +8):
```
Mask = Flag Alpha Long | Flag Bravo Long | Flag Charlie Long
        | Flag Delta Long | Flag Echo Long;
```

Rules:
- Break occurs **before** the operator; the operator leads the continuation line.
- Only top-level operators of the chain are break candidates; parenthesized
  sub-expressions are kept flat unless they are themselves the over-budget
  outermost construct (out of scope — single level only, §3.2).
- Greedy fill: pack operands until the next operand+operator would overflow, then
  break.

### 5.3 Control-flow headers

```
if (First Long Condition && Second Long Condition
        && Third Long Condition) {
    Body = 1;
}
```

The condition inside `if (...)` / `while-like` headers wraps using the
binary-chain rule (§5.2); the opening `{` rides on the line that emits the
closing `)`. `when (...)`, `expand (...)`, and `is (...)` headers wrap the same
way *if* their subject is a binary chain; subject expressions that are single
calls wrap via §5.1.

### 5.4 Blank lines (`--max-blank-lines`)

| Input | `--max-blank-lines 2` (default) | `--max-blank-lines 1` |
|-------|---------------------------------|-----------------------|
| 4 consecutive blank lines | 2 | 1 |
| blank line right after `{` | removed | removed |
| blank line right before `}` | removed | removed |
| leading blank lines at file top | removed | removed |

---

## 6. Error Handling

Unchanged from v1 (§6 of the v1 spec). Specifically:

| Condition | Behaviour |
|-----------|-----------|
| Input has syntax errors | Pass through unchanged; no wrapping attempted |
| Line cannot be brought under 88 (unbreakable atom) | Emit flat; produce `LineTooLong` warning (as v1) |
| `--max-blank-lines 0` | Legal; collapses all blank-line runs to zero |

A `trial` render that would itself panic (it cannot — it calls the same emitters
as the real path) would surface as the same `[m1-fmt BUG]` panic contract as v1.
Wrapping introduces no new error variants and no new `FormatError` cases.

---

## 7. Testing Strategy

The three v1 invariants are **load-bearing for v2** and must continue to pass,
now including wrapped output:

**Invariant 1 — Idempotency:** `format(format(src)) == format(src)`. Wrapping
must reproduce itself (§3.6). Run over the corpus *and* over proptest-generated
inputs.

**Invariant 2 — Output reparses clean:** wrapped output must parse with zero
syntax diagnostics. Critically guards against the trailing-comma trap (§3.5).

**Invariant 3 — Semantic token preservation:** `token_seq(src) ==
token_seq(format(src))`. Wrapping changes only inter-token whitespace, so the
token sequence is unchanged. This is the strongest guard against a wrapping bug
that drops or duplicates an argument.

### 7.1 New snapshot fixtures (`tests/snapshots/`)

Added in the existing `tests/snapshots.rs` runner pattern:

- `wrap_arg_list.m1scr` / `.expected` — a `Func(...)` call over 88 cols, greedy-
  filled continuation at +8.
- `wrap_binary_chain.m1scr` / `.expected` — `a | b | c | ...` over 88, broken
  before operators.
- `wrap_if_condition.m1scr` / `.expected` — `if (a && b && c)` header over 88.
- `wrap_no_trailing_comma.m1scr` / `.expected` — asserts the wrapped call ends
  `...)` not `...,)`.
- `wrap_idempotent.m1scr` / `.expected` — input that is *already* wrapped; output
  equals input (the fixture's `.expected` equals running the formatter once,
  re-verified by the idempotency test).
- `wrap_unbreakable_atom.m1scr` / `.expected` — a single identifier-with-spaces
  longer than 88; emitted flat, unchanged.
- `wrap_eol_comment.m1scr` / `.expected` — statement that overflows only because
  of its trailing comment; expression wraps, comment lands on the last line.
- `blank_lines_max1.m1scr` / `.expected` — exercised with `--max-blank-lines 1`
  via `format_str_with`.
- `blank_lines_brace_adjacent.m1scr` / `.expected` — blanks after `{` / before
  `}` removed.

The `run_snapshot` helper is extended with a `run_snapshot_with(name, opts)`
variant so blank-line fixtures can pass non-default `FormatOptions`.

### 7.2 Unit tests

- `printer.rs` `#[cfg(test)]`: `current_col` arithmetic, `exceeds_limit`
  boundary at exactly 88 vs 89, `trial` does not mutate `self.output` or consume
  trivia, `emit_continuation_indent` width.
- `collapse_blank_lines` with `max = 0, 1, 2`.

### 7.3 Property tests (`tests/proptest_invariants.rs`)

Add `proptest = "1"` to `[dev-dependencies]`. A small M1-fragment generator
produces valid statements (assignments, calls with N args, binary chains, `if`
blocks) with randomly long identifier names, so generated lines frequently exceed
88 and force the wrapper. Each generated source is asserted to satisfy:

```rust
proptest! {
    #[test]
    fn idempotent(src in m1_fragment()) {
        let once = m1_fmt::format_str(&src).unwrap().output;
        let twice = m1_fmt::format_str(&once).unwrap().output;
        prop_assert_eq!(once, twice);
    }

    #[test]
    fn reparses_clean(src in m1_fragment()) {
        if m1_core::parse(&src).syntax_diagnostics().is_empty() {
            let out = m1_fmt::format_str(&src).unwrap().output;
            prop_assert!(m1_core::parse(&out).syntax_diagnostics().is_empty());
        }
    }

    #[test]
    fn tokens_preserved(src in m1_fragment()) {
        // reuse tests/semantic.rs token extractor
        let out = m1_fmt::format_str(&src).unwrap().output;
        prop_assert_eq!(tokens(&src), tokens(&out));
    }
}
```

### 7.4 Corpus regression

The existing `tests/corpus.rs`, `tests/idempotency.rs`, and `tests/semantic.rs`
run unchanged and must stay green. A new assertion in `corpus.rs` verifies that
**after** v2, no formatted corpus line exceeds 88 columns *unless* it is an
unbreakable atom (a line whose CST is a single leaf or identifier-with-spaces).

---

## 8. Deferred to v3

| Item | Reason |
|------|--------|
| Column-alignment of assignment groups | Contentious; needs its own design + opt-in flag |
| Nested / recursive wrapping (wrap arguments that are themselves over budget after one break) | Single-level break already clears the corpus; recursion adds idempotency edge cases |
| One-arg-per-line "expanded" call style + hanging `)` | Greedy fill suffices for v2; expanded style is a stylistic refinement |
| Wrapping `when`/`is` *subject* expressions | Subjects are short in practice |
| Ternary-expression wrapping | Needs a distinct break model from binary chains |
| Comment-prose rewrapping to fit 88 cols | Changes author intent; opaque text |
| `.m1fmt.toml` config file | One CLI knob today; revisit when a third accrues |
| `--line-width` configurable column limit | 88 is the single agreed limit; field exists, flag deferred |
| Joining author-broken short lines | Idempotency risk; only on demand |

---

## 9. Open Questions for the Owner

1. **Greedy fill vs one-per-line for argument lists.** v2 chooses greedy fill
   (pack until overflow) for minimal diff churn. If the team prefers the
   `black`-style "all args on their own line once any break is needed," that is a
   one-function change in `emit_arg_list_wrapped` — confirm the preference before
   the snapshots are frozen.

2. **+8 vs +4 continuation indent.** §3.3 argues +8 to disambiguate from block
   bodies. Confirm this reads well against the real corpus's deepest nesting
   (3 levels observed) before freezing.

3. **`--max-blank-lines` default.** Default is 2 (matches v1 behaviour). Should
   the brace-adjacent stripping (blank after `{` / before `}`) be unconditional,
   or also gated behind the flag? v2 makes it unconditional on the grounds that
   such padding is never intentional; confirm.
