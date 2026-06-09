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
