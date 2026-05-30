# m1-fmt v1 — Design Specification

**Date:** 2026-05-30
**Status:** Approved for implementation
**Author:** Christian Nucifora

> **Note:** Example identifiers in this document (e.g. `Vund Klee.Trilby Glonk`)
> are synthetic placeholders, not drawn from any real project. The corpus path is
> resolved via the `M1_CORPUS_PATH` env var (falling back to the sibling m1-example
> example project).

---

## 1. Purpose

`m1-fmt` is a deterministic, idempotent autoformatter for the MoTeC M1 script language (`.m1scr`). It reads a source file, parses it via `m1-core`, and emits a canonically-formatted version that satisfies the style rules in the project's CONTRIBUTING guide. Its primary goal is to eliminate formatting debates in code review and enforce a single consistent style across the corpus of M1 scripts.

---

## 2. Scope

### 2.1 In-scope (v1)

- 4-space indentation; no tabs anywhere
- Brace placement: opening brace on the same line as the statement; closing brace on its own line aligned with the opener
- Operator spacing: spaces around all binary operators (`+`, `-`, `*`, `/`, `%`, `==`, `!=`, `<`, `>`, `<=`, `>=`, `&&`, `||`, `&`, `|`, `^`, `<<`, `>>`, `=`, `+=`, `-=`, `*=`, `/=`, `?`, `:`)
- Keyword-paren spacing: one space between a control keyword (`if`, `when`, `is`, `expand`) and the following `(`
- Function-call paren spacing: NO space between a function name (identifier) and its `(`
- Trailing whitespace removal on every line
- Final newline (exactly one `\n` at end of file)
- Comment spacing: `//` followed by a single space before comment text (i.e., `// text`); two spaces before an end-of-line comment on a statement line; own-line comments indented to the same level as surrounding code
- Blank line normalization: collapse 3+ consecutive blank lines to 2; preserve intentional blank lines that separate logical sections
- One statement per line (already enforced by the grammar; the formatter preserves this)
- `expand`/`when`/`is` construct formatting consistent with the above rules

### 2.2 Out-of-scope (v1) — YAGNI

| Feature | Rationale for deferral |
|---------|----------------------|
| Automatic line-wrapping of long lines (>88 chars) | Complex to do safely; affects readability decisions a human should control; deferred to v2 |
| Column-alignment of assignments in a group | Contentious; not specified in CONTRIBUTING; deferred |
| Import/use reordering | No import system in M1; N/A |
| Blank-line insertion between functions/sections | Requires semantic heuristics; v2 |
| Sorting of identifiers or declarations | Not in spec |
| Reformatting of string literals | Literals are opaque; must be preserved verbatim |
| `$(SEG)` interpolation reformatting | Interpolations inside identifiers are opaque; preserved verbatim |
| Multi-line expression wrapping | Deferred to v2 |
| Configuration file (`.m1fmt.toml`) | One global style; no config needed in v1 |

---

## 3. Critical Design Decisions

### 3.1 Reprint Strategy: Full Pretty-Print from the CST

**Decision:** Full pretty-print (also called "reprinting" or "unparsing") — discard all original whitespace and emit a fresh token stream, applying spacing and indentation rules from scratch.

**Justification:**

1. *Correctness*: Minimal-edit approaches (only rewrite whitespace at detected violations) are complex to make exhaustive; every new rule requires a new violation detector. Full reprinting makes the output deterministic by construction — the same AST structure always produces the same output regardless of the input's original whitespace.

2. *Idempotency for free*: A full reprinter is idempotent if the rules are self-consistent, because reprinting the output a second time runs the exact same rules over the exact same structure.

3. *Simplicity*: The printer visits every node once in a single recursive descent. There is no need to detect "is this already correct?" before rewriting.

