//! Helpers shared by the `adept_cli` integration test binaries.
//!
//! Each test file is its own crate, so anything used by more than one of
//! them lives here rather than being copy-pasted. `#![allow(dead_code)]` is
//! unavoidable: every consumer pulls in the whole module but uses only the
//! part it needs.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
// The `use` path reference below is flagged by `clippy::disallowed_types`
// (configured via the workspace-root `clippy.toml`, see `run_mcp`) the same
// as the construction site is, so the allow needs to be here too.
#[allow(clippy::disallowed_types)]
use std::process::{Command as StdCommand, Stdio};

use assert_cmd::Command;

/// Path to a checked-in test fixture directory.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The `adept` binary under test, with every `ADEPT_*` variable the tests
/// care about cleared.
///
/// The test process may inherit any of these from the developer's shell;
/// every assertion in these suites is about the flag/env matrix the test
/// sets explicitly, so they all start from a known-empty state.
pub fn adept() -> Command {
    let mut cmd = Command::cargo_bin("adept").unwrap();
    cmd.env_remove("ADEPT_LOG")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY");
    cmd
}

/// A real `initialize` request, id 1.
pub const MCP_INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
/// A real `tools/list` request, id 2.
pub const MCP_TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;

/// A `tools/call` request for `name` with `arguments`, at the given id.
pub fn mcp_tools_call(id: i64, name: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
    .to_string()
}

/// A skill source small enough to inline, clean enough not to distract.
pub const SAMPLE_SKILL: &str =
    "---\nname: sample\ndescription: does a thing. Use when the user asks for a thing.\n---\nBody.\n";

/// Drive `adept mcp` over real stdio, writing one `requests` line at a time,
/// and return `(stdout, stderr)` once the server exits.
///
/// `env` is applied on top of the same cleared baseline [`adept`] uses;
/// `args` are extra arguments placed after the `mcp` subcommand.
pub fn run_mcp(env: &[(&str, &str)], args: &[&str], requests: &[String]) -> (String, String) {
    // Driving the built `adept` binary over stdio is exactly what this
    // integration test harness must do to exercise `adept mcp` as a black
    // box; it is not the shipped binary spawning a subprocess itself.
    #[allow(clippy::disallowed_types)]
    let mut command = StdCommand::new(assert_cmd::cargo::cargo_bin("adept"));
    command.arg("mcp");
    for arg in args {
        command.arg(arg);
    }
    command
        .env_remove("ADEPT_LOG")
        .env_remove("ADEPT_MODEL")
        .env_remove("ADEPT_BASE_URL")
        .env_remove("ADEPT_API_KEY");
    for (key, value) in env {
        command.env(key, value);
    }

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// Assert every non-empty stdout line is a JSON-RPC 2.0 message carrying an
/// id, and that exactly `expected_ids` came back, in order.
///
/// Nothing else (log lines, panic messages) is permitted on MCP stdout, so
/// the parse itself is half the assertion.
pub fn assert_pure_jsonrpc(stdout: &str, context: &str, expected_ids: &[i64]) {
    let mut seen = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|err| {
            panic!("[{context}] stdout line was not valid JSON: {err}\nline={line}")
        });
        assert_eq!(
            parsed["jsonrpc"], "2.0",
            "[{context}] non-JSON-RPC object on stdout: {parsed}"
        );
        let id = parsed["id"]
            .as_i64()
            .unwrap_or_else(|| panic!("[{context}] response without an id: {parsed}"));
        seen.push(id);
    }
    assert_eq!(
        seen, expected_ids,
        "[{context}] expected exactly one response per request, in order"
    );
}

/// Parse every non-empty stdout line as JSON-RPC, panicking on anything that
/// is not, for tests that then assert on individual responses.
pub fn jsonrpc_messages(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("stdout line was not valid JSON: {err}\nline={line}"))
        })
        .collect()
}
