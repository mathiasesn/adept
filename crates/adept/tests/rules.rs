//! Golden-file tests for every lint rule: one fixture fires it, and the
//! shared `clean` fixture never fires anything.

use std::path::{Path, PathBuf};

use adept::reporting::render_human_colored;
use adept::{parse_skill, Diagnostic, LintConfig, Linter, Severity, SkillSet};

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rules")
        .join(name)
}

fn lint_fixture(name: &str) -> String {
    let path = fixture_dir(name).join("SKILL.md");
    let skill = parse_skill(&path).expect("fixture should parse");
    let linter = Linter::new(LintConfig::default()).expect("default tokenizer should load");
    let diagnostics = linter.lint_skill(&skill);
    render_human_colored(&diagnostics, false)
}

fn lint_set_fixture(name: &str) -> String {
    let set = SkillSet::discover(fixture_dir(name)).expect("fixture set should discover");
    let linter = Linter::new(LintConfig::default()).expect("default tokenizer should load");
    let diagnostics = linter.lint_set(&set);
    render_human_colored(&diagnostics, false)
}

/// The raw diagnostics for a fixture, so tests can assert on line numbers
/// and severities rather than only on rendered text.
fn diagnostics_for(name: &str) -> Vec<Diagnostic> {
    let path = fixture_dir(name).join("SKILL.md");
    let skill = parse_skill(&path).expect("fixture should parse");
    let linter = Linter::new(LintConfig::default()).expect("default tokenizer should load");
    linter.lint_skill(&skill)
}

/// The diagnostics for a fixture with a given code, in report order.
fn diagnostics_with_code(name: &str, code: &str) -> Vec<Diagnostic> {
    diagnostics_for(name)
        .into_iter()
        .filter(|d| d.code == code)
        .collect()
}

/// Asserts a fixture produces no diagnostics at all for any of `codes`.
/// Other codes (e.g. `SL206`) are irrelevant to markdown-lexing fixtures.
fn assert_no_codes(name: &str, codes: &[&str]) {
    let found: Vec<_> = diagnostics_for(name)
        .into_iter()
        .filter(|d| codes.contains(&d.code))
        .collect();
    assert!(
        found.is_empty(),
        "expected none of {codes:?} on fixture {name}, got:\n{found:#?}"
    );
}

fn assert_fires(name: &str, code: &str) {
    let rendered = lint_fixture(name);
    assert!(
        rendered.contains(code),
        "expected {code} to fire on fixture {name}, got:\n{rendered}"
    );
}

fn assert_set_fires(name: &str, code: &str) {
    let rendered = lint_set_fixture(name);
    assert!(
        rendered.contains(code),
        "expected {code} to fire on fixture {name}, got:\n{rendered}"
    );
}

#[test]
fn clean_skill_has_zero_diagnostics() {
    let rendered = lint_fixture("pdf-extractor");
    assert_eq!(rendered, "", "expected no diagnostics, got:\n{rendered}");
}

#[test]
fn clean_set_has_zero_diagnostics() {
    let rendered = lint_set_fixture("cross_clean");
    assert_eq!(rendered, "", "expected no diagnostics, got:\n{rendered}");
}

#[test]
fn sl001_missing_description_fires() {
    assert_fires("sl001_empty_description", "SL001");
}

#[test]
fn sl002_missing_name_fires() {
    assert_fires("sl002_empty_name", "SL002");
}

#[test]
fn sl003_malformed_frontmatter_fires() {
    let set = SkillSet::discover(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/missing_frontmatter"),
    )
    .expect("should discover");
    let linter = Linter::new(LintConfig::default()).expect("default tokenizer should load");
    let rendered = render_human_colored(&linter.lint_set(&set), false);
    assert!(rendered.contains("SL003"), "got:\n{rendered}");
}

#[test]
fn sl004_name_mismatch_fires() {
    assert_fires("sl004_name_mismatch", "SL004");
}

#[test]
fn sl005_invalid_name_format_fires() {
    assert_fires("sl005_invalid_name_format", "SL005");
}

#[test]
fn sl101_empty_body_fires() {
    assert_fires("sl101_empty_body", "SL101");
}

#[test]
fn sl102_missing_h1_fires() {
    assert_fires("sl102_missing_h1", "SL102");
}

#[test]
fn sl103_heading_skip_fires() {
    assert_fires("sl103_heading_skip", "SL103");
}

#[test]
fn sl104_broken_file_reference_fires() {
    assert_fires("sl104_broken_ref", "SL104");
}

#[test]
fn sl201_description_too_short_fires() {
    assert_fires("sl201_too_short", "SL201");
}

