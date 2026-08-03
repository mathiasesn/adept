//! CLI integration tests, driving the built `adept` binary via `assert_cmd`.

mod common;

use predicates::prelude::*;

use common::{
    adept, assert_pure_jsonrpc, fixture, jsonrpc_messages, mcp_tools_call, run_mcp, MCP_INITIALIZE,
    MCP_TOOLS_LIST, SAMPLE_SKILL,
};

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
fn eval_without_model_or_results_exits_two_with_actionable_message() {
    adept()
        .arg("eval")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nothing to evaluate"));
}

#[test]
fn eval_select_triggering_without_model_exits_two_naming_model() {
    adept()
        .arg("eval")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .arg("--select")
        .arg("triggering")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("model"));
}

#[test]
fn eval_credentials_in_base_url_exits_two_not_silently_downgraded() {
    // Regression test: probe_model_available must not treat a credential
    // error as "no model configured", or `adept eval` with no `--select`
    // silently narrows to the offline analyses. Note the exit code alone
    // does not catch that — the downgraded path also exits 2, via "nothing
    // to evaluate" — so the assertion that stderr names `api_key` is the
    // part doing the work here.
    //
    // `--model` is passed so the credential error is the *only* resolution
    // failure; without it this would also satisfy `ConfigError::MissingModel`
    // and would silently start passing for the wrong reason if the order of
    // the two checks in `LlmConfig::resolve` ever changed.
    adept()
        .arg("eval")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .arg("--model")
        .arg("test-model")
        .arg("--base-url")
        .arg("https://alice:hunter2secret@gw.example/v1")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("api_key")
                .and(predicate::str::contains("alice:hunter2secret@").not())
                .and(predicate::str::contains("hunter2secret").not()),
        );
}

#[test]
fn eval_select_unknown_analysis_is_rejected() {
    adept()
        .arg("eval")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .arg("--select")
        .arg("bogus")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown analysis"));
}

#[test]
fn eval_select_evals_without_results_exits_two_naming_results() {
    adept()
        .arg("eval")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .arg("--select")
        .arg("evals")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--results"));
}

#[test]
fn eval_select_evals_offline_runs_with_no_model_configured() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("demo-skill");
    std::fs::create_dir_all(skill_dir.join("evals")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo-skill\ndescription: does a demo thing. Use when demoing things.\n---\nBody.\n",
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("evals").join("evals.jsonl"),
        "{\"schema_version\":1,\"prompt\":\"demo\",\"assertions\":[{\"kind\":\"contains\",\"value\":\"ok\"}]}\n",
    )
    .unwrap();
    let results_path = dir.path().join("results.jsonl");
    std::fs::write(&results_path, "{\"case\":1,\"response\":\"it is ok\"}\n").unwrap();

    adept()
        .arg("eval")
        .arg(skill_dir.join("SKILL.md"))
        .arg("--results")
        .arg(&results_path)
        .arg("--select")
        .arg("evals")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .success()
        .stdout(predicate::str::contains("Eval-dataset grading"));
}

#[test]
fn eval_select_evals_omits_unselected_analysis_sections() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("demo");
    std::fs::create_dir_all(skill_dir.join("evals")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo\ndescription: does a demo thing. Use when demoing things.\n---\nBody.\n",
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("evals").join("evals.jsonl"),
        "{\"schema_version\":1,\"prompt\":\"one\",\"assertions\":[{\"kind\":\"contains\",\"value\":\"ok\"}]}\n\
         {\"schema_version\":1,\"prompt\":\"two\",\"assertions\":[{\"kind\":\"contains\",\"value\":\"ok\"},{\"kind\":\"command\",\"command\":\"true\"}]}\n",
    )
    .unwrap();
    let results_path = dir.path().join("results.jsonl");
    std::fs::write(
        &results_path,
        "{\"case\":1,\"response\":\"it is ok\"}\n\
         {\"case\":2,\"response\":\"nope\"}\n\
         {\"case\":1,\"response\":\"whatever\",\"arm\":\"baseline\"}\n\
         {\"case\":2,\"response\":\"whatever\",\"arm\":\"baseline\"}\n",
    )
    .unwrap();

    adept()
        .arg("eval")
        .arg(skill_dir.join("SKILL.md"))
        .arg("--results")
        .arg(&results_path)
        .arg("--select")
        .arg("evals")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("Triggering accuracy")
                .not()
                .and(predicate::str::contains("Token bloat").not())
                .and(predicate::str::contains("Overlap/conflict detection").not())
                .and(predicate::str::contains("prompt set version").not())
                .and(predicate::str::contains("pass rate: 50% (2 cases)"))
                .and(predicate::str::contains("assertions: 1/2 met (1 skipped)")),
        );

    adept()
        .arg("eval")
        .arg(skill_dir.join("SKILL.md"))
        .arg("--results")
        .arg(&results_path)
        .arg("--select")
        .arg("evals")
        .arg("--format")
        .arg("json")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("\"triggering\"")
                .not()
                .and(predicate::str::contains("\"token_bloat\"").not())
                .and(predicate::str::contains("\"overlaps\"").not())
                .and(predicate::str::contains("\"evals\"")),
        );
}