4. *M1 corpus characteristics*: The corpus shows no cases where existing whitespace carries semantic meaning beyond what the AST captures. Identifiers with internal spaces (e.g., `Vund Klee.Trilby Glonk`) are single `Identifier` tokens — their internal text is preserved verbatim by the printer, not rewritten.

**Verbatim preservation guarantee:** The printer MUST preserve the literal text of all leaf tokens that carry semantic meaning: `Identifier.text()` (including internal spaces and `$(...)` interpolations), `Number.text()`, `String.text()`, `Boolean.text()`. It only controls the whitespace *between* tokens, not *within* them.

**Semantic equivalence:** The formatter must never change the non-whitespace token sequence. This is verified by the test suite (see Section 7).

### 3.2 Comment and Trivia Handling

**The problem:** In the tree-sitter grammar, `LineComment` and `BlockComment` nodes are defined as `extras`. This means they can appear between any two tokens but are NOT guaranteed to appear as `named_children()` of statement nodes. They may be siblings at the SourceFile level, appear in unexpected positions in the child list, or be entirely absent from `named_children()` while still present in the full child list (including anonymous nodes).

**Concrete approach: Position-guided trivia collection**

The printer uses a two-pass strategy:

**Pass 1 — Trivia collection:** Walk the entire CST using `Node::children()` (NOT `named_children()`) recursively, collecting every `LineComment` and `BlockComment` node along with its byte offset (`node.byte_range().start`). Build a sorted `Vec<TriviaItem>` ordered by source position.

```
struct TriviaItem {
    byte_offset: usize,
    text: &str,          // raw comment text including //
    is_block: bool,
    source_line: usize,  // 0-indexed line in original source
}
```

**Pass 2 — Printing with trivia injection:** The printer walks the CST in source order. Before emitting each statement/declaration node, check the trivia list for any `TriviaItem` whose `byte_offset` falls between the previous emitted token and the current node's start. Emit those trivia items first, at the current indentation level, then emit the node.

For end-of-line comments: a `TriviaItem` whose `source_line` matches the statement's start line is treated as an EOL comment and appended (with two spaces) after the statement's trailing `;` or `}`.

**Formatting rules for emitted comments:**
- `LineComment`: emit `// ` + trimmed comment body (strip leading `//` + any existing space, then re-emit as `// ` + body). If the comment text after `//` is already a single space, preserve the body exactly.
- `BlockComment`: emit as-is, preserving internal newlines, but apply current indentation to continuation lines if the block comment spans multiple lines.
- Own-line comment: indented to current block depth (4 spaces × depth).
- EOL comment: two spaces before `//`, on the same line as the statement.

**Edge cases:**
- Comments at file scope (depth 0): emitted at column 0.
- Comments inside `expand` blocks: indented at the block's depth.
- Orphaned comments after the last statement in a block: emitted before the closing `}`.
- Comments at the very end of the file: emitted after the last statement, before the final newline.

**Why not a token+trivia stream?** tree-sitter's API (as exposed through m1-core) does not provide a linear token stream directly. Building the trivia list by CST walk is O(n) and straightforward. The CST's source-position information (`byte_range()`) is sufficient to correlate trivia to their surrounding nodes without needing a separate tokenizer.

### 3.3 Idempotency and Safety

**Idempotency:** `fmt(fmt(src)) == fmt(src)` for all valid inputs. This holds by construction for the full-reprint strategy, because the second application sees the same node structure and applies identical rules. The test suite enforces this explicitly.

**Syntax-error pass-through:** If `m1_core::parse(src).syntax_diagnostics()` returns a non-empty list, the formatter returns the source unchanged and emits a diagnostic message to stderr:
```
m1-fmt: skipping <path>: input has N syntax error(s)
```
This guarantees the formatter never emits broken output. It never attempts to format partially-valid files.

**No semantic changes:** The formatter guarantees that the non-whitespace, non-comment token sequence of the output is byte-for-byte identical to that of the input. The test suite verifies this by extracting and comparing token sequences (see Section 7).

