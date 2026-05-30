use std::path::Path;

fn run_snapshot(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let input_path = dir.join(format!("{}.m1scr", name));
    let expected_path = dir.join(format!("{}.expected", name));

    let input = std::fs::read_to_string(&input_path)
        .unwrap_or_else(|_| panic!("missing snapshot input: {}", input_path.display()));
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|_| panic!("missing snapshot expected: {}", expected_path.display()));

    let result = m1_fmt::format_str(&input).expect("format_str failed");
    assert_eq!(
        result.output, expected,
        "snapshot mismatch for {}\n--- expected ---\n{}\n--- actual ---\n{}",
        name, expected, result.output
    );
}

#[test]
fn test_operator_spacing() {
    run_snapshot("operator_spacing");
}
#[test]
fn test_brace_placement() {
    run_snapshot("brace_placement");
}
#[test]
fn test_comment_eol() {
    run_snapshot("comment_eol");
}
#[test]
fn test_comment_own_line() {
    run_snapshot("comment_own_line");
}
#[test]
fn test_keyword_paren_spacing() {
    run_snapshot("keyword_paren_spacing");
}
#[test]
fn test_identifier_internal_spaces() {
    run_snapshot("identifier_internal_spaces");
}
#[test]
fn test_expand_interpolation() {
    run_snapshot("expand_interpolation");
}
#[test]
fn test_trailing_whitespace() {
    run_snapshot("trailing_whitespace");
}
#[test]
fn test_final_newline() {
    run_snapshot("final_newline");
}