/// Finding 1's central regression: a skill that passes every case must exit
/// `0` even though the baseline arm (which is *expected* to fail — that's
/// what makes lift meaningful) has failing results. Exit code must be
/// derived from `Arm::Skill` cases only, not from every `CaseReport`
/// `grade` produces.
#[test]
fn eval_all_skill_cases_pass_with_failing_baseline_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("demo");
    std::fs::create_dir_all(skill_dir.join("evals")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo\ndescription: does a demo thing. Use when demoing things.\n---\nBody.\n",
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("evals").join("evals.jsonl"),
        "{\"schema_version\":1,\"prompt\":\"one\",\"assertions\":[{\"kind\":\"contains\",\"value\":\"ok\"}]}\n",
    )
    .unwrap();
    let results_path = dir.path().join("results.jsonl");
    std::fs::write(
        &results_path,
        "{\"case\":1,\"response\":\"it is ok\"}\n\
         {\"case\":1,\"response\":\"nope\",\"arm\":\"baseline\"}\n",
    )
    .unwrap();

    adept()
        .arg("eval")
        .arg(skill_dir.join("SKILL.md"))
        .arg("--results")
        .arg(&results_path)
        .arg("--select")
        .arg("evals")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("pass rate: 100%"));
}

/// Complement of the above: one failing skill case must still exit `1`,
/// baseline or no baseline.
#[test]
fn eval_one_failing_skill_case_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("demo");
    std::fs::create_dir_all(skill_dir.join("evals")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo\ndescription: does a demo thing. Use when demoing things.\n---\nBody.\n",
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("evals").join("evals.jsonl"),
        "{\"schema_version\":1,\"prompt\":\"one\",\"assertions\":[{\"kind\":\"contains\",\"value\":\"ok\"}]}\n",
    )
    .unwrap();
    let results_path = dir.path().join("results.jsonl");
    std::fs::write(&results_path, "{\"case\":1,\"response\":\"nope\"}\n").unwrap();

    adept()
        .arg("eval")
        .arg(skill_dir.join("SKILL.md"))
        .arg("--results")
        .arg(&results_path)
        .arg("--select")
        .arg("evals")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("pass rate: 0%"));
}

#[test]
fn eval_accepts_skill_directory_path_not_just_skill_md() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("demo-skill");
    std::fs::create_dir_all(skill_dir.join("evals")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo-skill\ndescription: does a demo thing. Use when demoing things.\n---\nBody.\n",
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("evals").join("evals.jsonl"),
        "{\"schema_version\":1,\"prompt\":\"demo\",\"assertions\":[{\"kind\":\"contains\",\"value\":\"ok\"}]}\n",
    )
    .unwrap();
    let results_path = dir.path().join("results.jsonl");
    std::fs::write(&results_path, "{\"case\":1,\"response\":\"it is ok\"}\n").unwrap();

    // Pass the skill *directory*, not the SKILL.md file, as the spec's
    // examples do (`adept eval ./my-skill`).
    adept()
        .arg("eval")
        .arg(&skill_dir)
        .arg("--results")
        .arg(&results_path)
        .arg("--select")
        .arg("evals")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .success()
        .stdout(predicate::str::contains("Eval-dataset grading"));
}