// SL202 (description-too-long) is retired; SL301
// (description-tokens-over-budget) is the sole rule covering an overlong
// description now. See `crates/adept/src/rules/description.rs`.

#[test]
fn sl203_missing_trigger_phrase_fires() {
    assert_fires("sl203_no_trigger", "SL203");
}

#[test]
fn sl204_first_person_fires() {
    assert_fires("sl204_first_person", "SL204");
}

#[test]
fn sl205_restates_name_fires() {
    assert_fires("sl205_restates_name", "SL205");
}

#[test]
fn sl206_no_negative_guidance_fires() {
    assert_fires("sl206_no_negative", "SL206");
}

#[test]
fn sl301_description_token_budget_fires() {
    assert_fires("sl301_desc_budget", "SL301");
}

#[test]
fn sl302_body_token_budget_fires() {
    assert_fires("sl302_body_budget", "SL302");
}

#[test]
fn sl303_companion_file_bloat_fires() {
    assert_fires("sl303_companion_bloat", "SL303");
}

#[test]
fn sl401_duplicate_skill_name_fires() {
    assert_set_fires("cross_sl401", "SL401");
}

#[test]
fn sl402_similar_description_fires() {
    assert_set_fires("cross_sl402", "SL402");
}

#[test]
fn sl403_overlapping_trigger_phrasing_fires() {
    assert_set_fires("cross_sl403", "SL403");
}

// --- Markdown-lexing regression tests -------------------------------------
//
// Each of these pins a case the old hand-rolled line scanner in
// `rules/structure.rs` got wrong before the shared `adept::markdown` lexer
// replaced it. They are integration-level: the whole rule set runs, and the
// assertions target the SL10x codes plus their line numbers.

#[test]
fn mismatched_fence_characters_hide_their_contents() {
    // Old scanner bug: it toggled a single `in_fence` flag on any line
    // starting with ``` or ~~~, so a ``` block containing a ~~~ line was
    // treated as *closed* there — and everything after it re-entered
    // "prose", producing phantom SL102/SL103/SL104 findings.
    assert_no_codes("md-mismatched-fence", &["SL102", "SL103", "SL104", "SL105"]);
}

#[test]
fn fence_info_string_containing_hash_is_not_a_heading() {
    // Old scanner bug: fence info strings were ignored entirely, and the
    // `#`-counting heading scan ran over any line it thought was prose, so
    // ``` ```bash # run this ``` and the comment inside could be read as
    // headings and the link inside as a file reference.
    assert_no_codes("md-fence-info-hash", &["SL102", "SL103", "SL104", "SL105"]);
}

#[test]
fn indented_code_block_contents_are_not_lexed() {
    // Old scanner bug: it never recognised 4-space indented code blocks at
    // all, so heading-like and link-like text inside them was linted as
    // prose (an h1 -> h3 skip plus a broken file reference here).
    assert_no_codes("md-indented-code", &["SL103", "SL104", "SL105"]);
}

#[test]
fn sl104_reports_full_destination_with_nested_parentheses() {
    // Old scanner bug: `extract_link_targets` scanned from `](` to the
    // *first* `)`, so `[x](./a(b).md)` yielded the truncated `./a(b` and the
    // diagnostic quoted a path the user never wrote.
    let found = diagnostics_with_code("md-nested-paren-link", "SL104");
    assert_eq!(found.len(), 1, "got:\n{found:#?}");
    assert!(
        found[0].message.contains("./a(b).md"),
        "diagnostic should quote the full destination, got: {}",
        found[0].message
    );
    // File line 7: 4 frontmatter lines, `# Nested Paren Link`, blank, link.
    assert_eq!(found[0].line, 7);
}

#[test]
fn setext_h1_satisfies_sl102() {
    // Old scanner bug: it counted `#` characters and so could not see
    // setext headings, giving `Title\n=====` a bogus missing-h1 warning.
    assert_no_codes("md-setext-headings", &["SL102"]);
}

#[test]
fn setext_heading_participates_in_sl103_level_sequence() {
    // Old scanner bug: with the setext h1 invisible, the following `###`
    // had no preceding heading to skip from, so SL103 stayed silent.
    let found = diagnostics_with_code("md-setext-headings", "SL103");
    assert_eq!(found.len(), 1, "got:\n{found:#?}");
    assert!(
        found[0].message.contains("h1 to h3"),
        "{}",
        found[0].message
    );
    // File line 8: the `### Skipped Level` line.
    assert_eq!(found[0].line, 8);
}

