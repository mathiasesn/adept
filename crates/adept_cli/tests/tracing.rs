//! Integration tests for the tracing / capture surfaces.
//!
//! Three properties are pinned here:
//!
//! 1. **MCP stdout purity under maximum verbosity.** `tracing_subscriber::fmt()`
//!    defaults to *stdout*, which `adept mcp` owns; a subscriber that lands
//!    there breaks every MCP client silently. These tests drive the real
//!    binary with `-vvv` and with `ADEPT_LOG=trace` and assert every stdout
//!    line still parses as JSON-RPC.
//! 2. **The default is byte-identical to a build with no logging.** No `-v`
//!    and no `ADEPT_LOG` means the filter is `off`: empty stderr, and stdout
//!    unchanged from the same run with logging cranked up.
//! 3. **The `--capture-dir` surface exists and fails loudly.** No test here
//!    performs network I/O; the capture *write* path is exercised by
//!    `adept_agent`'s own unit tests against a `CaptureSink` directly.

mod common;

use predicates::prelude::*;

use common::{
    adept, assert_pure_jsonrpc, fixture, jsonrpc_messages, mcp_tools_call, run_mcp, MCP_INITIALIZE,
    MCP_TOOLS_LIST, SAMPLE_SKILL,
};

/// The request set every MCP test here drives: a real `initialize`, a real
/// `tools/list`, and a real `check_skill` call, so the server does actual
/// work while logging.
fn mcp_requests() -> Vec<String> {
    vec![
        MCP_INITIALIZE.to_string(),
        MCP_TOOLS_LIST.to_string(),
        mcp_tools_call(
            3,
            "check_skill",
            serde_json::json!({ "content": SAMPLE_SKILL }),
        ),
    ]
}

#[test]
fn mcp_stdout_stays_pure_jsonrpc_at_maximum_verbosity_flag() {
    // The headline regression test: `-vvv` selects TRACE, and
    // `tracing_subscriber::fmt()` defaults to stdout. If a future change
    // drops `.with_writer(std::io::stderr)`, this fails.
    let (stdout, _stderr) = run_mcp(&[], &["-vvv"], &mcp_requests());
    assert_pure_jsonrpc(&stdout, "-vvv", &[1, 2, 3]);
}

#[test]
fn mcp_stdout_stays_pure_jsonrpc_with_adept_log_trace() {
    // `ADEPT_LOG` bypasses the `-v` mapping entirely and installs an
    // `EnvFilter` from directives, so it is a separate route to a live
    // subscriber and needs its own guard.
    let (stdout, _stderr) = run_mcp(&[("ADEPT_LOG", "trace")], &[], &mcp_requests());
    assert_pure_jsonrpc(&stdout, "ADEPT_LOG=trace", &[1, 2, 3]);

    // Belt and braces: both activation surfaces at once.
    let (stdout, _stderr) = run_mcp(&[("ADEPT_LOG", "trace")], &["-vvv"], &mcp_requests());
    assert_pure_jsonrpc(&stdout, "ADEPT_LOG=trace -vvv", &[1, 2, 3]);
}

#[test]
fn mcp_stdout_is_identical_with_and_without_logging() {
    // Beyond "it parses": the byte stream a client sees must not change at
    // all when logging is enabled. With no network activity stderr may
    // legitimately be empty, so it is not asserted on.
    let (quiet_stdout, quiet_stderr) = run_mcp(&[], &[], &mcp_requests());
    let (loud_stdout, _loud_stderr) =
        run_mcp(&[("ADEPT_LOG", "trace")], &["-vvv"], &mcp_requests());

    assert!(
        quiet_stderr.is_empty(),
        "default mcp run should write nothing to stderr, got: {quiet_stderr}"
    );
    assert_eq!(
        quiet_stdout, loud_stdout,
        "enabling logging must not perturb a single byte of MCP stdout"
    );
}

#[test]
fn mcp_eval_skill_schema_never_exposes_capture() {
    // Spec §12: capture is CLI-only and unreachable from MCP, so the
    // `eval_skill` tool's public JSON-RPC contract must be unchanged by
    // the capture layer. Driven over real stdio, against the real schema.
    //
    // `eval_skill` is always advertised (grading needs no model), so
    // `ADEPT_MODEL` is set — the tool is listed, never called, so this
    // performs no network I/O.
    let (stdout, _stderr) = run_mcp(&[], &[], &mcp_requests());
    let list = jsonrpc_messages(&stdout)
        .into_iter()
        .find(|message| message["id"] == 2)
        .expect("a tools/list response");

    let tool = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["name"] == "eval_skill")
        .expect("an eval_skill tool");

    let schema = &tool["inputSchema"];
    let properties = schema["properties"]
        .as_object()
        .expect("eval_skill input schema properties");
    for name in properties.keys() {
        assert!(
            !name.to_ascii_lowercase().contains("capture"),
            "capture reached the MCP tool schema via property `{name}`"
        );
    }
    // Belt and braces: nothing anywhere in the serialized schema mentions
    // it either (descriptions, nested objects, required lists).
    let rendered = serde_json::to_string(schema).unwrap().to_ascii_lowercase();
    assert!(
        !rendered.contains("capture"),
        "capture reached the eval_skill schema: {rendered}"
    );
}

