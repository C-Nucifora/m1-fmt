//! Regression tests for specific reported defects.
//!
//! - #125: a bare-identifier `local` initializer must not gain a double space.
//! - #126: an own-line comment before `else`/`else if` must stay above it.
//! - #127: a long assignment with no internal break point must break after `=`.

use m1_fmt::{FormatOptions, format_str, format_str_with};

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
