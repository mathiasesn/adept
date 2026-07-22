//! `adept mcp`: a Model Context Protocol server over stdio.
//!
//! Implementation choice: this implements the JSON-RPC 2.0 stdio transport
//! directly (newline-delimited JSON messages, per the MCP spec) rather than
//! depending on the `rmcp` SDK crate. `adept mcp` only needs to expose two
//! static, offline tools (`check_skill`, `format_skill`) behind
//! `initialize` / `tools/list` / `tools/call`, which is a small enough
//! surface that a direct implementation keeps the dependency footprint (and
//! the risk of an unfamiliar SDK writing to stdout on our behalf) minimal.
//!
//! **Critical invariant**: stdout carries only JSON-RPC response messages.
//! All logging/diagnostics go to stderr. [`handle_message`] never prints
//! anything itself; [`serve`] is the only place that writes to stdout.

use std::io::{BufRead, Write};

use adept::{AnthropicSkillParser, LintConfig, Linter, SkillParser};
use adept_fmt::{format_str, FmtConfig};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "adept";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the MCP stdio server: read newline-delimited JSON-RPC requests from
/// `stdin`, write newline-delimited JSON-RPC responses to `stdout`, and log
/// everything else to `stderr`. Runs until stdin is closed (EOF).
pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("adept mcp: error reading stdin: {err}");
                return 2;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_message(&line) {
            if let Err(err) = writeln!(stdout, "{response}") {
                eprintln!("adept mcp: error writing stdout: {err}");
                return 2;
            }
            if let Err(err) = stdout.flush() {
                eprintln!("adept mcp: error flushing stdout: {err}");
                return 2;
            }
        }
    }
    0
}

/// Handle one raw JSON-RPC request line, returning the JSON-encoded
/// response to write to stdout, or `None` if the message was a notification
/// (no `id`, so no response is expected).
///
/// Pure w.r.t. I/O: never touches stdin/stdout/stderr itself, which is what
/// lets tests drive it directly without spawning the binary.
pub fn handle_message(line: &str) -> Option<String> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            return Some(
                json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {err}") }
                })
                .to_string(),
            );
        }
    };

    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);

    let Some(method) = method else {
        return id
            .map(|id| error_response(id, -32600, "invalid request: missing `method`").to_string());
    };

    let is_notification = id.is_none();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => Ok(handle_initialize()),
        "notifications/initialized" => return None,
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(&params),
        other => Err((-32601, format!("method not found: {other}"))),
    };

    if is_notification {
        return None;
    }
    let id = id.unwrap_or(Value::Null);

    Some(match result {
        Ok(value) => success_response(id, value).to_string(),
        Err((code, message)) => error_response(id, code, &message).to_string(),
    })
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "check_skill",
                "description": "Lint a SKILL.md file, given a filesystem path or raw content, returning structured diagnostics.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Path to a SKILL.md file or skill directory." },
                        "content": { "type": "string", "description": "Raw SKILL.md source text (used instead of `path`)." }
                    },
                    "anyOf": [ { "required": ["path"] }, { "required": ["content"] } ]
                }
            },
            {
                "name": "format_skill",
                "description": "Format a SKILL.md file's content, given a filesystem path or raw content, returning the canonically formatted text.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Path to a SKILL.md file." },
                        "content": { "type": "string", "description": "Raw SKILL.md source text (used instead of `path`)." },
                        "line_width": { "type": "integer", "description": "Target line width for prose reflow (default 100)." }
                    },
                    "anyOf": [ { "required": ["path"] }, { "required": ["content"] } ]
                }
            }
        ]
    })
}

fn handle_tools_call(params: &Value) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "missing `name`".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    match name {
        "check_skill" => Ok(tool_result(check_skill_tool(&arguments))),
        "format_skill" => Ok(tool_result(format_skill_tool(&arguments))),
        other => Err((-32602, format!("unknown tool: {other}"))),
    }
}

/// Wrap a tool's (text, is_error) result into the MCP `tools/call` result
/// shape.
fn tool_result((text, is_error): (String, bool)) -> Value {
    json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": is_error
    })
}

/// Read either `content` (raw source) or `path` (read from disk) from
/// `arguments`, returning the source text and the path to attribute
/// diagnostics to.
fn read_source(arguments: &Value) -> Result<(String, std::path::PathBuf), String> {
    if let Some(content) = arguments.get("content").and_then(Value::as_str) {
        let path = arguments
            .get("path")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("SKILL.md"));
        return Ok((content.to_string(), path));
    }
    if let Some(path) = arguments.get("path").and_then(Value::as_str) {
        let path = std::path::PathBuf::from(path);
        let content = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        return Ok((content, path));
    }
    Err("must provide either `path` or `content`".to_string())
}

fn check_skill_tool(arguments: &Value) -> (String, bool) {
    let (source, path) = match read_source(arguments) {
        Ok(pair) => pair,
        Err(message) => return (message, true),
    };

    let skill = match AnthropicSkillParser.parse_str(&path, &source) {
        Ok(skill) => skill,
        Err(err) => return (json!({ "error": err.to_string() }).to_string(), true),
    };

    let linter = Linter::new(LintConfig::default());
    let diagnostics = linter.lint_skill(&skill);
    match adept::reporting::render_json(&diagnostics) {
        Ok(json) => (json, false),
        Err(err) => (format!("failed to render diagnostics: {err}"), true),
    }
}

fn format_skill_tool(arguments: &Value) -> (String, bool) {
    let (source, _path) = match read_source(arguments) {
        Ok(pair) => pair,
        Err(message) => return (message, true),
    };

    let mut config = FmtConfig::default();
    if let Some(width) = arguments.get("line_width").and_then(Value::as_u64) {
        config.line_width = width as usize;
    }

    match format_str(&source, &config) {
        Ok(formatted) => (formatted, false),
        Err(err) => (err.to_string(), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SKILL: &str = "---\nname: sample\ndescription: does a thing. Use when the user asks for a thing. Do not use otherwise.\n---\n\n# Sample\n\nBody text.\n";

    #[test]
    fn initialize_returns_protocol_version() {
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert_eq!(
            parsed["result"]["protocolVersion"],
            Value::String(PROTOCOL_VERSION.to_string())
        );
    }

    #[test]
    fn tools_list_exposes_check_and_format_skill() {
        let request = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        let tools = parsed["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"check_skill"));
        assert!(names.contains(&"format_skill"));
        for tool in tools {
            assert!(tool["inputSchema"]["type"] == "object");
        }
    }

    #[test]
    fn tools_call_check_skill_returns_diagnostics_json() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "check_skill", "arguments": { "content": SAMPLE_SKILL } }
        });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["id"], 3);
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        let diagnostics: Value = serde_json::from_str(text).unwrap();
        assert!(diagnostics.is_array());
        assert_eq!(parsed["result"]["isError"], false);
    }

    #[test]
    fn tools_call_format_skill_returns_formatted_text() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "format_skill", "arguments": { "content": SAMPLE_SKILL } }
        });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("---\n"));
        assert_eq!(parsed["result"]["isError"], false);
    }

    #[test]
    fn tools_call_unknown_tool_is_an_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "does_not_exist", "arguments": {} }
        });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        assert!(parsed.get("error").is_some());
    }

    #[test]
    fn notification_with_no_id_produces_no_response() {
        let request = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_message(&request.to_string()).is_none());
    }
}