### 3.4 Line Wrapping

**Decision: Not implemented in v1.**

The formatter normalizes indentation and spacing but does NOT auto-wrap long lines (>88 characters). Existing line breaks are preserved as-is; continuation lines are indented at their current depth + 4 spaces (or the block's natural depth if they are part of a multi-line expression already broken by the user).

The 88-character limit is treated as advisory only in v1. A `--check` run will report lines exceeding 88 characters as warnings, but the formatter will not modify them.

This is deferred to v2 with the following note: line-wrapping requires decisions about where to break expressions (after an operator? before? at argument boundaries?) that have no single correct answer and require a separate design pass.

---

## 4. Architecture

### 4.1 Crate Layout

```
m1-fmt/
  Cargo.toml         (workspace member; depends on m1-core)
  src/
    main.rs          (CLI: arg parsing, stdin/file I/O, exit codes)
    lib.rs           (public API: format_str, format_file, FormatResult)
    printer.rs       (core CST walker and token emitter)
    trivia.rs        (comment/trivia collection and injection)
    rules.rs         (spacing and punctuation rules; pure functions)
    diagnostics.rs   (FormatError, FormatWarning types)
  tests/
    idempotency.rs   (property-based: fmt(fmt(x)) == fmt(x))
    corpus.rs        (run against all 80 corpus scripts)
    semantic.rs      (non-trivia token sequence preservation)
    snapshots/       (hand-crafted golden inputs with expected outputs)
```

**Dependency rule:** `m1-fmt` depends ONLY on `m1-core`. It does NOT import `tree-sitter` or `tree-sitter-m1` directly. All CST access goes through `m1-core`'s public API (`m1_core::parse`, `Node`, `Kind`, `Cst`, etc.).

### 4.2 Module Responsibilities

**`main.rs`**
- Parse CLI arguments: `[OPTIONS] [FILE...]`
- Flags: `--check` (exit 1 if any file would change), `--diff` (print unified diff), `--in-place` / `-i` (overwrite), `--stdin-filename` (for stdin mode)
- Read from stdin if no files given
- Call `lib::format_str` or `lib::format_file`
- Print diagnostics to stderr; formatted output to stdout (or in-place)
- Exit codes: 0 = all files already formatted; 1 = files would change (check mode) or error

**`lib.rs`**
- `pub fn format_str(src: &str) -> FormatResult` — parses and formats a string
- `pub fn format_file(path: &Path) -> FormatResult` — reads file, calls `format_str`
- `pub struct FormatResult { pub output: String, pub changed: bool, pub warnings: Vec<FormatWarning> }`
- `pub struct FormatError` — returned if input has syntax errors or is otherwise unformattable
- Checks `syntax_diagnostics()` before formatting; returns `Err(FormatError)` if non-empty

**`printer.rs`**
- `pub struct Printer<'src>` — owns the CST, trivia list, indent depth, output buffer
- `pub fn print(cst: &Cst) -> String` — entry point; builds trivia list, then walks from root
- Recursive descent methods: `print_source_file`, `print_statement`, `print_block`, `print_expression`, `print_declaration`, etc.
- Manages `indent_level: usize` (bumped on entering `{`, decremented on leaving `}`)
- Calls into `trivia.rs` to inject comments at the right positions

**`trivia.rs`**
- `pub fn collect_trivia(cst: &Cst) -> Vec<TriviaItem>` — full CST walk, collects all `LineComment`/`BlockComment` nodes
- `pub struct TriviaItem { byte_offset: usize, end_offset: usize, text: String, source_line: usize }`
- `pub fn format_line_comment(raw: &str) -> String` — normalizes `// text` (strips excess spaces after `//`, ensures single space)
- Trivia items are consumed from the front of a sorted list as the printer advances through the source

**`rules.rs`**
- Pure functions returning spacing decisions
- `pub fn space_before_token(kind: Kind, parent_kind: Kind, prev_kind: Kind) -> SpaceDecision`
- `pub enum SpaceDecision { None, Single, Newline, NewlineIndented }`
- Encodes: no space before `;` or `,`; space around binary operators; space after keyword before `(`; no space between identifier and `(`; etc.
- No state; entirely unit-testable in isolation

**`diagnostics.rs`**
- `pub enum FormatError { SyntaxErrors(Vec<Diagnostic>), IoError(std::io::Error) }`
- `pub struct FormatWarning { pub kind: WarningKind, pub line: usize, pub col: usize, pub message: String }`
- `pub enum WarningKind { LineTooLong, ... }`

### 4.3 Data Flow

```
                    ┌─────────────────────────────────────────────┐
  source text       │                   lib.rs                    │
  (str / file)  ──► │  1. m1_core::parse(src) → Cst              │
                    │  2. cst.syntax_diagnostics() → empty?       │
                    │     no  → return Err(FormatError::Syntax)   │
                    │     yes → continue                          │
                    │  3. trivia::collect_trivia(&cst)            │
                    │        → Vec<TriviaItem>                    │
                    │  4. printer::Printer::new(cst, trivia)      │
                    │  5. printer.print_source_file()             │
                    │        → String (formatted output)          │
                    │  6. append final \n if missing              │
                    │  7. return FormatResult { output, changed } │
                    └─────────────────────────────────────────────┘
                                         │
                              formatted text / error
                                         │
                              ┌──────────▼──────────┐
                              │       main.rs        │
                              │  stdout / in-place   │
                              │  diff / check-only   │
                              └─────────────────────┘
```

### 4.4 Printer Algorithm (pseudocode)

```
fn print_source_file(root: Node):
    for child in root.children():
        inject_trivia_before(child.byte_range().start)
        print_statement(child)
    inject_remaining_trivia()
    ensure_final_newline()

fn print_statement(node: Node):
    match node.kind():
        LocalDeclaration  => print_local_decl(node)
        AssignmentStatement => print_assignment(node)
        ExpressionStatement => print_expression_stmt(node)
        IfStatement       => print_if(node)
        WhenStatement     => print_when(node)
        ExpandStatement   => print_expand(node)
        EmptyStatement    => // emit nothing (bare semicolons stripped)
        LineComment / BlockComment => // handled by trivia injector
        _                 => emit_verbatim(node)

fn print_if(node: Node):
    emit("if (")
    print_expression(condition_child(node))
    emit(") {")
    newline()
    indent += 1
    for stmt in block_children(node):
        inject_trivia_before(stmt)
        emit_indent()
        print_statement(stmt)
        newline()
    indent -= 1
    emit_indent()
    emit("}")
    if has_else(node):
        emit(" else {")
        // ... similar
    newline()

fn inject_trivia_before(byte_pos: usize):
    while trivia_list.front().byte_offset < byte_pos:
        item = trivia_list.pop_front()
        if item is eol_comment (same source line as current stmt):
            // defer: attach after current stmt's semicolon
            pending_eol_comment = item
        else:
            emit_indent()
            emit(format_line_comment(item.text))
            newline()
```

---

## 5. Formatting Rules Reference

### 5.1 Indentation

- Base indentation: 0 spaces at file scope
- Each `{...}` block increases indent by 4 spaces
- `expand`, `when`, `if`, `else` blocks all use the same rule
- Continuation lines (not implemented in v1 — see Section 3.4): would use 8 spaces (base + 4)

### 5.2 Braces

```
// Correct
if (condition) {
    body;
}

// Correct
when (x) {
    is (0) {
        body;
    }
}

// Correct
expand (SEG = 1 to 6) {
    body;
}
```

Opening `{` always on the same line, preceded by a single space. Closing `}` always on its own line, aligned to the column of the keyword that opened the block.

### 5.3 Operators and Spacing

Binary operators: `a + b`, `a - b`, `a * b`, `a / b`, `a % b`, `a == b`, `a != b`, `a < b`, `a > b`, `a <= b`, `a >= b`, `a && b`, `a || b`, `a & b`, `a | b`, `a ^ b`, `a << b`, `a >> b`

Assignment: `x = expr`, `x += expr`, `x -= expr`, `x *= expr`, `x /= expr`

Ternary: `cond ? a : b` (spaces around `?` and `:`)

Unary: `-x`, `!x`, `~x` — no space between operator and operand

Member access: `Obj.Member` — no space around `.`

### 5.4 Parentheses

Control keywords: `if (`, `when (`, `is (`, `expand (`

Function calls: `Func(arg1, arg2)` — no space before `(`

Inside parens: no padding spaces — `(expr)` not `( expr )`

### 5.5 Comments

Own-line comment:
```
// This is a comment
statement;
```

End-of-line comment (two spaces before `//`):
```
statement;  // explanation
```

Block comment (indented, internal newlines preserved):
```
/*
 * Multi-line block comment
 */
```

### 5.6 Special Language Constructs

**Identifiers with internal spaces** (`Vund Klee.Trilby Glonk`): The `Identifier` node's `text()` is emitted verbatim. The formatter does not inspect or modify the contents of identifier tokens.

**`$(SEG)` interpolation**: Interpolations appear inside identifier tokens (e.g., `Channel.$(SEG).Value`). The entire identifier token text is emitted verbatim.

**`expand` with range**: `expand (SEG = 1 to 6) {` — the `to` keyword receives spaces on both sides per binary-operator rules.

**`when`/`is` clauses**:
```
when (expr) {
    is (value) {
        body;
    }
}
```

---

## 6. Error Handling

| Condition | Behaviour |
|-----------|-----------|
| Input has syntax errors | Return input unchanged; print `m1-fmt: skipping <path>: N syntax error(s)` to stderr; exit 0 (not an error of the formatter) |
| File not found | Print IO error to stderr; exit 2 |
| Permission denied | Print IO error to stderr; exit 2 |
| Formatter internal panic | Propagate as Rust panic with `[m1-fmt BUG]` prefix; ask user to file an issue |
| Output would be empty but input was not | Treat as internal error; return input unchanged |

The formatter is explicitly non-crashing on malformed input. Syntax-error pass-through means that even if tree-sitter partially-parsed a file, the original bytes are returned verbatim.

---

## 7. Testing Strategy

### 7.1 No Golden Oracle Problem

Because the corpus scripts are the ground truth and we cannot guarantee they are already correctly formatted, we do NOT use corpus scripts as golden-output fixtures. Instead, the test suite relies on three invariants that must hold for any valid formatter:

**Invariant 1 — Idempotency:**
```
∀ src: format(format(src)) == format(src)
```
Run against all 80 corpus scripts. If formatting is stable, the second application must produce no changes.

**Invariant 2 — Output reparses clean:**
```
∀ src where syntax_diagnostics(src).is_empty():
    syntax_diagnostics(format(src)).is_empty()
```
The formatted output must always be syntactically valid M1.

**Invariant 3 — Semantic token preservation:**
```
∀ src: token_seq(src) == token_seq(format(src))
```
Where `token_seq` extracts all non-whitespace, non-comment tokens in source order. The formatter must not add, remove, or alter any semantic token.

### 7.2 Test Modules

**`tests/idempotency.rs`**
```rust
// For each script in corpus, run format twice and assert equality
#[test]
fn idempotency_corpus() {
    for script in corpus_scripts() {
        let once = format_str(&script).unwrap().output;
        let twice = format_str(&once).unwrap().output;
        assert_eq!(once, twice, "idempotency failed for {}", script_name);
    }
}
```

**`tests/semantic.rs`**
```rust
// Strip whitespace/comments from input and output, compare token sequences
fn extract_tokens(src: &str) -> Vec<String> {
    let cst = m1_core::parse(src);
    cst.root()
       .children()
       .iter()
       .filter(|n| !matches!(n.kind(), Kind::LineComment | Kind::BlockComment))
       .map(|n| n.text().to_string())
       .collect()
    // Note: recursive; need to descend fully
}

#[test]
fn semantic_preservation_corpus() {
    for script in corpus_scripts() {
        let result = format_str(&script).unwrap();
        let orig_tokens = extract_tokens(&script);
        let fmt_tokens = extract_tokens(&result.output);
        assert_eq!(orig_tokens, fmt_tokens);
    }
}
```

**`tests/corpus.rs`**
```rust
// Run formatter over all 80 scripts; check no panics, no syntax errors introduced
#[test]
fn corpus_no_crash() {
    for script in corpus_scripts() {
        let result = std::panic::catch_unwind(|| format_str(&script));
        assert!(result.is_ok(), "formatter panicked on {}", script_name);
    }
}

#[test]
fn corpus_output_parses_clean() {
    for script in corpus_scripts() {
        if m1_core::parse(&script).syntax_diagnostics().is_empty() {
            let fmt = format_str(&script).unwrap().output;
            let diags = m1_core::parse(&fmt).syntax_diagnostics();
            assert!(diags.is_empty(), "formatted output has syntax errors: {:?}", diags);
        }
    }
}
```

**`tests/snapshots/`**
Hand-crafted test cases covering:
- `operator_spacing.m1scr` / `operator_spacing.expected` — all binary operators
- `brace_placement.m1scr` / `brace_placement.expected` — if/when/expand blocks
- `comment_eol.m1scr` / `comment_eol.expected` — end-of-line comment spacing
- `comment_own_line.m1scr` / `comment_own_line.expected` — own-line comment indentation
- `keyword_paren_spacing.m1scr` / `keyword_paren_spacing.expected` — control vs function parens
- `identifier_internal_spaces.m1scr` / `identifier_internal_spaces.expected` — verbatim preservation
- `expand_interpolation.m1scr` / `expand_interpolation.expected` — `$(SEG)` preservation
- `trailing_whitespace.m1scr` / `trailing_whitespace.expected` — stripped trailing space
- `final_newline.m1scr` / `final_newline.expected` — missing/extra newlines
- `syntax_error_passthrough.m1scr` — must return unchanged (error case)

### 7.3 Property Tests

Use `proptest` or hand-rolled fuzzing with randomly-shuffled whitespace in valid M1 fragments to verify idempotency and non-crash properties over a wider space.

---

## 8. Open Questions for the Owner

1. **Comment attachment heuristic:** The proposed strategy attaches a comment to the *next* statement if it appears on its own line. An alternative is to attach to the *previous* statement when the comment is on the same line as the closing `}` of a block. Which convention matches the corpus intent? (This affects whether `// end of if block` lands before or after the `}`.)

2. **Blank line policy:** Should the formatter collapse all 3+ consecutive blank lines to 2, or should it be more aggressive (e.g., collapse all intra-block blank lines to at most 1)? The current spec says "collapse 3+ to 2" but the CONTRIBUTING guide says "blank lines between logical sections" without a max count. Does the team want `--max-blank-lines` configurable or is 2 always correct?

3. **EOL comment threshold:** The current rule emits two spaces before an end-of-line comment (`stmt;  // note`). The CONTRIBUTING guide says "two spaces before an end-of-line comment." Should the formatter always normalize to exactly two spaces, even if the original had 4 or more for alignment? (Normalizing to 2 breaks aligned comment columns.)

4. **`--check` exit code semantics:** The proposed exit code 1 means "files would change." Should the formatter use exit code 2 for IO/syntax errors and reserve 1 for "formatting differences found" (mirroring `gofmt -l` or `rustfmt --check`)? Or should any error be a non-zero exit, and "differences found" be reported differently?
