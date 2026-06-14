//! Regression tests for specific reported defects.
//!
//! - #125: a bare-identifier `local` initializer must not gain a double space.
//! - #126: an own-line comment before `else`/`else if` must stay above it.
//! - #127: a long assignment with no internal break point must break after `=`.
//! - tilde: `~expr` (bitwise NOT) formats correctly and idempotently.

use m1_fmt::{FormatError, FormatOptions, format_str, format_str_with};

fn fmt(src: &str) -> String {
    format_str(src).unwrap().output
}

/// #125: `local a = b;` (a bare-identifier initializer) must round-trip
/// unchanged. The declared-name `Identifier` arm previously also matched the
/// initializer identifier, emitting a leading space on top of the `" = "`
/// separator and producing `local a =  b;`.
#[test]
fn local_bare_identifier_initializer_keeps_single_space() {
    let out = fmt("local a = b;\n");
    assert_eq!(out, "local a = b;\n", "got:\n{out}");
}

/// #125: a multi-word identifier RHS (M1 allows spaces in identifiers) must
/// also stay single-spaced after `=`, and the result must be idempotent.
#[test]
fn local_multiword_identifier_initializer_is_stable() {
    let out = fmt("local Left Motor Torque = Torque Request;\n");
    assert_eq!(
        out, "local Left Motor Torque = Torque Request;\n",
        "got:\n{out}"
    );
    assert_eq!(fmt(&out), out, "not idempotent");
}

/// #125: a `static` + typed local with a bare-identifier initializer keeps the
/// declared name spaced once and the initializer spaced once.
#[test]
fn static_typed_local_bare_initializer_is_correct() {
    let out = fmt("static local <T> name = other;\n");
    assert_eq!(out, "static local <T> name = other;\n", "got:\n{out}");
}

/// #126: an own-line comment immediately before `else` must remain above the
/// keyword, not be relocated between `else` and its opening brace.
#[test]
fn comment_before_else_stays_above_keyword() {
    let src = "if (a > 0)\n{\n\tb = 1;\n}\n// choose fallback\nelse\n{\n\tb = 2;\n}\n";
    let out = fmt(src);
    assert_eq!(out, src, "comment was relocated; got:\n{out}");
}

/// #126: the same must hold for `else if`.
#[test]
fn comment_before_else_if_stays_above_keyword() {
    let src = "if (a > 0)\n{\n\tb = 1;\n}\n// fallback branch\nelse if (a < 0)\n{\n\tb = 2;\n}\n";
    let out = fmt(src);
    assert_eq!(out, src, "comment was relocated; got:\n{out}");
}

/// #127: an assignment whose RHS overflows `line_width` but has no internal
/// break point (a call with an empty argument list) must break after `=` onto a
/// tab-indented continuation line, rather than being emitted over-budget with
/// only a warning.
#[test]
fn long_assignment_without_break_point_breaks_after_equals() {
    let src = "Driveline.Accumulator.BMS Version = DBC.BMU.ExtendedPackStatus.BmuHardwareVersion.GetUnsignedInteger();\n";
    let result = format_str_with(src, &FormatOptions::default()).unwrap();
    assert_eq!(
        result.output,
        "Driveline.Accumulator.BMS Version =\n\tDBC.BMU.ExtendedPackStatus.BmuHardwareVersion.GetUnsignedInteger();\n",
        "got:\n{}",
        result.output
    );
    // No line-too-long warning should remain once the break is applied.
    assert!(
        result.warnings.is_empty(),
        "unexpected warnings: {:?}",
        result.warnings
    );
    // The break must be a fixed point.
    assert_eq!(fmt(&result.output), result.output, "not idempotent");
}

/// #127: a normal-width assignment must NOT gain a spurious break after `=`.
#[test]
fn short_assignment_is_not_broken() {
    let out = fmt("x = some.call();\n");
    assert_eq!(out, "x = some.call();\n", "got:\n{out}");
}

// ---- Tilde (bitwise NOT) coverage -----------------------------------------
//
// `Kind::Tilde` is covered by `m1_core::is_unary_op` but `emit_unary` and
// `emit_expr_flat` previously matched only `Kind::Minus` and `Kind::Bang`
// explicitly, leaving `~` to fall through to `emit_verbatim`. The verbatim
// path happened to produce correct output, but there were no tests and no
// explicit arm. A `Kind::Tilde` arm matching `Kind::Minus` (tight-binding,
// no trailing space) is now explicit. Tests added for coverage and stability.

/// `~expr` formats with `~` tight against the operand, identical to `-` and `!`.
#[test]
fn tilde_unary_formats_correctly() {
    let out = fmt("x = ~a;\n");
    assert_eq!(out, "x = ~a;\n", "got:\n{out}");
}

/// `~expr` is idempotent: formatting twice changes nothing.
#[test]
fn tilde_unary_is_idempotent() {
    let out = fmt("x = ~a;\n");
    assert_eq!(fmt(&out), out, "not idempotent");
}

/// `~` applied to a member expression keeps the operand intact.
#[test]
fn tilde_member_expression_formats_correctly() {
    let out = fmt("x = ~foo.bar;\n");
    assert_eq!(out, "x = ~foo.bar;\n", "got:\n{out}");
    assert_eq!(fmt(&out), out, "not idempotent");
}

/// `~` applied to a parenthesized expression.
#[test]
fn tilde_paren_expression_formats_correctly() {
    let out = fmt("x = ~(a + b);\n");
    assert_eq!(out, "x = ~(a + b);\n", "got:\n{out}");
    assert_eq!(fmt(&out), out, "not idempotent");
}

/// `~` as the inner operand of a binary expression.
#[test]
fn tilde_inside_binary_expression_formats_correctly() {
    let out = fmt("x = a & ~b;\n");
    assert_eq!(out, "x = a & ~b;\n", "got:\n{out}");
    assert_eq!(fmt(&out), out, "not idempotent");
}

// ---- FormatError surface ---------------------------------------------------
//
// `FormatError` is the formatter's only error type. Syntax errors are NOT an
// error: an unparseable buffer is returned as `Ok(FormatResult { changed:
// false, .. })` (data-preserving passthrough) and surfaced separately via
// `syntax_error_count`. The only `Err` the library ever produces is
// `FormatError::IoError` (a failed file read). This test pins that surface so
// a never-constructed `SyntaxErrors`-style variant cannot creep back in: it is
// an *exhaustive* match with a single arm, which fails to compile if any other
// variant exists.

/// `FormatError` exposes exactly one variant — `IoError`. An exhaustive match
/// over it must compile with that single arm; any extra variant breaks it.
#[test]
fn format_error_has_only_io_error_variant() {
    let err = FormatError::IoError(std::io::Error::other("boom"));
    match err {
        FormatError::IoError(e) => assert_eq!(e.kind(), std::io::ErrorKind::Other),
    }
}

/// A syntax-error buffer is the data-preserving passthrough, not an `Err`: the
/// original bytes come back unchanged inside `Ok`. This is the real control
/// flow the (now-removed) dead `SyntaxErrors` arm misrepresented.
#[test]
fn syntax_error_buffer_is_ok_passthrough_not_err() {
    let src = "x = 1\n"; // missing `;` -> syntax error
    let result = format_str(src).expect("syntax errors are Ok, not Err");
    assert_eq!(result.output, src, "original must pass through unchanged");
    assert!(!result.changed, "a passed-through buffer is unchanged");
    // ...and the syntax errors are reported via the separate count pass.
    assert!(
        m1_fmt::syntax_error_count(src) > 0,
        "syntax errors are surfaced via syntax_error_count, not FormatError"
    );
}
