//! `adept mcp`: a Model Context Protocol server over stdio.
//!
//! Implementation choice: this implements the JSON-RPC 2.0 stdio transport
//! directly (newline-delimited JSON messages, per the MCP spec) rather than
//! depending on the `rmcp` SDK crate. `adept mcp` exposes three tools
//! (`check_skill`, `format_skill`, `score_skill`) behind `initialize` /
//! `tools/list` / `tools/call`, which is a small enough surface that a
//! direct implementation keeps the dependency footprint (and the risk of
//! an unfamiliar SDK writing to stdout on our behalf) minimal.
//!
//! **Critical invariant**: stdout carries only JSON-RPC response messages.
//! All logging/diagnostics go to stderr. [`handle_message`] never prints
//! anything itself; [`serve`] is the only place that writes to stdout.

use std::io::{BufRead, Write};
use std::sync::OnceLock;
use std::time::Duration;

use adept::{AnthropicSkillParser, LintConfig, Linter, Skill, SkillParser, SkillSet};
use adept_fmt::{format_str, FmtConfig};
use adept_score::{LlmConfig, OpenAiCompatClient, ScoreOptions};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "adept";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Minimum/maximum accepted `line_width` for `format_skill`. Guards against
/// `0` (degenerate one-word-per-line output) and unreasonably large values.
const MIN_LINE_WIDTH: u64 = 20;
const MAX_LINE_WIDTH: u64 = 500;

/// How long `score_skill` will wait for the LLM backend before giving up.
const SCORE_TIMEOUT: Duration = Duration::from_secs(30);

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
    let mut tools = vec![
        json!({
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
        }),
        json!({
            "name": "format_skill",
            "description": "Format a SKILL.md file's content, given a filesystem path or raw content, returning the canonically formatted text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to a SKILL.md file." },
                    "content": { "type": "string", "description": "Raw SKILL.md source text (used instead of `path`)." },
                    "line_width": {
                        "type": "integer",
                        "description": "Target line width for prose reflow (default 100; must be between 20 and 500).",
                        "minimum": MIN_LINE_WIDTH,
                        "maximum": MAX_LINE_WIDTH
                    }
                },
                "anyOf": [ { "required": ["path"] }, { "required": ["content"] } ]
            }
        }),
    ];

    // Only advertise `score_skill` when an LLM backend can actually be
    // resolved (network-backed; requires `ADEPT_MODEL` etc.) so agents
    // don't discover a tool that's guaranteed to fail.
    if LlmConfig::default().resolve().is_ok() {
        tools.push(json!({
            "name": "score_skill",
            "description": "Score a skill's triggering accuracy, token bloat, and overlap with sibling skills using an LLM. Requires ADEPT_MODEL (and optionally ADEPT_BASE_URL/ADEPT_API_KEY) to be configured; network-backed with a timeout.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to a SKILL.md file or skill directory." },
                    "content": { "type": "string", "description": "Raw SKILL.md source text (used instead of `path`)." },
                    "directory": { "type": "string", "description": "Skills root to search for sibling skills when detecting overlap. Defaults to the parent directory of `path`; required to get overlap detection when scoring raw `content`." },
                    "model": { "type": "string", "description": "Override the model to score with (defaults to ADEPT_MODEL)." },
                    "base_url": { "type": "string", "description": "Override the OpenAI-compatible base URL (defaults to ADEPT_BASE_URL or the OpenAI API)." }
                },
                "anyOf": [ { "required": ["path"] }, { "required": ["content"] } ]
            }
        }));
    }

    json!({ "tools": tools })
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
        "score_skill" => Ok(tool_result(score_skill_tool(&arguments))),
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

    // Built once for the life of the server: `Linter::new` loads the
    // tiktoken BPE tables, which is far more expensive than the lint itself
    // and must not be repeated on every tool call.
    static LINTER: OnceLock<Result<Linter, String>> = OnceLock::new();
    let linter = match LINTER
        .get_or_init(|| Linter::new(LintConfig::default()).map_err(|e| e.to_string()))
    {
        Ok(linter) => linter,
        Err(err) => return (json!({ "error": err }).to_string(), true),
    };
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
    if let Some(width_value) = arguments.get("line_width") {
        let width = match width_value.as_u64() {
            Some(width) if (MIN_LINE_WIDTH..=MAX_LINE_WIDTH).contains(&width) => width,
            _ => {
                return (
                    format!(
                        "invalid `line_width`: must be an integer between {MIN_LINE_WIDTH} and {MAX_LINE_WIDTH}"
                    ),
                    true,
                );
            }
        };
        config.line_width = width as usize;
    }

    match format_str(&source, &config) {
        Ok(formatted) => (formatted, false),
        Err(err) => (err.to_string(), true),
    }
}

/// Build the skillset used for overlap detection in `score_skill`.
///
/// Overlap detection is pairwise, so a skillset containing only the target
/// skill can never surface an overlap — the skill is compared against itself.
/// Mirror the `adept score` CLI: discover sibling skills so the target is
/// adjudicated against its neighbours.
///
/// The search root is `directory` if given. Otherwise, for a real on-disk
/// `path` (the synthetic `"SKILL.md"` default used for raw `content` is not
/// treated as a location), it is the parent of the skill's *own* directory —
/// in the standard `<root>/<skill-name>/SKILL.md` layout, that is `<root>`,
/// where the sibling skill directories live. (`discover` walks recursively,
/// so searching the skill's own directory would only ever re-find itself.)
/// When neither is available, fall back to the target alone — overlap is
/// genuinely undetectable then. The target skill is always included so its
/// own pairs are considered even if discovery (e.g. from `content` that
/// differs from disk) would miss it.
fn overlap_skillset(arguments: &Value, path: &std::path::Path, skill: &Skill) -> Vec<Skill> {
    let search_root = arguments
        .get("directory")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .or_else(|| {
            // Only treat `path` as a filesystem location when the caller
            // actually passed one, not the synthetic `read_source` default.
            arguments.get("path").and_then(Value::as_str).map(|_| {
                // The skill's own directory: the file's parent, or the given
                // directory itself. Siblings live one level above that.
                let skill_dir = if path.is_dir() {
                    path.to_path_buf()
                } else {
                    path.parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                };
                skill_dir
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or(skill_dir)
            })
        });

    let Some(root) = search_root else {
        return vec![skill.clone()];
    };

    let mut skills = SkillSet::discover(&root)
        .map(|set| set.skills)
        .unwrap_or_default();
    if !skills
        .iter()
        .any(|s| s.frontmatter.name == skill.frontmatter.name)
    {
        skills.push(skill.clone());
    }
    skills
}