#[test]
fn check_default_output_is_unchanged_by_the_logging_layer() {
    let quiet = adept()
        .arg("check")
        .arg(fixture("defective-skill"))
        .output()
        .unwrap();
    assert!(
        quiet.stderr.is_empty(),
        "a default `check` run must write nothing to stderr, got: {}",
        String::from_utf8_lossy(&quiet.stderr)
    );

    let loud = adept()
        .arg("check")
        .arg(fixture("defective-skill"))
        .arg("-vvv")
        .env("ADEPT_LOG", "trace")
        .output()
        .unwrap();

    assert_eq!(
        quiet.stdout, loud.stdout,
        "`check` stdout must be byte-identical with and without logging"
    );
    assert_eq!(quiet.status.code(), Some(1));
    assert_eq!(loud.status.code(), quiet.status.code());
}

#[test]
fn fmt_check_default_output_is_unchanged_by_the_logging_layer() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("SKILL.md"),
        "---\nname: unformatted\ndescription: a description here that is long enough to pass\n---\nBody   text.\n",
    )
    .unwrap();

    let quiet = adept()
        .arg("fmt")
        .arg(dir.path())
        .arg("--check")
        .output()
        .unwrap();
    assert!(
        quiet.stderr.is_empty(),
        "a default `fmt --check` run must write nothing to stderr, got: {}",
        String::from_utf8_lossy(&quiet.stderr)
    );

    let loud = adept()
        .arg("fmt")
        .arg(dir.path())
        .arg("--check")
        .arg("-vvv")
        .env("ADEPT_LOG", "trace")
        .output()
        .unwrap();

    assert_eq!(
        quiet.stdout, loud.stdout,
        "`fmt --check` stdout must be byte-identical with and without logging"
    );
    assert_eq!(quiet.status.code(), Some(1));
    assert_eq!(loud.status.code(), quiet.status.code());
}

#[test]
fn verbose_flag_is_global_and_documented() {
    adept()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--verbose"));

    // `global = true` means every subcommand accepts it.
    for subcommand in ["check", "fmt", "eval", "fix", "mcp"] {
        adept()
            .arg(subcommand)
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("--verbose"));
    }
}

#[test]
fn capture_dir_flag_is_advertised_on_eval_and_fix_only() {
    for subcommand in ["eval", "fix"] {
        adept()
            .arg(subcommand)
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("--capture-dir"));
    }

    // Capture is CLI-only for the LLM-backed commands; the offline
    // commands must not grow the flag.
    for subcommand in ["check", "fmt", "mcp"] {
        adept()
            .arg(subcommand)
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("--capture-dir").not());
    }
}

#[test]
fn eval_with_uncreatable_capture_dir_exits_two_before_any_network_io() {
    // `run` resolves the capture sink *before* `execute` builds a runtime or
    // issues a request, so this exercises the failure path without a single
    // byte on the wire. The base URL is a reserved-for-documentation address
    // that is never dialled.
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"this is a regular file").unwrap();
    let capture = blocker.join("run");

    adept()
        .arg("eval")
        .arg(fixture("clean-skill").join("SKILL.md"))
        .arg("--capture-dir")
        .arg(&capture)
        .env("ADEPT_MODEL", "test-model")
        .env("ADEPT_BASE_URL", "http://127.0.0.1:1/v1")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("adept: error:"))
        .stderr(predicate::str::contains("capture directory"));
}

#[test]
fn fix_with_uncreatable_capture_dir_exits_two_before_any_network_io() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"this is a regular file").unwrap();
    let capture = blocker.join("run");

    adept()
        .arg("fix")
        .arg(fixture("defective-skill"))
        .arg("--diff")
        .arg("--capture-dir")
        .arg(&capture)
        .env("ADEPT_MODEL", "test-model")
        .env("ADEPT_BASE_URL", "http://127.0.0.1:1/v1")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("adept: error:"))
        .stderr(predicate::str::contains("capture directory"));
}
