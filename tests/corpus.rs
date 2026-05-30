mod common;

#[test]
fn corpus_no_crash_and_output_reparses() {
    let scripts = common::corpus_scripts();
    assert!(!scripts.is_empty(), "no corpus scripts found");

    for (path, src) in &scripts {
        let input_diags = m1_core::parse(src).syntax_diagnostics();

        // Should never panic
        let result = m1_fmt::format_str(src);

        if input_diags.is_empty() {
            let fmt_output = result
                .unwrap_or_else(|e| panic!("{}: format_str returned Err: {}", path.display(), e))
                .output;

            let output_diags = m1_core::parse(&fmt_output).syntax_diagnostics();
            assert!(
                output_diags.is_empty(),
                "{}: formatted output has {} syntax error(s): {:?}",
                path.display(),
                output_diags.len(),
                output_diags
            );
        } else {
            // Files with syntax errors: formatter should pass through unchanged
            let fmt_output = result
                .expect("should not error on syntax-error input")
                .output;
            assert_eq!(
                src, &fmt_output,
                "{}: syntax-error file was not passed through unchanged",
                path.display()
            );
        }
    }
}