/// `score_skill` MCP tool: runs LLM-assisted scoring, given a resolvable
/// `ADEPT_MODEL`/`ADEPT_BASE_URL`/`ADEPT_API_KEY` (or `model`/`base_url`
/// arguments). Never panics or hangs: a missing/unresolvable LLM config, a
/// malformed skill, or a timed-out request all come back as a structured
/// `(text, is_error=true)` result rather than propagating a panic.
fn score_skill_tool(arguments: &Value) -> (String, bool) {
    let (source, path) = match read_source(arguments) {
        Ok(pair) => pair,
        Err(message) => return (message, true),
    };

    let skill = match AnthropicSkillParser.parse_str(&path, &source) {
        Ok(skill) => skill,
        Err(err) => return (json!({ "error": err.to_string() }).to_string(), true),
    };

    let llm_config = LlmConfig {
        base_url: arguments
            .get("base_url")
            .and_then(Value::as_str)
            .map(str::to_string),
        api_key: None,
        model: arguments
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    let resolved = match llm_config.resolve() {
        Ok(resolved) => resolved,
        Err(err) => {
            return (
                json!({
                    "error": format!(
                        "no LLM model configured for score_skill: {err} (set ADEPT_MODEL, or pass a `model` argument)"
                    )
                })
                .to_string(),
                true,
            );
        }
    };

    let client = OpenAiCompatClient::new(resolved.clone());
    let options = ScoreOptions::for_model(&resolved.model, adept::Tokenizer::default());
    let skillset = overlap_skillset(arguments, &path, &skill);

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(err) => {
            return (
                format!("failed to start async runtime for score_skill: {err}"),
                true,
            );
        }
    };

    let outcome = runtime.block_on(tokio::time::timeout(
        SCORE_TIMEOUT,
        adept_score::score_skill(&client, &skill, &skillset, &options),
    ));

    match outcome {
        Ok(Ok(report)) => match serde_json::to_string(&report) {
            Ok(json) => (json, false),
            Err(err) => (format!("failed to render score report: {err}"), true),
        },
        Ok(Err(err)) => (json!({ "error": err.to_string() }).to_string(), true),
        Err(_elapsed) => (
            json!({ "error": format!("score_skill timed out after {SCORE_TIMEOUT:?}") })
                .to_string(),
            true,
        ),
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

    fn write_skill(dir: &std::path::Path, name: &str, description: &str) -> std::path::PathBuf {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        std::fs::write(
            &path,
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\nBody.\n"),
        )
        .unwrap();
        path
    }

    #[test]
    fn overlap_skillset_discovers_siblings_from_path_parent() {
        let root = tempfile::tempdir().unwrap();
        let target_path = write_skill(root.path(), "alpha", "Does alpha things. Use when alpha.");
        write_skill(root.path(), "beta", "Does beta things. Use when beta.");

        let (source, path) = read_source(&json!({ "path": target_path.to_str().unwrap() })).unwrap();
        let skill = AnthropicSkillParser.parse_str(&path, &source).unwrap();

        let skillset =
            overlap_skillset(&json!({ "path": target_path.to_str().unwrap() }), &path, &skill);

        let names: Vec<&str> = skillset.iter().map(|s| s.frontmatter.name.as_str()).collect();
        assert!(names.contains(&"alpha"), "target must be present: {names:?}");
        assert!(names.contains(&"beta"), "sibling must be discovered: {names:?}");
    }

    #[test]
    fn overlap_skillset_falls_back_to_target_for_raw_content() {
        // No `path` and no `directory`: overlap is genuinely undetectable,
        // but the target must still be present so scoring proceeds.
        let path = std::path::PathBuf::from("SKILL.md");
        let skill = AnthropicSkillParser.parse_str(&path, SAMPLE_SKILL).unwrap();
        let skillset = overlap_skillset(&json!({ "content": SAMPLE_SKILL }), &path, &skill);
        assert_eq!(skillset.len(), 1);
        assert_eq!(skillset[0].frontmatter.name, "sample");
    }

    #[test]
    fn overlap_skillset_honors_explicit_directory_for_content() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "gamma", "Does gamma things. Use when gamma.");

        let path = std::path::PathBuf::from("SKILL.md");
        let skill = AnthropicSkillParser.parse_str(&path, SAMPLE_SKILL).unwrap();
        let skillset = overlap_skillset(
            &json!({ "content": SAMPLE_SKILL, "directory": root.path().to_str().unwrap() }),
            &path,
            &skill,
        );

        let names: Vec<&str> = skillset.iter().map(|s| s.frontmatter.name.as_str()).collect();
        assert!(names.contains(&"gamma"), "directory sibling: {names:?}");
        assert!(names.contains(&"sample"), "target appended: {names:?}");
    }

    #[test]
    fn notification_with_no_id_produces_no_response() {
        let request = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_message(&request.to_string()).is_none());
    }
}