#[test]
fn eval_rejects_directory_without_skill_md_with_exit_two() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("not-a-skill")).unwrap();
    let results_path = dir.path().join("results.jsonl");
    std::fs::write(&results_path, "{\"case\":1,\"response\":\"it is ok\"}\n").unwrap();

    adept()
        .arg("eval")
        .arg(dir.path().join("not-a-skill"))
        .arg("--results")
        .arg(&results_path)
        .arg("--select")
        .arg("evals")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no SKILL.md found"));
}

#[test]
fn adept_score_is_gone_from_help_and_fails_rather_than_running() {
    adept()
        .arg("score")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));

    adept()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("eval").and(predicate::str::contains("score").not()));
}

#[test]
fn legacy_score_config_section_fails_with_exit_two_naming_eval() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("adept.toml"),
        "[score]\nmodel = \"old-model\"\n",
    )
    .unwrap();

    adept()
        .current_dir(dir.path())
        .arg("check")
        .arg(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("[score]").and(predicate::str::contains("[eval]")));
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
fn eval_help_documents_tokenizer_flag() {
    adept()
        .arg("eval")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--tokenizer"));
}

#[test]
fn mcp_eval_skill_without_llm_config_returns_structured_error_not_hang_or_panic() {
    let (stdout, _stderr) = run_mcp(
        &[],
        &[],
        &[
            MCP_INITIALIZE.to_string(),
            mcp_tools_call(
                2,
                "eval_skill",
                serde_json::json!({ "content": SAMPLE_SKILL, "select": ["triggering"] }),
            ),
        ],
    );

    let mut saw_eval_error = false;
    for parsed in jsonrpc_messages(&stdout) {
        assert_eq!(parsed["jsonrpc"], "2.0");
        if parsed["id"] == 2 {
            // Either a structured tool-level error (isError: true) or a
            // JSON-RPC-level error is acceptable, but it must not hang, and
            // it must not be a bare panic message.
            let is_tool_error = parsed["result"]["isError"] == true;
            let is_rpc_error = parsed.get("error").is_some();
            assert!(
                is_tool_error || is_rpc_error,
                "expected a structured error for eval_skill without LLM config, got {parsed}"
            );
            saw_eval_error = true;
        }
    }
    assert!(saw_eval_error, "expected a response for id=2");
}

#[test]
fn mcp_tools_call_score_skill_errors_naming_eval_skill() {
    let (stdout, _stderr) = run_mcp(
        &[],
        &[],
        &[
            MCP_INITIALIZE.to_string(),
            mcp_tools_call(
                2,
                "score_skill",
                serde_json::json!({ "content": SAMPLE_SKILL }),
            ),
        ],
    );

    let response = jsonrpc_messages(&stdout)
        .into_iter()
        .find(|parsed| parsed["id"] == 2)
        .expect("expected a response for id=2");
    let error = response
        .get("error")
        .expect("score_skill must be a JSON-RPC error, not a silent no-op");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("eval_skill"),
        "error should name the replacement tool: {message}"
    );
}

#[test]
fn mcp_format_skill_rejects_out_of_range_line_width() {
    for bad_width in [0, 10_000] {
        let request = mcp_tools_call(
            1,
            "format_skill",
            serde_json::json!({ "content": SAMPLE_SKILL, "line_width": bad_width }),
        );
        let (stdout, _stderr) = run_mcp(&[], &[], &[request]);
        let parsed = jsonrpc_messages(&stdout)
            .into_iter()
            .next()
            .expect("expected one response line");
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
    adept().arg("eval").arg("--help").assert().success();
}

#[test]
fn mcp_stdout_carries_only_well_formed_jsonrpc_lines() {
    let (stdout, _stderr) = run_mcp(
        &[],
        &[],
        &[MCP_INITIALIZE.to_string(), MCP_TOOLS_LIST.to_string()],
    );
    assert_pure_jsonrpc(&stdout, "initialize + tools/list", &[1, 2]);
}
