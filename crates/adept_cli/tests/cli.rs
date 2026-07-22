//! CLI integration tests, driving the built `adept` binary via `assert_cmd`.

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

#[test]
fn check_on_clean_skill_exits_zero() {
    adept()
        .arg("check")
        .arg(fixture("clean-skill"))
        .assert()
        .success();
}

#[test]
fn check_on_defective_skill_exits_one_and_names_rule_code() {
    adept()
        .arg("check")
        .arg(fixture("defective-skill"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains("SL102"));
}

#[test]
fn check_format_json_emits_valid_json() {
    let output = adept()
        .arg("check")
        .arg(fixture("defective-skill"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("output was not valid JSON: {err}\n{stdout}"));
    assert!(parsed.is_array());
    assert!(!parsed.as_array().unwrap().is_empty());
}

#[test]
fn check_exit_zero_flag_forces_zero() {
    adept()
        .arg("check")
        .arg(fixture("defective-skill"))
        .arg("--exit-zero")
        .assert()
        .success();
}

#[test]
fn check_unreadable_path_exits_two() {
    adept()
        .arg("check")
        .arg(fixture("does_not_exist"))
        .assert()
        .code(2);
}

#[test]
fn check_select_only_runs_selected_rule() {
    // sl102_missing_h1 in the core crate's own fixtures also trips SL203 and
    // SL206; --select SL102 should suppress those.
    let output = adept()
        .arg("check")
        .arg(fixture("defective-skill"))
        .arg("--select")
        .arg("SL102")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let codes: Vec<&str> = parsed
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"SL102"));
    assert!(codes.iter().all(|c| *c == "SL102"));
}

#[test]
fn check_statistics_prints_counts() {
    adept()
        .arg("check")
        .arg(fixture("defective-skill"))
        .arg("--statistics")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Statistics:"));
}

#[test]
fn fmt_check_exits_one_on_unformatted_input() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("SKILL.md"),
        "---\nname: unformatted\ndescription: a description here that is long enough to pass\n---\nBody   text.\n",
    )
    .unwrap();

    adept()
        .arg("fmt")
        .arg(dir.path())
        .arg("--check")
        .assert()
        .code(1);
}

#[test]
fn fmt_check_exits_zero_on_already_formatted_input() {
    adept()
        .arg("fmt")
        .arg(fixture("clean-skill"))
        .arg("--check")
        .assert()
        .success();
}

#[test]
fn fmt_in_place_rewrites_file_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    std::fs::write(
        &path,
        "---\nname: unformatted\ndescription: a description here that is long enough to pass\n---\nBody   text.\n",
    )
    .unwrap();

    adept().arg("fmt").arg(dir.path()).assert().success();
    let once = std::fs::read_to_string(&path).unwrap();
    assert_ne!(once, "");

    adept().arg("fmt").arg(dir.path()).assert().success();
    let twice = std::fs::read_to_string(&path).unwrap();
    assert_eq!(once, twice, "fmt should be idempotent");

    // A second `--check` run should now report already-formatted.
    adept()
        .arg("fmt")
        .arg(dir.path())
        .arg("--check")
        .assert()
        .success();
}

#[test]
fn score_without_model_exits_two_with_actionable_message() {
    adept()
        .arg("score")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ADEPT_MODEL"));
}

#[test]
fn check_accepts_tokenizer_flag_for_both_values() {
    for tokenizer in ["o200k-base", "cl100k-base"] {
        let output = adept()
            .arg("check")
            .arg(fixture("clean-skill"))
            .arg("--tokenizer")
            .arg(tokenizer)
            .arg("--format")
            .arg("json")
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|err| panic!("output was not valid JSON: {err}\n{stdout}"));
        assert!(parsed.is_array());
    }
}

#[test]
fn check_rejects_invalid_tokenizer_value() {
    adept()
        .arg("check")
        .arg(fixture("clean-skill"))
        .arg("--tokenizer")
        .arg("not-a-real-tokenizer")
        .assert()
        .failure();
}

#[test]
fn score_help_documents_tokenizer_flag() {
    adept()
        .arg("score")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--tokenizer"));
}

#[test]
fn mcp_score_skill_without_llm_config_returns_structured_error_not_hang_or_panic() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("adept"))
        .arg("mcp")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
        )
        .unwrap();
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "score_skill",
                "arguments": { "content": "---\nname: sample\ndescription: does a thing. Use when the user asks for a thing.\n---\nBody.\n" }
            }
        });
        writeln!(stdin, "{request}").unwrap();
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    let mut saw_score_error = false;
    for line in stdout.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|err| panic!("stdout line was not valid JSON: {err}\nline={line}"));
        assert_eq!(parsed["jsonrpc"], "2.0");
        if parsed["id"] == 2 {
            // Either a structured tool-level error (isError: true) or a
            // JSON-RPC-level error is acceptable, but it must not hang, and
            // it must not be a bare panic message.
            let is_tool_error = parsed["result"]["isError"] == true;
            let is_rpc_error = parsed.get("error").is_some();
            assert!(
                is_tool_error || is_rpc_error,
                "expected a structured error for score_skill without LLM config, got {parsed}"
            );
            saw_score_error = true;
        }
    }
    assert!(saw_score_error, "expected a response for id=2");
}

#[test]
fn mcp_format_skill_rejects_out_of_range_line_width() {
    for bad_width in [0, 10_000] {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "format_skill",
                "arguments": {
                    "content": "---\nname: sample\ndescription: does a thing. Use when the user asks for a thing.\n---\nBody.\n",
                    "line_width": bad_width
                }
            }
        });

        let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("adept"))
            .arg("mcp")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        {
            use std::io::Write;
            writeln!(child.stdin.as_mut().unwrap(), "{request}").unwrap();
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let line = stdout.lines().next().expect("expected one response line");
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            parsed["result"]["isError"], true,
            "line_width={bad_width} should be rejected, got {parsed}"
        );
    }
}

#[test]
fn help_and_version_work() {
    adept().arg("--help").assert().success();
    adept().arg("--version").assert().success();
    adept().arg("check").arg("--help").assert().success();
    adept().arg("fmt").arg("--help").assert().success();
    adept().arg("score").arg("--help").assert().success();
}

#[test]
fn mcp_stdout_carries_only_well_formed_jsonrpc_lines() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("adept"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
        )
        .unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut saw_initialize = false;
    let mut saw_tools_list = false;
    for line in stdout.lines() {
        // Every non-empty stdout line must parse as a JSON-RPC response;
        // nothing else (logs, panics) is permitted on stdout.
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|err| panic!("stdout line was not valid JSON: {err}\nline={line}"));
        assert_eq!(parsed["jsonrpc"], "2.0");
        match parsed["id"].as_i64() {
            Some(1) => saw_initialize = true,
            Some(2) => saw_tools_list = true,
            _ => {}
        }
    }
    assert!(saw_initialize && saw_tools_list);
}
