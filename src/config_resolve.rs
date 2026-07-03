//! Format-option precedence resolution for the CLI.
//!
//! Layers the configuration sources lowest-first into a [`FormatOptions`]:
//! built-in defaults → the unified `m1-tools.toml` `[format]` section → the
//! tool-specific `.m1fmt.toml` (overrides the unified file) → CLI flags. Both
//! config files are discovered by walking up from the target directory.
//!
//! Kept independent of the clap `Args` struct: the CLI overrides are passed as
//! plain `Option`s so the precedence logic can be tested without constructing a
//! full argument set.

use m1_fmt::{BraceStyle, FormatOptions, IndentStyle};
use std::path::Path;

/// Upper bound for the indent-width knobs (`indent_width`, `continuation_indent`).
///
/// These feed the wrapping column math — `visual_width` sums `indent_width` per
/// tab and the last-resort assignment break computes
/// `(indent + continuation_indent) * indent_width`. Config files accept any
/// non-negative integer up to `i64::MAX` (`config.rs` `uint`) and the CLI any
/// `usize`, so a pathologically large value overflows those arithmetic ops and
/// panics (debug) / wraps (release). No realistic configuration needs more than
/// a handful of columns, so we cap both at a small sane maximum at
/// option-resolution time. For tabs, `indent_width` is only the *assumed* display
/// width used for wrap math, so a cap of 64 is harmless to real output.
pub use m1_fmt::config::MAX_INDENT_WIDTH;

/// Cap `value` at [`MAX_INDENT_WIDTH`], emitting a one-line stderr warning naming
/// `knob` when it was out of range, so the user learns the input was clamped
/// rather than silently formatting with a different width.
fn clamp_indent(knob: &str, value: usize) -> usize {
    if value > MAX_INDENT_WIDTH {
        eprintln!(
            "m1-fmt: {knob} {value} exceeds the maximum of {MAX_INDENT_WIDTH}; clamped to {MAX_INDENT_WIDTH}"
        );
        MAX_INDENT_WIDTH
    } else {
        value
    }
}

/// Explicit CLI flag overrides (highest precedence). `None` means "not given on
/// the command line", so the resolved value comes from config or the default.
///
/// The two enum knobs are carried as the already-parsed [`IndentStyle`] /
/// [`BraceStyle`] (not raw strings): `main` validates the flag values eagerly so
/// a bad spelling is a usage error (exit 2) before any formatting runs, the same
/// as `--range` / `--jobs`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CliOverrides {
    pub max_blank_lines: Option<usize>,
    pub line_width: Option<usize>,
    pub indent_style: Option<IndentStyle>,
    pub brace_style: Option<BraceStyle>,
    pub indent_width: Option<usize>,
    pub continuation_indent: Option<usize>,
    /// Opt-in booleans (#96 / #95 / #116). `Some(true)` forces the option on,
    /// `Some(false)` forces it off (the `--no-…` flag), `None` defers to config.
    pub align_assignments: Option<bool>,
    pub reflow_comments: Option<bool>,
    pub final_blank_line: Option<bool>,
}

