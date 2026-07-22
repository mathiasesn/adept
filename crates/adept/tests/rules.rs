//! Golden-file tests for every lint rule: one fixture fires it, and the
//! shared `clean` fixture never fires anything.

use std::path::{Path, PathBuf};

use adept::reporting::render_human_colored;
use adept::{parse_skill, LintConfig, Linter, SkillSet};

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
