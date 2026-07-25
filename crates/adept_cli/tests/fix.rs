//! Integration tests for `adept fix`.
//!
//! LLM-free assertions drive the compiled `adept` binary directly. Anything
//! that needs `fix_skill` to actually run drives it in-process against
//! `adept_score::MockLlmClient`, since the real binary would need a live
//! LLM endpoint.

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn adept() -> Command {
    Command::cargo_bin("adept").unwrap()
}

/// Copy a fixture directory into a fresh temp dir so tests that exercise
/// writes never mutate the checked-in fixtures.
fn copy_fixture_to_tempdir(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = fixture(name);
    for entry in std::fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        let dest = dir.path().join(entry.file_name());
        std::fs::copy(entry.path(), dest).unwrap();
    }
    dir
}

#[test]
fn fix_without_model_exits_two() {
    adept()
        .arg("fix")
        .arg(fixture("clean-skill"))
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ADEPT_MODEL"));
}

#[test]
fn fix_help_lists_all_flags() {
    let assert = adept().arg("fix").arg("--help").assert().success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--write",
        "--check",
        "--diff",
        "--select",
        "--ignore",
        "--model",
        "--base-url",
        "--max-rounds",
        "--tokenizer",
        "--format",
    ] {
        assert!(
            stdout.contains(flag),
            "help output missing {flag}:\n{stdout}"
        );
    }
}

#[test]
fn fix_write_and_check_are_mutually_exclusive() {
    adept()
        .arg("fix")
        .arg(fixture("clean-skill"))
        .arg("--write")
        .arg("--check")
        .assert()
        .failure();
}

// --- In-process tests driving fix_skill/MockLlmClient directly ---

fn read_fixture_source(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("SKILL.md")).unwrap()
}

#[tokio::test]
async fn preview_computes_but_writes_nothing() {
    let dir = copy_fixture_to_tempdir("over-budget-description");
    let path = dir.path().join("SKILL.md");
    let before = read_fixture_source(dir.path());

    let skill = adept::parse_skill(&path).unwrap();
    let short_description =
        "Extracts data from PDF forms and documents. Do not use for scanned image-only PDFs.";
    let mock = adept_score::MockLlmClient::with_texts(vec![format!(
        r#"{{"description": "{short_description}"}}"#
    )]);
    let options = adept_fix::FixOptions::for_model("test-model", adept::Tokenizer::O200kBase);

    let report = adept_fix::fix_skill(&mock, &skill, &[], &options)
        .await
        .unwrap();
    assert!(report.accepted, "{report:?}");
    assert!(!report.files.is_empty());

    // Preview never writes: the on-disk file must be untouched.
    let after = read_fixture_source(dir.path());
    assert_eq!(before, after);
}

#[tokio::test]
async fn write_all_transactionally_clears_fixable_findings() {
    let dir = copy_fixture_to_tempdir("over-budget-description");
    let path = dir.path().join("SKILL.md");

    let skill = adept::parse_skill(&path).unwrap();
    let short_description =
        "Extracts data from PDF forms and documents. Do not use for scanned image-only PDFs.";
    let mock = adept_score::MockLlmClient::with_texts(vec![format!(
        r#"{{"description": "{short_description}"}}"#
    )]);
    let options = adept_fix::FixOptions::for_model("test-model", adept::Tokenizer::O200kBase);

    let report = adept_fix::fix_skill(&mock, &skill, &[], &options)
        .await
        .unwrap();
    assert!(report.accepted, "{report:?}");

    adept_fix::write_all_transactionally(&report.files).unwrap();

    // Re-lint the written result: SL301/SL206 must be gone.
    let rewritten = adept::parse_skill(&path).unwrap();
    let linter = adept::Linter::new(options.lint_config.clone()).unwrap();
    let diagnostics = linter.lint_skill(&rewritten);
    assert!(!diagnostics.iter().any(|d| d.code == "SL301"));
    assert!(!diagnostics.iter().any(|d| d.code == "SL206"));
}

#[tokio::test]
async fn check_equivalent_reports_pending_changes_when_over_budget() {
    let dir = copy_fixture_to_tempdir("over-budget-body");
    let path = dir.path().join("SKILL.md");
    let skill = adept::parse_skill(&path).unwrap();

    let description_response = serde_json::json!({
        "description": "Does a thing. Use when the user needs this thing done reliably. Do not use for anything else."
    })
    .to_string();
    let short_body = "# Over Budget Body\n\nSee REFERENCE.md for details.\n";
    let relocated = "word ".repeat(2500);
    let body_response = serde_json::json!({
        "body": short_body,
        "companion_edits": [
            {"path": "REFERENCE.md", "appended_content": relocated}
        ]
    })
    .to_string();
    let mock = adept_score::MockLlmClient::with_texts(vec![description_response, body_response]);
    let options = adept_fix::FixOptions::for_model("test-model", adept::Tokenizer::O200kBase);

    let report = adept_fix::fix_skill(&mock, &skill, &[], &options)
        .await
        .unwrap();
    // Pending changes: files non-empty is the --check-equivalent "would
    // change" signal, i.e. exit 1.
    assert!(!report.files.is_empty());
}

#[tokio::test]
async fn check_equivalent_reports_clean_on_already_clean_skill() {
    let path = fixture("clean-skill").join("SKILL.md");
    let skill = adept::parse_skill(&path).unwrap();
    let mock = adept_score::MockLlmClient::with_texts(Vec::<String>::new());
    let options = adept_fix::FixOptions::for_model("test-model", adept::Tokenizer::O200kBase);

    let report = adept_fix::fix_skill(&mock, &skill, &[], &options)
        .await
        .unwrap();
    assert!(report.files.is_empty());
    assert!(!report.accepted);
}

#[tokio::test]
async fn rejected_rewrite_leaves_original_file_unchanged() {
    let dir = copy_fixture_to_tempdir("over-budget-body");
    let path = dir.path().join("SKILL.md");
    let before = read_fixture_source(dir.path());
    let skill = adept::parse_skill(&path).unwrap();

    // Still-over-budget rewrite every round: the model never actually
    // shrinks the fixable set.
    let still_long_body = format!("# Over Budget Body\n\n{}", "word ".repeat(2400));
    let response = serde_json::json!({ "body": still_long_body }).to_string();
    let mock = adept_score::MockLlmClient::with_texts(vec![response.clone(), response]);
    let options = adept_fix::FixOptions::for_model("test-model", adept::Tokenizer::O200kBase);

    let report = adept_fix::fix_skill(&mock, &skill, &[], &options)
        .await
        .unwrap();
    assert!(!report.accepted, "{report:?}");
    assert!(report.files.is_empty());

    // Nothing was written by fix_skill itself; the file on disk must be
    // byte-for-byte unchanged.
    let after = read_fixture_source(dir.path());
    assert_eq!(before, after);
}