/// Resolve [`FormatOptions`] for a file/stdin in `dir`, layering lowest-first:
/// built-in defaults → the unified `m1-tools.toml` `[format]` section → the
/// tool-specific `.m1fmt.toml` (overrides the unified file) → CLI flags. Both
/// config files are discovered by walking up from `dir`.
pub fn resolve_opts(overrides: CliOverrides, dir: &Path) -> FormatOptions {
    // Layers 1 (unified m1-tools.toml) + 2 (.m1fmt.toml) come from the shared
    // library resolver, so the CLI, the LSP and the MCP server all agree on the
    // config-file layering. This function adds only the CLI-flag layer on top.
    let mut o = m1_fmt::config::resolve_options(dir);

    // Layer 3: explicit CLI flags win over everything.
    if let Some(n) = overrides.max_blank_lines {
        o.max_blank_lines = n;
    }
    if let Some(n) = overrides.line_width {
        o.line_width = n;
    }
    if let Some(s) = overrides.indent_style {
        o.indent_style = s;
    }
    if let Some(s) = overrides.brace_style {
        o.brace_style = s;
    }
    if let Some(n) = overrides.indent_width {
        o.indent_width = n;
    }
    if let Some(n) = overrides.continuation_indent {
        o.continuation_indent = n;
    }
    if let Some(b) = overrides.align_assignments {
        o.align_assignments = b;
    }
    if let Some(b) = overrides.reflow_comments {
        o.reflow_comments = b;
    }
    if let Some(b) = overrides.final_blank_line {
        o.final_blank_line = b;
    }

    // Cap the width knobs to a sane maximum after all layering. A pathologically
    // large value (from a committed config file or a stray CLI flag) would
    // otherwise overflow the wrapping column math and panic/wrap. The cap is
    // applied once, here, instead of scattering saturating ops through the
    // printer.
    o.indent_width = clamp_indent("indent_width", o.indent_width);
    o.continuation_indent = clamp_indent("continuation_indent", o.continuation_indent);
    o
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    #[test]
    fn unified_tools_toml_drives_brace_style() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nbrace_style = \"kr\"\nindent_style = \"spaces\"\nindent_width = 2\nline_width = 100\n",
        )
        .unwrap();
        let o = resolve_opts(CliOverrides::default(), tmp.path());
        assert_eq!(o.brace_style, m1_fmt::BraceStyle::Kr);
        assert_eq!(o.indent_style, m1_fmt::IndentStyle::Spaces);
        assert_eq!(o.indent_width, 2);
        assert_eq!(o.line_width, 100);
    }

    #[test]
    fn unified_tools_toml_drives_continuation_align_reflow() {
        // #25 cascade: the continuation/align/reflow knobs are settable from the
        // unified m1-tools.toml [format] section now, not just .m1fmt.toml.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\ncontinuation_indent = 3\nalign_assignments = true\nreflow_comments = true\n",
        )
        .unwrap();
        let o = resolve_opts(CliOverrides::default(), tmp.path());
        assert_eq!(o.continuation_indent, 3);
        assert!(o.align_assignments);
        assert!(o.reflow_comments);
    }

    #[test]
    fn m1fmt_toml_drives_final_blank_line() {
        // #116: settable from .m1fmt.toml AND the unified [format] section
        // (FormatSection::final_blank_line, m1-workspace v0.10.0).
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".m1fmt.toml"), "final_blank_line = true\n").unwrap();
        let o = resolve_opts(CliOverrides::default(), tmp.path());
        assert!(o.final_blank_line);
        assert!(!m1_fmt::FormatOptions::default().final_blank_line, "opt-in");

        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp2.path().join("m1-tools.toml"),
            "[format]\nfinal_blank_line = true\n",
        )
        .unwrap();
        let o2 = resolve_opts(CliOverrides::default(), tmp2.path());
        assert!(o2.final_blank_line, "unified [format] key drives it too");
    }

    #[test]
    fn m1fmt_toml_overrides_unified_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nbrace_style = \"kr\"\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".m1fmt.toml"), "brace_style = \"allman\"\n").unwrap();
        let o = resolve_opts(CliOverrides::default(), tmp.path());
        assert_eq!(
            o.brace_style,
            m1_fmt::BraceStyle::Allman,
            ".m1fmt.toml wins"
        );
    }

    #[test]
    fn flag_overrides_both() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nline_width = 100\n",
        )
        .unwrap();
        let o = resolve_opts(
            CliOverrides {
                line_width: Some(70),
                ..Default::default()
            },
            tmp.path(),
        );
        assert_eq!(o.line_width, 70);
    }

    #[test]
    fn style_flags_override_both_config_files() {
        // The manual-mandated style knobs (and the width knobs) must be settable
        // by CLI flag, winning over both config files. Both files set every knob;
        // the CLI overrides flip each one to a different value.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nindent_style = \"tab\"\nbrace_style = \"allman\"\nindent_width = 4\ncontinuation_indent = 1\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".m1fmt.toml"),
            "indent_style = \"tab\"\nbrace_style = \"allman\"\nindent_width = 4\ncontinuation_indent = 1\n",
        )
        .unwrap();
        let o = resolve_opts(
            CliOverrides {
                indent_style: Some(IndentStyle::Spaces),
                brace_style: Some(BraceStyle::Kr),
                indent_width: Some(2),
                continuation_indent: Some(3),
                ..Default::default()
            },
            tmp.path(),
        );
        assert_eq!(o.indent_style, IndentStyle::Spaces, "flag beats config");
        assert_eq!(o.brace_style, BraceStyle::Kr, "flag beats config");
        assert_eq!(o.indent_width, 2, "flag beats config");
        assert_eq!(o.continuation_indent, 3, "flag beats config");
    }

    #[test]
    fn bool_flags_override_config_both_ways() {
        // The opt-in booleans (align/reflow/final_blank) must be CLI-settable in
        // both directions: a flag can force one ON when config is silent, and a
        // `--no-` flag (Some(false)) can force one OFF when config turned it on.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nalign_assignments = true\nreflow_comments = true\nfinal_blank_line = true\n",
        )
        .unwrap();
        let o = resolve_opts(
            CliOverrides {
                align_assignments: Some(false),
                reflow_comments: Some(false),
                final_blank_line: Some(false),
                ..Default::default()
            },
            tmp.path(),
        );
        assert!(!o.align_assignments, "--no- flag beats config true");
        assert!(!o.reflow_comments, "--no- flag beats config true");
        assert!(!o.final_blank_line, "--no- flag beats config true");

        // And ON over a silent config (default false).
        let empty = tempfile::tempdir().unwrap();
        let o2 = resolve_opts(
            CliOverrides {
                align_assignments: Some(true),
                reflow_comments: Some(true),
                final_blank_line: Some(true),
                ..Default::default()
            },
            empty.path(),
        );
        assert!(o2.align_assignments && o2.reflow_comments && o2.final_blank_line);

        // Absent (None) must not clobber a config that enabled them.
        let o3 = resolve_opts(CliOverrides::default(), tmp.path());
        assert!(o3.align_assignments && o3.reflow_comments && o3.final_blank_line);
    }

    #[test]
    fn pathological_width_knobs_are_clamped() {
        // A committed .m1fmt.toml (config.rs `uint` accepts any non-negative
        // integer up to i64::MAX) with absurd indent widths must be capped at
        // resolution time so the wrapping column math can't overflow downstream.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".m1fmt.toml"),
            "indent_width = 9223372036854775807\ncontinuation_indent = 9223372036854775807\n",
        )
        .unwrap();
        let o = resolve_opts(CliOverrides::default(), tmp.path());
        assert_eq!(o.indent_width, MAX_INDENT_WIDTH);
        assert_eq!(o.continuation_indent, MAX_INDENT_WIDTH);

        // A CLI flag (any usize) is clamped too.
        let empty = tempfile::tempdir().unwrap();
        let o2 = resolve_opts(
            CliOverrides {
                indent_width: Some(usize::MAX),
                continuation_indent: Some(usize::MAX),
                ..Default::default()
            },
            empty.path(),
        );
        assert_eq!(o2.indent_width, MAX_INDENT_WIDTH);
        assert_eq!(o2.continuation_indent, MAX_INDENT_WIDTH);
    }

    #[test]
    fn sane_width_knobs_pass_through_unclamped() {
        // Values within range are untouched (no spurious clamping/warning).
        let o = resolve_opts(
            CliOverrides {
                indent_width: Some(4),
                continuation_indent: Some(2),
                ..Default::default()
            },
            tempfile::tempdir().unwrap().path(),
        );
        assert_eq!(o.indent_width, 4);
        assert_eq!(o.continuation_indent, 2);
    }

    #[test]
    fn absent_style_flags_fall_through_to_config() {
        // A `None` style override must not clobber the config value (the
        // `if let Some` guard); only an explicit flag wins.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".m1fmt.toml"),
            "indent_style = \"spaces\"\nbrace_style = \"kr\"\n",
        )
        .unwrap();
        let o = resolve_opts(CliOverrides::default(), tmp.path());
        assert_eq!(o.indent_style, IndentStyle::Spaces);
        assert_eq!(o.brace_style, BraceStyle::Kr);
    }
}
