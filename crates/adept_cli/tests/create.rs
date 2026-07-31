//! Integration tests for `adept create`'s CLI surface (binary-level,
//! LLM-free). Anything that needs `create_skill` to actually run is a unit
//! test inside `src/commands/create.rs`, since the real binary would need a
//! live LLM endpoint and cannot be handed a `MockLlmClient`.

mod common;

use predicates::prelude::*;

use common::adept;

#[test]
fn create_without_model_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let brief = tmp.path().join("brief.md");
    std::fs::write(&brief, "Do the thing.\n").unwrap();

    adept()
        .arg("create")
        .arg("--from-file")
        .arg(&brief)
        .arg("--out")
        .arg(tmp.path().join("out"))
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ADEPT_MODEL"));
}

#[test]
fn create_help_lists_all_flags() {
    let assert = adept().arg("create").arg("--help").assert().success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--from-file",
        "--out",
        "--name",
        "--write",
        "--overwrite",
        "--model",
        "--base-url",
        "--tokenizer",
        "--max-rounds",
        "--format",
    ] {
        assert!(
            stdout.contains(flag),
            "help output missing {flag}:\n{stdout}"
        );
    }
}

#[test]
fn no_from_file_empty_stdin_and_no_terminal_exits_two_naming_both_mechanisms() {
    let tmp = tempfile::tempdir().unwrap();

    adept()
        .arg("create")
        .arg("--out")
        .arg(tmp.path().join("out"))
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--from-file"))
        .stderr(predicate::str::contains("stdin"));
}

#[test]
fn out_dir_with_existing_skill_md_fails_without_overwrite_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("existing-skill");
    std::fs::create_dir_all(&out_dir).unwrap();
    std::fs::write(
        out_dir.join("SKILL.md"),
        "---\nname: existing\ndescription: does a thing already. Use when asked.\n---\nBody.\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(out_dir.join("SKILL.md")).unwrap();

    let brief = tmp.path().join("brief.md");
    std::fs::write(&brief, "Do the thing.\n").unwrap();

    adept()
        .arg("create")
        .arg("--from-file")
        .arg(&brief)
        .arg("--out")
        .arg(&out_dir)
        .env_remove("ADEPT_MODEL")
        .assert()
        .code(2);

    assert_eq!(
        std::fs::read_to_string(out_dir.join("SKILL.md")).unwrap(),
        before,
        "existing SKILL.md must be untouched"
    );
}

#[tokio::test]
async fn generated_skill_passes_real_check_and_fmt_check() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("demo-skill");

    let good = serde_json::json!({
        "name": "demo-skill",
        "description": "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
        "disable_model_invocation": false,
        "body": "# Demo Skill\n\n## Overview\n\nDoes the one thing this skill is for.\n\n## Steps\n\n1. Read the input.\n2. Produce the output.\n",
        "companion_files": [],
    })
    .to_string();
    let cases: Vec<_> = (0..10)
        .map(|i| {
            serde_json::json!({
                "prompt": format!("prompt {i}"),
                "assertions": [{"kind": "contains", "value": "ok"}],
            })
        })
        .collect();
    let eval = serde_json::json!({ "cases": cases }).to_string();

    let mock = adept_agent::MockLlmClient::with_texts(vec![good, eval]);
    let options = adept_agent::CreateOptions::for_model("test-model", adept::Tokenizer::O200kBase);
    let report = adept_agent::create_skill(&mock, "Extract PDF form data", &out_dir, &options)
        .await
        .unwrap();
    assert!(report.is_clean(), "{report:?}");

    for path in report.files.keys() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    adept_agent::write_all_transactionally(&report.files).unwrap();

    adept().arg("check").arg(&out_dir).assert().success();

    adept()
        .arg("fmt")
        .arg("--check")
        .arg(out_dir.join("SKILL.md"))
        .assert()
        .success();
}

#[test]
fn stdin_brief_is_accepted_when_not_a_tty() {
    // Same failure mode as `create_without_model_exits_two`, but the brief
    // comes from stdin instead of --from-file, proving the two input paths
    // are otherwise equivalent (both reach the same "no model" usage error,
    // rather than a "no brief" one).
    let tmp = tempfile::tempdir().unwrap();

    adept()
        .arg("create")
        .arg("--out")
        .arg(tmp.path().join("out"))
        .write_stdin("Do the thing.\n")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ADEPT_MODEL"));
}
