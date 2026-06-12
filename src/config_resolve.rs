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

use m1_fmt::FormatOptions;
use std::path::Path;

/// Explicit CLI flag overrides (highest precedence). `None` means "not given on
/// the command line", so the resolved value comes from config or the default.
#[derive(Debug, Default, Clone, Copy)]
pub struct CliOverrides {
    pub max_blank_lines: Option<usize>,
    pub line_width: Option<usize>,
}

/// Resolve [`FormatOptions`] for a file/stdin in `dir`, layering lowest-first:
/// built-in defaults → the unified `m1-tools.toml` `[format]` section → the
/// tool-specific `.m1fmt.toml` (overrides the unified file) → CLI flags. Both
/// config files are discovered by walking up from `dir`.
pub fn resolve_opts(overrides: CliOverrides, dir: &Path) -> FormatOptions {
    let mut o = FormatOptions::default();

    // Layer 1: the unified m1-tools.toml [format] section (lowest config layer).
    if let Some(tc) = m1_workspace::config::M1ToolsConfig::discover(dir) {
        let f = tc.format;
        if let Some(n) = f.line_width {
            o.line_width = n;
        }
        if let Some(n) = f.max_blank_lines {
            o.max_blank_lines = n;
        }
        if let Some(n) = f.indent_width {
            o.indent_width = n;
        }
        if let Some(s) = f
            .indent_style
            .as_deref()
            .and_then(m1_fmt::config::parse_indent_style)
        {
            o.indent_style = s;
        }
        if let Some(s) = f
            .brace_style
            .as_deref()
            .and_then(m1_fmt::config::parse_brace_style)
        {
            o.brace_style = s;
        }
        // The continuation/align/reflow knobs reached the unified [format]
        // section in m1-workspace v0.9.0 (#25); previously they were
        // .m1fmt.toml-only, so a unified-config-only team couldn't set them.
        if let Some(n) = f.continuation_indent {
            o.continuation_indent = n;
        }
        if let Some(b) = f.align_assignments {
            o.align_assignments = b;
        }
        if let Some(b) = f.reflow_comments {
            o.reflow_comments = b;
        }
        // final_blank_line reached the unified [format] section in
        // m1-workspace v0.10.0 (#116's L027-pairing knob).
        if let Some(b) = f.final_blank_line {
            o.final_blank_line = b;
        }
    }

    // Layer 2: the tool-specific .m1fmt.toml overrides the unified file.
    if let Some(cfg) = m1_fmt::config::discover(dir) {
        if let Some(n) = cfg.max_line_length {
            o.line_width = n;
        }
        if let Some(n) = cfg.max_blank_lines {
            o.max_blank_lines = n;
        }
        if let Some(n) = cfg.indent_width {
            o.indent_width = n;
        }
        if let Some(s) = cfg.indent_style {
            o.indent_style = s;
        }
        if let Some(s) = cfg.brace_style {
            o.brace_style = s;
        }
        if let Some(n) = cfg.continuation_indent {
            o.continuation_indent = n;
        }
        if let Some(b) = cfg.align_assignments {
            o.align_assignments = b;
        }
        if let Some(b) = cfg.reflow_comments {
            o.reflow_comments = b;
        }
        if let Some(b) = cfg.final_blank_line {
            o.final_blank_line = b;
        }
    }

    // Layer 3: explicit CLI flags win over everything.
    if let Some(n) = overrides.max_blank_lines {
        o.max_blank_lines = n;
    }
    if let Some(n) = overrides.line_width {
        o.line_width = n;
    }
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
}