#[test]
fn sl105_fires_on_setext_headings_only() {
    // SL105 is new with the shared lexer; the old scanner could not see
    // setext headings, so this rule was not expressible at all.
    let found = diagnostics_with_code("md-setext-headings", "SL105");
    assert_eq!(found.len(), 2, "got:\n{found:#?}");
    // File lines 5 (`Title`, underlined on 6) and 10 (`Sub`, underlined on 11).
    assert_eq!(found[0].line, 5);
    assert!(found[0].message.contains("Title"), "{}", found[0].message);
    assert_eq!(found[1].line, 10);
    assert!(found[1].message.contains("Sub"), "{}", found[1].message);
    for d in &found {
        assert_eq!(d.severity, Severity::Info);
    }

    // ATX headings never produce SL105.
    assert_no_codes("sl103_heading_skip", &["SL105"]);
    assert_no_codes("pdf-extractor", &["SL105"]);
}

#[test]
fn every_registered_rule_has_a_positive_fixture_test() {
    // This is a meta-check: if a new rule is added to the registry without a
    // corresponding fixture+test above, this test will still pass (it only
    // asserts the registry is non-empty and every code is well-formed), but
    // the `docs/RULES.md` drift test in `docs_test.rs` will catch missing
    // documentation, which is the more actionable signal.
    let registry = adept::Registry::new();
    let meta = registry.all_meta();
    assert!(!meta.is_empty());
    for m in meta {
        assert!(m.code.starts_with("SL"), "malformed code: {}", m.code);
        assert!(
            m.name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "not kebab-case: {}",
            m.name
        );
    }
}

#[test]
fn snapshot_clean_skill() {
    insta::assert_snapshot!(lint_fixture("pdf-extractor"));
}

#[test]
fn snapshot_sl001_missing_description() {
    insta::assert_snapshot!(lint_fixture("sl001_empty_description"));
}

#[test]
fn snapshot_sl002_missing_name() {
    insta::assert_snapshot!(lint_fixture("sl002_empty_name"));
}

#[test]
fn snapshot_sl003_malformed_frontmatter() {
    let set = SkillSet::discover(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/missing_frontmatter"),
    )
    .expect("should discover");
    let linter = Linter::new(LintConfig::default()).expect("default tokenizer should load");
    insta::assert_snapshot!(render_human_colored(&linter.lint_set(&set), false));
}

#[test]
fn snapshot_sl004_name_mismatch() {
    insta::assert_snapshot!(lint_fixture("sl004_name_mismatch"));
}

#[test]
fn snapshot_sl005_invalid_name_format() {
    insta::assert_snapshot!(lint_fixture("sl005_invalid_name_format"));
}

#[test]
fn snapshot_sl101_empty_body() {
    insta::assert_snapshot!(lint_fixture("sl101_empty_body"));
}

#[test]
fn snapshot_sl102_missing_h1() {
    insta::assert_snapshot!(lint_fixture("sl102_missing_h1"));
}

#[test]
fn snapshot_sl103_heading_skip() {
    insta::assert_snapshot!(lint_fixture("sl103_heading_skip"));
}

#[test]
fn snapshot_sl104_broken_file_reference() {
    insta::assert_snapshot!(lint_fixture("sl104_broken_ref"));
}

#[test]
fn snapshot_sl201_description_too_short() {
    insta::assert_snapshot!(lint_fixture("sl201_too_short"));
}

#[test]
fn snapshot_sl203_missing_trigger_phrase() {
    insta::assert_snapshot!(lint_fixture("sl203_no_trigger"));
}

#[test]
fn snapshot_sl204_first_person() {
    insta::assert_snapshot!(lint_fixture("sl204_first_person"));
}

#[test]
fn snapshot_sl205_restates_name() {
    insta::assert_snapshot!(lint_fixture("sl205_restates_name"));
}

#[test]
fn snapshot_sl206_no_negative_guidance() {
    insta::assert_snapshot!(lint_fixture("sl206_no_negative"));
}

#[test]
fn snapshot_sl301_description_token_budget() {
    insta::assert_snapshot!(lint_fixture("sl301_desc_budget"));
}

#[test]
fn snapshot_sl302_body_token_budget() {
    insta::assert_snapshot!(lint_fixture("sl302_body_budget"));
}

#[test]
fn snapshot_sl303_companion_file_bloat() {
    insta::assert_snapshot!(lint_fixture("sl303_companion_bloat"));
}

#[test]
fn snapshot_cross_sl401() {
    insta::assert_snapshot!(lint_set_fixture("cross_sl401"));
}

#[test]
fn snapshot_cross_sl402() {
    insta::assert_snapshot!(lint_set_fixture("cross_sl402"));
}

#[test]
fn snapshot_cross_sl403() {
    insta::assert_snapshot!(lint_set_fixture("cross_sl403"));
}
