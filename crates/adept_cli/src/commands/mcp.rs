//! `adept mcp`: a Model Context Protocol server over stdio.
//!
//! Implementation choice: this implements the JSON-RPC 2.0 stdio transport
//! directly (newline-delimited JSON messages, per the MCP spec) rather than
//! depending on the `rmcp` SDK crate. `adept mcp` exposes five tools
//! (`check_skill`, `format_skill`, `eval_skill`, `create_skill`,
//! `generate_evals`) behind `initialize` /
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

use adept::{sibling_root, AnthropicSkillParser, LintConfig, Linter, Skill, SkillParser, SkillSet};
use adept_agent::{create_skill, CreateOptions};
use adept_agent::{
    llm_available, ConfigError, EvalOptions, EvalReport, LlmClient, LlmConfig, OpenAiCompatClient,
};
use adept_fmt::{format_str, FmtConfig};
use serde_json::{json, Value};

use crate::commands::eval::{narrow_options, needs_llm, resolve_analyses};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "adept";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Minimum/maximum accepted `line_width` for `format_skill`. Guards against
/// `0` (degenerate one-word-per-line output) and unreasonably large values.
const MIN_LINE_WIDTH: u64 = 20;
const MAX_LINE_WIDTH: u64 = 500;

/// Minimum/maximum accepted `max_rounds` for `create_skill`. Same rationale
/// as [`MIN_LINE_WIDTH`]/[`MAX_LINE_WIDTH`] (see ARCHI §12): an MCP client
/// talks to this server over public JSON-RPC with no other gate on LLM
/// spend, so every numeric argument that drives LLM calls needs an explicit
/// bound, not just a type.
const MIN_MAX_ROUNDS: u64 = 1;
const MAX_MAX_ROUNDS: u64 = 10;

/// Minimum/maximum accepted `eval_cases` for `create_skill`/`generate_evals`.
const MIN_EVAL_CASES: u64 = 1;
const MAX_EVAL_CASES: u64 = 50;

/// Validate a bounded numeric argument the same way `format_skill` validates
/// `line_width`: present-and-in-range is accepted, present-and-out-of-range
/// (or the wrong type) is a hard `is_error=true` tool result, and absent is
/// `Ok(None)` so the caller falls back to its own default.
fn bounded_u64_argument(
    arguments: &Value,
    name: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, String> {
    match arguments.get(name) {
        None => Ok(None),
        Some(value) => match value.as_u64() {
            Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(format!(
                "invalid `{name}`: must be an integer between {min} and {max}"
            )),
        },
    }
}

/// How long `eval_skill` will wait for the LLM backend before giving up.
const EVAL_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `create_skill` will wait for the LLM backend before giving up.
/// Longer than [`EVAL_TIMEOUT`]: `create` may make several authoring/repair
/// rounds plus one eval-generation call, all before returning.
const CREATE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long `generate_evals` will wait for the LLM backend before giving up.
/// A single call, so shorter than [`CREATE_TIMEOUT`].
const GENERATE_EVALS_TIMEOUT: Duration = Duration::from_secs(30);

/// Default number of synthetic eval cases `generate_evals` requests when the
/// caller doesn't specify `eval_cases`. Mirrors
/// `adept_agent::create::DEFAULT_EVAL_CASES`.
const DEFAULT_GENERATE_EVALS_CASES: usize = adept_agent::create::DEFAULT_EVAL_CASES;

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

    // `eval_skill` is **always advertised**, unlike `create_skill`/
    // `generate_evals` below: its `evals` (grading) analysis needs no model
    // at all, so gating it on `llm_configured` would hide a tool that works
    // fine offline. The `triggering`/`token-bloat`/`overlap` analyses still
    // need `ADEPT_MODEL` — see the description and `resolve_analyses`, which
    // enforces that per-analysis rather than at advertisement time.
    tools.push(json!({
        "name": "eval_skill",
        "description": "Evaluate a skill: triggering accuracy, token bloat, and overlap with sibling skills (each requires ADEPT_MODEL, optionally ADEPT_BASE_URL/ADEPT_API_KEY; network-backed with a timeout), plus offline eval-dataset grading against inline `results` (no model needed, no network call). Selects a default set of analyses from what's available unless `select`/`ignore` narrow it explicitly.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to a SKILL.md file or skill directory." },
                "content": { "type": "string", "description": "Raw SKILL.md source text (used instead of `path`)." },
                "directory": { "type": "string", "description": "Skills root to search for sibling skills when detecting overlap. Defaults to the parent directory of `path`; required to get overlap detection when evaluating raw `content`." },
                "model": { "type": "string", "description": "Override the model to use for the triggering/token-bloat/overlap analyses (defaults to ADEPT_MODEL)." },
                "base_url": { "type": "string", "description": "Override the OpenAI-compatible base URL (defaults to ADEPT_BASE_URL or the OpenAI API)." },
                "results": {
                    "type": "array",
                    "description": "Inline run results to grade against the eval dataset (same fields as a `results.jsonl` line: case, arm, response, cwd, command_exit_codes, tokens). Enables the `evals` analysis. Passing `results` alongside `content` (no real skill directory) grades `contains` only; `file_exists`/`file_contains` are reported as skipped, naming the missing directory.",
                    "items": { "type": "object" }
                },
                "evals": { "type": "string", "description": "Override path to the eval dataset (defaults to `evals/evals.jsonl` relative to `path`)." },
                "select": {
                    "type": "array",
                    "description": "Only run these analyses (`triggering`, `token-bloat`, `overlap`, `evals`). An explicitly selected analysis whose precondition is missing (no model, no `results`) is an error naming what's missing.",
                    "items": { "type": "string", "enum": ["triggering", "token-bloat", "overlap", "evals"] }
                },
                "ignore": {
                    "type": "array",
                    "description": "Skip these analyses (`triggering`, `token-bloat`, `overlap`, `evals`).",
                    "items": { "type": "string", "enum": ["triggering", "token-bloat", "overlap", "evals"] }
                }
            },
            "anyOf": [ { "required": ["path"] }, { "required": ["content"] } ]
        }
    }));

    // Only advertise `create_skill`/`generate_evals` when an LLM backend can
    // actually be resolved (network-backed; requires `ADEPT_MODEL` etc.) so
    // agents don't discover a tool that's guaranteed to fail. Resolved once
    // and shared between the two — this is the one gate `eval_skill` above
    // deliberately no longer participates in, since grading doesn't need it.
    let llm_configured = llm_available(&LlmConfig::default().resolve());

    // `create_skill`/`generate_evals` are also network-backed (same
    // `ADEPT_MODEL` resolution as `eval_skill`'s LLM analyses) and preview-only: neither
    // accepts any output-path argument, and neither ever touches the
    // filesystem for writing — see `create_skill_tool`/`generate_evals_tool`.
    if llm_configured {
        tools.push(json!({
            "name": "create_skill",
            "description": "Generate a new Agent Skill (SKILL.md, companion files, and a synthetic eval dataset) from a task brief using an LLM. Preview-only: returns the generated files and dataset as data and never writes to disk. Requires ADEPT_MODEL (and optionally ADEPT_BASE_URL/ADEPT_API_KEY) to be configured; network-backed with a timeout.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "brief": { "type": "string", "description": "The task brief describing what the skill should do." },
                    "directory": { "type": "string", "description": "The directory the generated skill would be created in (used to attribute paths and, via its parent, to discover sibling skills for the collision screen). Read-only: never written to. Defaults to the current directory." },
                    "name": { "type": "string", "description": "Override the skill name the model would otherwise derive from the brief." },
                    "model": { "type": "string", "description": "Override the model to generate with (defaults to ADEPT_MODEL)." },
                    "base_url": { "type": "string", "description": "Override the OpenAI-compatible base URL (defaults to ADEPT_BASE_URL or the OpenAI API)." },
                    "max_rounds": { "type": "integer", "description": "Maximum authoring/repair rounds (defaults to adept's built-in default).", "minimum": 1 },
                    "eval_cases": { "type": "integer", "description": "Number of synthetic eval cases to generate (defaults to adept's built-in default).", "minimum": 1 }
                },
                "required": ["brief"]
            }
        }));
        tools.push(json!({
            "name": "generate_evals",
            "description": "Generate a synthetic eval dataset (evals.jsonl cases) for a skill, given a filesystem path or raw content plus the original task brief, using an LLM. Preview-only: returns the generated dataset as data and never writes to disk. Requires ADEPT_MODEL (and optionally ADEPT_BASE_URL/ADEPT_API_KEY) to be configured; network-backed with a timeout.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to a SKILL.md file or skill directory." },
                    "content": { "type": "string", "description": "Raw SKILL.md source text (used instead of `path`)." },
                    "brief": { "type": "string", "description": "The original task brief behind the skill, used to ground the generated cases." },
                    "eval_cases": { "type": "integer", "description": "Number of synthetic eval cases to generate (default 10).", "minimum": 1 },
                    "model": { "type": "string", "description": "Override the model to generate with (defaults to ADEPT_MODEL)." },
                    "base_url": { "type": "string", "description": "Override the OpenAI-compatible base URL (defaults to ADEPT_BASE_URL or the OpenAI API)." }
                },
                "required": ["brief"],
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
        "eval_skill" => Ok(tool_result(eval_skill_tool(&arguments))),
        "create_skill" => Ok(tool_result(create_skill_tool(&arguments))),
        "generate_evals" => Ok(tool_result(generate_evals_tool(&arguments))),
        "score_skill" => Err((
            -32602,
            "unknown tool: score_skill was renamed to eval_skill".to_string(),
        )),
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

/// Build the skillset used for overlap detection in `eval_skill`.
///
/// Overlap detection is pairwise, so a skillset containing only the target
/// skill can never surface an overlap — the skill is compared against itself.
/// Mirror the `adept eval` CLI: discover sibling skills so the target is
/// adjudicated against its neighbours.
///
/// The search root is `directory` if given. Otherwise, for a real on-disk
/// `path` (the synthetic `"SKILL.md"` default used for raw `content` is not
/// treated as a location), it is `adept::sibling_root(path)` — the parent of
/// the skill's *own* directory, where sibling skill directories live.
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
            // Only when the caller actually passed `path`, not the synthetic
            // `read_source` default. Siblings live one level above the skill's
            // own directory (see `sibling_root`).
            arguments
                .get("path")
                .and_then(Value::as_str)
                .map(|_| sibling_root(path))
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

/// Read `select`/`ignore` arrays of analysis names from `arguments`,
/// defaulting to empty when absent or malformed (validated for real by
/// [`resolve_analyses`]).
fn read_analysis_names(arguments: &Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the `results` argument (an inline JSON array, one object per
/// `results.jsonl` line) into [`adept::evals::CaseResult`]s.
///
/// `content_mode` is `true` when the skill was supplied as raw `content`
/// rather than a filesystem `path`: an MCP client's `cwd` has no
/// corresponding real directory on the server in that case, so every
/// result's `cwd` is forced to `None` regardless of what was sent — this is
/// what makes `file_exists`/`file_contains` grade as *skipped* (naming the
/// missing directory) rather than silently passing or erroring.
fn parse_inline_results(
    arguments: &Value,
    content_mode: bool,
) -> Result<Vec<adept::evals::CaseResult>, String> {
    let items = arguments
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "`results` must be an array of result objects".to_string())?;

    let mut results = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let mut result: adept::evals::CaseResult = serde::Deserialize::deserialize(item)
            .map_err(|err| format!("results[{idx}]: {err}"))?;
        if content_mode {
            result.cwd = None;
        }
        results.push(result);
    }
    Ok(results)
}

/// Grade inline `results` against the eval dataset (an `evals` argument
/// override, or `evals/evals.jsonl` relative to `path`'s skill directory).
/// Purely offline: no LLM client is touched anywhere in this path. Shares
/// the read/validate/parse/grade sequence with the CLI's `grade_from_args`
/// via `commands::eval::{resolve_dataset_path, grade_results}`, so the two
/// surfaces can't drift on error wording.
fn grade_inline(
    arguments: &Value,
    path: &std::path::Path,
    content_mode: bool,
) -> Result<adept::evals::EvalBenchmarkReport, String> {
    let results = parse_inline_results(arguments, content_mode)?;

    let evals_override = arguments
        .get("evals")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);
    let dataset_path = crate::commands::eval::resolve_dataset_path(evals_override, path);
    crate::commands::eval::grade_results(&results, &dataset_path)
}

/// `eval_skill` MCP tool: runs whichever of the four analyses
/// (`triggering`/`token-bloat`/`overlap`/`evals`) are selected — by default,
/// whatever's available (a model for the first three, inline `results` for
/// the last) — and returns one [`EvalReport`] covering all of them. Never
/// panics or hangs: a missing/unresolvable LLM config, a malformed skill or
/// dataset, or a timed-out request all come back as a structured
/// `(text, is_error=true)` result rather than propagating a panic. Read-only
/// — this function never writes to the skill directory.
fn eval_skill_tool(arguments: &Value) -> (String, bool) {
    let (source, path) = match read_source(arguments) {
        Ok(pair) => pair,
        Err(message) => return (message, true),
    };
    // Mirrors `overlap_skillset`'s distinction: only a caller-supplied
    // `path` argument counts as "on disk", not `read_source`'s synthetic
    // `SKILL.md` default used for raw `content`.
    let content_mode = arguments.get("path").and_then(Value::as_str).is_none();

    let skill = match AnthropicSkillParser.parse_str(&path, &source) {
        Ok(skill) => skill,
        Err(err) => return (json!({ "error": err.to_string() }).to_string(), true),
    };

    let select = read_analysis_names(arguments, "select");
    let ignore = read_analysis_names(arguments, "ignore");

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
    let model_available = llm_available(&llm_config.resolve());
    let results_available = arguments.get("results").is_some();

    let selection = match resolve_analyses(&select, &ignore, model_available, results_available) {
        Ok(selection) => selection,
        Err(message) => return (json!({ "error": message }).to_string(), true),
    };

    let mut report = EvalReport::new(skill.frontmatter.name.clone());

    if needs_llm(&selection) {
        let resolved = match llm_config.resolve() {
            Ok(resolved) => resolved,
            Err(err) => {
                return (
                    json!({ "error": llm_resolution_error_message(Some("eval_skill"), err) })
                        .to_string(),
                    true,
                );
            }
        };

        let client = OpenAiCompatClient::new(resolved.clone());
        let mut options = EvalOptions::for_model(&resolved.model, adept::Tokenizer::default());
        narrow_options(&mut options, &selection);

        let skillset = if selection.contains("overlap") {
            overlap_skillset(arguments, &path, &skill)
        } else {
            Vec::new()
        };

        match run_with_timeout(
            "eval_skill",
            EVAL_TIMEOUT,
            adept_agent::eval_skill(&client, &skill, &skillset, &options),
        ) {
            Ok(llm_report) => {
                report.prompt_version = llm_report.prompt_version;
                report.triggering = llm_report.triggering;
                report.token_bloat = llm_report.token_bloat;
                report.overlaps = llm_report.overlaps;
            }
            Err(failure) => return failure,
        }
    }

    if selection.contains("evals") {
        match grade_inline(arguments, &path, content_mode) {
            Ok(benchmark) => report.evals = Some(benchmark),
            Err(message) => return (json!({ "error": message }).to_string(), true),
        }
    }

    match serde_json::to_string(&report) {
        Ok(json) => (json, false),
        Err(err) => (format!("failed to render eval report: {err}"), true),
    }
}

/// Run `future` to completion on a fresh single-use `tokio` runtime, bounded
/// by `timeout`. Shared by `create_skill_tool`, `generate_evals_tool`, and
/// `eval_skill_tool` so a runtime-start failure or a timeout is reported
/// identically across all three rather than as independently hand-rolled
/// error strings. Generic over the future's error type (`E: Display`) since
/// `create_skill`/`generate_evals` fail with `adept_agent::CreateError` while
/// `eval_skill` fails with `adept_agent::EvalError`.
fn run_with_timeout<F, T, E>(
    tool_name: &str,
    timeout: Duration,
    future: F,
) -> Result<T, (String, bool)>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let runtime = tokio::runtime::Runtime::new().map_err(|err| {
        (
            format!("failed to start async runtime for {tool_name}: {err}"),
            true,
        )
    })?;

    match runtime.block_on(async { tokio::time::timeout(timeout, future).await }) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err((json!({ "error": err.to_string() }).to_string(), true)),
        Err(_elapsed) => Err((
            json!({ "error": format!("{tool_name} timed out after {timeout:?}") }).to_string(),
            true,
        )),
    }
}

/// Build an [`LlmConfig`] from `model`/`base_url` argument overrides, then
/// resolve it. Shared by `create_skill` and `generate_evals`, whose
/// model-selection story is identical to `eval_skill`'s.
fn resolve_llm_from_arguments(arguments: &Value) -> Result<adept_agent::ResolvedLlmConfig, String> {
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
    llm_config
        .resolve()
        .map_err(|err| llm_resolution_error_message(None, err))
}

/// Build the MCP-tool error message for a failed [`adept_agent::LlmConfig::resolve`]
/// call, distinguishing [`ConfigError::MissingModel`] (no model is
/// configured at all — point at `ADEPT_MODEL`/`model`) from
/// [`ConfigError::CredentialsInBaseUrl`] (a model *is* configured, but its
/// `base_url` embeds a credential — pointing at `ADEPT_MODEL` there would be
/// actively misleading, since the fix is `api_key`/`ADEPT_API_KEY`, not a
/// model). `tool_name` is only used for the `MissingModel` wording, which
/// names the calling tool (`eval_skill`) or is generic (empty context for
/// `create_skill`/`generate_evals`, which share [`resolve_llm_from_arguments`]).
///
/// `err`'s `Display` never contains the `base_url`, so interpolating it here
/// is safe — this only improves which remedy is suggested, it does not
/// change what's redacted.
///
/// An exhaustive match, not a wildcard: a future [`ConfigError`] variant must
/// be a compile error here, not a silent fallthrough to the wrong guidance.
#[must_use]
fn llm_resolution_error_message(tool_name: Option<&str>, err: ConfigError) -> String {
    match err {
        ConfigError::MissingModel => {
            let for_tool = tool_name.map_or_else(String::new, |name| format!(" for {name}"));
            format!(
                "no LLM model configured{for_tool}: {err} (set ADEPT_MODEL, or pass a `model` argument)"
            )
        }
        ConfigError::CredentialsInBaseUrl => {
            format!("{err} (pass an `api_key` argument, or set ADEPT_API_KEY, instead of embedding it in base_url)")
        }
    }
}

/// `create_skill` MCP tool: runs the full `adept_agent::create_skill`
/// pipeline (generate -> screen -> repair, then generate and validate an
/// eval dataset) and returns the result as data.
///
/// **Preview-only, by construction, not just by test**: this function never
/// calls `write_all_transactionally` or any other filesystem-writing API.
/// The `directory` argument (defaulting to `.`) is passed straight through
/// as `create_skill`'s `out_dir`: used only as a path prefix for attributing
/// diagnostics, and, via `adept::sibling_root`, as the read-only root
/// `adept::SkillSet::discover` searches for sibling skills — the same
/// discovery `eval_skill`'s `directory` argument already performs. Nothing
/// in this function or in `adept_agent::create_skill` opens a file for
/// writing.
fn create_skill_tool(arguments: &Value) -> (String, bool) {
    let resolved = match resolve_llm_from_arguments(arguments) {
        Ok(resolved) => resolved,
        Err(err) => return (json!({ "error": err }).to_string(), true),
    };
    let client = OpenAiCompatClient::new(resolved.clone());
    create_skill_tool_with_client(arguments, &client, &resolved.model)
}

/// The client-parameterized core of `create_skill`, kept separate from
/// [`create_skill_tool`] so tests can drive it with
/// `adept_agent::MockLlmClient` instead of a real network client.
fn create_skill_tool_with_client(
    arguments: &Value,
    client: &dyn LlmClient,
    model: &str,
) -> (String, bool) {
    let brief = match arguments.get("brief").and_then(Value::as_str) {
        Some(brief) if !brief.trim().is_empty() => brief,
        _ => return ("missing or empty `brief`".to_string(), true),
    };

    let out_dir = arguments
        .get("directory")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let tokenizer = adept::Tokenizer::default();
    let mut options = CreateOptions::for_model(model, tokenizer);
    match bounded_u64_argument(arguments, "max_rounds", MIN_MAX_ROUNDS, MAX_MAX_ROUNDS) {
        Ok(Some(max_rounds)) => options.max_rounds = max_rounds as usize,
        Ok(None) => {}
        Err(message) => return (message, true),
    }
    match bounded_u64_argument(arguments, "eval_cases", MIN_EVAL_CASES, MAX_EVAL_CASES) {
        Ok(Some(eval_cases)) => options.eval_cases = eval_cases as usize,
        Ok(None) => {}
        Err(message) => return (message, true),
    }
    options.name_override = arguments
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);

    match run_with_timeout(
        "create_skill",
        CREATE_TIMEOUT,
        create_skill(client, brief, &out_dir, &options),
    ) {
        Ok(report) => (render_create_report(&report), false),
        Err(failure) => failure,
    }
}

/// Render a [`adept_agent::CreateReport`] as the tool's JSON payload: the
/// generated file contents keyed by path, as data only (never written by
/// this process). Uses `CreateReport`'s own `Serialize` derive directly, the
/// same encoding `adept create --format json` uses (`commands::create`'s
/// `render_json`), so the two surfaces can never silently diverge.
fn render_create_report(report: &adept_agent::CreateReport) -> String {
    // `to_string` on a type that derives `Serialize` fails only on writer
    // errors, which a `String` buffer never produces.
    serde_json::to_string(report).expect("CreateReport serialization is infallible")
}

/// `generate_evals` MCP tool: given an existing skill (`path` or `content`)
/// and the original task brief, calls `adept_agent::generate_evals` — the
/// same eval-dataset generation step `create_skill` uses internally — and
/// returns the (already-validated) result as data.
///
/// Preview-only, by construction: this function only parses arguments,
/// dispatches to the shared library function, and shapes its output as
/// JSON; it never opens a file for writing.
fn generate_evals_tool(arguments: &Value) -> (String, bool) {
    let resolved = match resolve_llm_from_arguments(arguments) {
        Ok(resolved) => resolved,
        Err(err) => return (json!({ "error": err }).to_string(), true),
    };
    let client = OpenAiCompatClient::new(resolved.clone());
    generate_evals_tool_with_client(arguments, &client, &resolved.model)
}

/// The client-parameterized core of `generate_evals`, kept separate from
/// [`generate_evals_tool`] so tests can drive it with
/// `adept_agent::MockLlmClient` instead of a real network client.
fn generate_evals_tool_with_client(
    arguments: &Value,
    client: &dyn LlmClient,
    model: &str,
) -> (String, bool) {
    let brief = match arguments.get("brief").and_then(Value::as_str) {
        Some(brief) if !brief.trim().is_empty() => brief,
        _ => return ("missing or empty `brief`".to_string(), true),
    };

    let (source, path) = match read_source(arguments) {
        Ok(pair) => pair,
        Err(message) => return (message, true),
    };
    let skill = match AnthropicSkillParser.parse_str(&path, &source) {
        Ok(skill) => skill,
        Err(err) => return (json!({ "error": err.to_string() }).to_string(), true),
    };

    let mut options = CreateOptions::for_model(model, adept::Tokenizer::default());
    match bounded_u64_argument(arguments, "eval_cases", MIN_EVAL_CASES, MAX_EVAL_CASES) {
        Ok(Some(eval_cases)) => options.eval_cases = eval_cases as usize,
        Ok(None) => options.eval_cases = DEFAULT_GENERATE_EVALS_CASES,
        Err(message) => return (message, true),
    }

    match run_with_timeout(
        "generate_evals",
        GENERATE_EVALS_TIMEOUT,
        adept_agent::generate_evals(client, &skill, brief, &options),
    ) {
        Ok(cases) => {
            let jsonl = adept::evals::to_jsonl(&cases);
            (
                json!({
                    "eval_cases": cases,
                    "jsonl": jsonl,
                })
                .to_string(),
                false,
            )
        }
        Err(failure) => failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adept_agent::MockLlmClient;
    use std::sync::Mutex;

    const SAMPLE_SKILL: &str = "---\nname: sample\ndescription: does a thing. Use when the user asks for a thing. Do not use otherwise.\n---\n\n# Sample\n\nBody text.\n";

    /// Serializes tests that mutate process-wide `ADEPT_*` env vars, since
    /// `cargo test` runs this crate's unit tests on multiple threads in one
    /// process.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    use crate::test_fixtures::{
        clean_body, clean_description, valid_eval_json, valid_generate_json,
    };

    /// Recursively list every path under `dir`, for before/after snapshots
    /// asserting a preview-only tool wrote nothing.
    fn list_all(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(list_all(&path));
            } else {
                out.push(path);
            }
        }
        out.sort();
        out
    }

    #[test]
    fn tools_list_exposes_create_skill_and_generate_evals_when_model_configured() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("ADEPT_MODEL", "test-model");
        let request = json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/list" });
        let response = handle_message(&request.to_string()).expect("expected a response");
        std::env::remove_var("ADEPT_MODEL");

        let parsed: Value = serde_json::from_str(&response).unwrap();
        let tools = parsed["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"create_skill"), "names: {names:?}");
        assert!(names.contains(&"generate_evals"), "names: {names:?}");
    }

    /// Pins the schema field-by-field: neither tool may ever grow an
    /// output-path argument (an `out`/`out_dir`/`output_path`/... field), so
    /// a future edit adding write capability fails this test rather than
    /// silently shipping.
    #[test]
    fn create_skill_and_generate_evals_schemas_have_no_output_path_argument() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("ADEPT_MODEL", "test-model");
        let request = json!({ "jsonrpc": "2.0", "id": 11, "method": "tools/list" });
        let response = handle_message(&request.to_string()).expect("expected a response");
        std::env::remove_var("ADEPT_MODEL");

        let parsed: Value = serde_json::from_str(&response).unwrap();
        let tools = parsed["result"]["tools"].as_array().unwrap();

        let create = tools
            .iter()
            .find(|t| t["name"] == "create_skill")
            .expect("create_skill tool");
        let create_props: std::collections::BTreeSet<&str> = create["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            create_props,
            std::collections::BTreeSet::from([
                "brief",
                "directory",
                "name",
                "model",
                "base_url",
                "max_rounds",
                "eval_cases",
            ]),
            "create_skill schema must be exactly this field set: {create_props:?}"
        );

        let generate = tools
            .iter()
            .find(|t| t["name"] == "generate_evals")
            .expect("generate_evals tool");
        let generate_props: std::collections::BTreeSet<&str> = generate["inputSchema"]
            ["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            generate_props,
            std::collections::BTreeSet::from([
                "path",
                "content",
                "brief",
                "eval_cases",
                "model",
                "base_url",
            ]),
            "generate_evals schema must be exactly this field set: {generate_props:?}"
        );

        // Belt and braces: no field name on either schema even looks like an
        // output/write path, whatever exact set the assertions above pin.
        for tool_props in [&create_props, &generate_props] {
            for name in tool_props {
                let lower = name.to_ascii_lowercase();
                assert!(
                    !lower.contains("out") && !lower.contains("write") && !lower.contains("dest"),
                    "field `{name}` looks like an output-path argument"
                );
            }
        }
    }

    #[test]
    fn create_skill_tool_returns_generated_skill_and_eval_dataset_without_touching_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let before = list_all(tmp.path());
        assert!(before.is_empty());

        let good = valid_generate_json("demo-skill", clean_description(), clean_body());
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![good, eval]);

        // `directory` is the skill's own would-be directory (so its basename
        // must match the model-chosen name for SL004 to stay clean), not the
        // siblings root; `tmp.path()` (its parent) is what gets discovered
        // for siblings and is asserted empty below.
        let out_dir = tmp.path().join("demo-skill");
        let arguments = json!({
            "brief": "Extract PDF form data",
            "directory": out_dir.to_str().unwrap(),
        });
        let (text, is_error) = create_skill_tool_with_client(&arguments, &mock, "test-model");
        assert!(!is_error, "tool call failed: {text}");

        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["skill_name"], "demo-skill");
        assert_eq!(parsed["eval_cases"].as_array().unwrap().len(), 10);
        assert!(parsed["files"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.ends_with("SKILL.md")));
        assert!(parsed["files"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.ends_with("evals.jsonl")));

        // The central invariant: preview-only, so the directory used for
        // sibling discovery must be exactly as it started — no SKILL.md, no
        // evals/ directory, nothing at all.
        let after = list_all(tmp.path());
        assert_eq!(
            before, after,
            "create_skill must never write to disk, found: {after:?}"
        );
    }

    #[test]
    fn generate_evals_tool_returns_validated_dataset_without_touching_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let before = list_all(tmp.path());
        assert!(before.is_empty());

        let eval = valid_eval_json(5);
        let mock = MockLlmClient::with_texts(vec![eval]);

        let arguments = json!({
            "content": SAMPLE_SKILL,
            "brief": "Do a thing for the user",
            "eval_cases": 5,
        });
        let (text, is_error) = generate_evals_tool_with_client(&arguments, &mock, "test-model");
        assert!(!is_error, "tool call failed: {text}");

        let parsed: Value = serde_json::from_str(&text).unwrap();
        let cases = parsed["eval_cases"].as_array().unwrap();
        assert_eq!(cases.len(), 5);
        let jsonl = parsed["jsonl"].as_str().unwrap();
        adept::evals::validate(jsonl).expect("returned dataset must already be valid");

        let after = list_all(tmp.path());
        assert_eq!(
            before, after,
            "generate_evals must never write to disk, found: {after:?}"
        );
    }

    #[test]
    fn create_skill_tool_rejects_out_of_range_max_rounds_and_eval_cases() {
        let mock = MockLlmClient::with_texts(Vec::<String>::new());

        let too_many_rounds = json!({
            "brief": "Extract PDF form data",
            "max_rounds": MAX_MAX_ROUNDS + 1,
        });
        let (text, is_error) = create_skill_tool_with_client(&too_many_rounds, &mock, "test-model");
        assert!(is_error, "out-of-range max_rounds must be rejected");
        assert!(text.contains("max_rounds"), "text: {text}");

        let too_many_cases = json!({
            "brief": "Extract PDF form data",
            "eval_cases": MAX_EVAL_CASES + 1,
        });
        let (text, is_error) = create_skill_tool_with_client(&too_many_cases, &mock, "test-model");
        assert!(is_error, "out-of-range eval_cases must be rejected");
        assert!(text.contains("eval_cases"), "text: {text}");

        let zero_rounds = json!({
            "brief": "Extract PDF form data",
            "max_rounds": 0,
        });
        let (text, is_error) = create_skill_tool_with_client(&zero_rounds, &mock, "test-model");
        assert!(is_error, "zero max_rounds must be rejected");
        assert!(text.contains("max_rounds"), "text: {text}");
    }

    #[test]
    fn generate_evals_tool_rejects_out_of_range_eval_cases() {
        let mock = MockLlmClient::with_texts(Vec::<String>::new());
        let arguments = json!({
            "content": SAMPLE_SKILL,
            "brief": "Do a thing for the user",
            "eval_cases": MAX_EVAL_CASES + 1,
        });
        let (text, is_error) = generate_evals_tool_with_client(&arguments, &mock, "test-model");
        assert!(is_error, "out-of-range eval_cases must be rejected");
        assert!(text.contains("eval_cases"), "text: {text}");
    }

    #[test]
    fn generate_evals_tool_rejects_dataset_that_fails_validation() {
        let empty_eval = json!({ "cases": [] }).to_string();
        let mock = MockLlmClient::with_texts(vec![empty_eval]);

        let arguments = json!({
            "content": SAMPLE_SKILL,
            "brief": "Do a thing for the user",
        });
        let (text, is_error) = generate_evals_tool_with_client(&arguments, &mock, "test-model");
        assert!(is_error, "an empty dataset must be reported as an error");
        assert!(text.contains("validation"), "text: {text}");
    }

    #[test]
    fn handle_message_tools_call_create_skill_without_model_is_a_pure_synchronous_error() {
        // No `ADEPT_MODEL` in the process env and no `model` argument: this
        // must fail during config resolution, before any client is built or
        // any I/O (network or filesystem) is attempted — `handle_message`
        // itself never touches the network or the filesystem.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ADEPT_MODEL");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": { "name": "create_skill", "arguments": { "brief": "Do the thing" } }
        });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["result"]["isError"], true);
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("ADEPT_MODEL"), "text: {text}");
    }

    #[test]
    fn handle_message_tools_call_generate_evals_without_model_is_a_pure_synchronous_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ADEPT_MODEL");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "tools/call",
            "params": {
                "name": "generate_evals",
                "arguments": { "content": SAMPLE_SKILL, "brief": "Do the thing" }
            }
        });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["result"]["isError"], true);
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("ADEPT_MODEL"), "text: {text}");
    }

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

    /// `check_skill` uses `LintConfig::default()` and does not discover
    /// `adept.toml` — pins the intentional divergence documented in
    /// `docs/ARCHI.md` §12: the CLI is the only config-aware entry point.
    /// An `adept.toml` sitting right next to the skill on disk, raising
    /// `description_min_tokens` far above the default (6), must have no
    /// effect on a `path`-based `check_skill` call.
    #[test]
    fn check_skill_does_not_discover_adept_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[lint]\ndescription_min_tokens = 1000\n",
        )
        .unwrap();
        let skill_path = dir.path().join("SKILL.md");
        std::fs::write(&skill_path, SAMPLE_SKILL).unwrap();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/call",
            "params": { "name": "check_skill", "arguments": { "path": skill_path.to_str().unwrap() } }
        });
        let response = handle_message(&request.to_string()).expect("expected a response");
        let parsed: Value = serde_json::from_str(&response).unwrap();
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        let diagnostics: Value = serde_json::from_str(text).unwrap();
        let codes: Vec<&str> = diagnostics
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect();
        assert!(
            !codes.contains(&"SL201"),
            "check_skill must use LintConfig::default(), not the adept.toml sitting beside the skill: {codes:?}"
        );
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

    /// [`write_skill`] plus a one-case `evals/evals.jsonl` sidecar, so the
    /// grading path has something to grade. Shared by the tests that drive
    /// `eval_skill` — the dataset is schema-versioned, so a bump should not
    /// need editing in several places.
    fn write_skill_with_evals(
        dir: &std::path::Path,
        name: &str,
        description: &str,
    ) -> std::path::PathBuf {
        let path = write_skill(dir, name, description);
        let evals_dir = dir.join(name).join("evals");
        std::fs::create_dir_all(&evals_dir).unwrap();
        std::fs::write(
            evals_dir.join("evals.jsonl"),
            "{\"schema_version\":1,\"prompt\":\"p\",\"assertions\":[{\"kind\":\"contains\",\"value\":\"ok\"}]}\n",
        )
        .unwrap();
        path
    }

    /// The four properties every credential-error response must have: it is
    /// an error, it names the real remedy, it is not mis-framed as a missing
    /// model, and it never echoes the credential. Shared so a new tool's
    /// coverage cannot accidentally assert only three of the four.
    fn assert_credential_error(text: &str, is_error: bool) {
        assert!(
            is_error,
            "a credential-bearing base_url must be reported as an error, not silently downgraded: {text}"
        );
        assert!(
            text.contains("api_key") || text.contains("credentials"),
            "error must name the real problem: {text}"
        );
        assert!(
            !text.contains("ADEPT_MODEL") && !text.contains("no LLM model configured"),
            "a credential error must not be framed as a missing model: {text}"
        );
        assert!(
            !text.contains("hunter2secret") && !text.contains("alice:hunter2secret@"),
            "response must never contain the credential: {text}"
        );
    }

    #[test]
    fn overlap_skillset_discovers_siblings_from_path_parent() {
        let root = tempfile::tempdir().unwrap();
        let target_path = write_skill(root.path(), "alpha", "Does alpha things. Use when alpha.");
        write_skill(root.path(), "beta", "Does beta things. Use when beta.");

        let (source, path) =
            read_source(&json!({ "path": target_path.to_str().unwrap() })).unwrap();
        let skill = AnthropicSkillParser.parse_str(&path, &source).unwrap();

        let skillset = overlap_skillset(
            &json!({ "path": target_path.to_str().unwrap() }),
            &path,
            &skill,
        );

        let names: Vec<&str> = skillset
            .iter()
            .map(|s| s.frontmatter.name.as_str())
            .collect();
        assert!(
            names.contains(&"alpha"),
            "target must be present: {names:?}"
        );
        assert!(
            names.contains(&"beta"),
            "sibling must be discovered: {names:?}"
        );
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

        let names: Vec<&str> = skillset
            .iter()
            .map(|s| s.frontmatter.name.as_str())
            .collect();
        assert!(names.contains(&"gamma"), "directory sibling: {names:?}");
        assert!(names.contains(&"sample"), "target appended: {names:?}");
    }

    /// The guarantee item 3 in the cleanup brief exists for: `adept create
    /// --format json` (`commands::create::render_json`) and the
    /// `create_skill` MCP tool (`render_create_report`) must serialize the
    /// same `CreateReport` identically, since both now derive straight from
    /// `CreateReport`'s own `Serialize` rather than two hand-written
    /// encodings that could silently diverge on a new field.
    #[tokio::test]
    async fn create_report_json_is_identical_across_cli_and_mcp_surfaces() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let good = valid_generate_json("demo-skill", clean_description(), clean_body());
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![good, eval]);
        let options = CreateOptions::for_model("test-model", adept::Tokenizer::O200kBase);
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        let mcp_json = render_create_report(&report);
        let cli_json = crate::commands::create::render_json(&report).expect("render_json");

        let mcp_value: Value = serde_json::from_str(&mcp_json).unwrap();
        let cli_value: Value = serde_json::from_str(&cli_json).unwrap();
        assert_eq!(
            mcp_value, cli_value,
            "create_skill MCP tool and `adept create --format json` must emit the same JSON shape"
        );
    }

    #[test]
    fn notification_with_no_id_produces_no_response() {
        let request = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_message(&request.to_string()).is_none());
    }

    /// Pins the divergence from `create_skill`/`generate_evals`: `eval_skill`
    /// must be advertised even with no model configured at all, since its
    /// `evals` (grading) analysis needs no `ADEPT_MODEL`. Uses the same
    /// `ENV_LOCK` serialization as the other tests that mutate the
    /// process-wide `ADEPT_MODEL` var, so it can't race them.
    #[test]
    fn tools_list_advertises_eval_skill_even_with_no_model_configured() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ADEPT_MODEL");
        std::env::remove_var("ADEPT_BASE_URL");
        std::env::remove_var("ADEPT_API_KEY");

        let request = json!({ "jsonrpc": "2.0", "id": 20, "method": "tools/list" });
        let response = handle_message(&request.to_string()).expect("expected a response");

        let parsed: Value = serde_json::from_str(&response).unwrap();
        let tools = parsed["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(
            names.contains(&"eval_skill"),
            "eval_skill must be advertised with no model configured: {names:?}"
        );
        assert!(
            !names.contains(&"create_skill"),
            "create_skill must stay gated: {names:?}"
        );
        assert!(
            !names.contains(&"generate_evals"),
            "generate_evals must stay gated: {names:?}"
        );
    }

    /// A grading-only `eval_skill` call (no model configured, `results`
    /// supplied) must succeed — this is the offline path the whole feature
    /// exists to support.
    #[test]
    fn eval_skill_grading_only_call_succeeds_with_no_model_configured() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ADEPT_MODEL");
        std::env::remove_var("ADEPT_BASE_URL");
        std::env::remove_var("ADEPT_API_KEY");

        let dir = tempfile::tempdir().unwrap();
        let path =
            write_skill_with_evals(dir.path(), "demo", "Does a demo thing. Use when demoing.");

        let arguments = json!({
            "path": path.to_str().unwrap(),
            "results": [ { "case": 1, "response": "it is ok" } ],
        });
        let (text, is_error) = eval_skill_tool(&arguments);
        assert!(!is_error, "grading-only eval_skill call failed: {text}");
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert!(parsed.get("triggering").is_none());
        assert_eq!(parsed["evals"]["pass_rate"], 1.0);
    }

    /// Regression test for the MCP counterpart of the CLI fix in commit
    /// d6698ef: `eval_skill` with a credential-bearing `base_url` and no
    /// `select` must not have `model_available` collapse to `false` (which
    /// would make `needs_llm` false, `resolve()` never get called again at
    /// `mcp.rs:577`, and the tool return a *successful* result that
    /// silently ran only the offline analyses). It must instead reach the
    /// real `resolve()` and report the credential error, without ever
    /// putting the secret in the response.
    #[test]
    fn eval_skill_with_credentials_in_base_url_reports_the_error_not_a_silent_success() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ADEPT_MODEL");
        std::env::remove_var("ADEPT_BASE_URL");
        std::env::remove_var("ADEPT_API_KEY");

        let dir = tempfile::tempdir().unwrap();
        let path =
            write_skill_with_evals(dir.path(), "demo", "Does a demo thing. Use when demoing.");

        let arguments = json!({
            "path": path.to_str().unwrap(),
            "model": "test-model",
            "base_url": "https://alice:hunter2secret@gw.example/v1",
        });
        let (text, is_error) = eval_skill_tool(&arguments);
        assert_credential_error(&text, is_error);
    }

    /// Sibling of the `eval_skill` case above, for the
    /// `resolve_llm_from_arguments` path shared by `create_skill` and
    /// `generate_evals`: a credential-bearing `base_url` must be reported as
    /// an `api_key` problem, never as "no LLM model configured" pointing at
    /// `ADEPT_MODEL`.
    #[test]
    fn create_skill_tool_with_credentials_in_base_url_names_api_key_not_a_missing_model() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ADEPT_MODEL");
        std::env::remove_var("ADEPT_BASE_URL");
        std::env::remove_var("ADEPT_API_KEY");

        let arguments = json!({
            "brief": "Extract PDF form data",
            "model": "test-model",
            "base_url": "https://alice:hunter2secret@gw.example/v1",
        });
        let (text, is_error) = create_skill_tool(&arguments);
        assert_credential_error(&text, is_error);
    }

    /// `eval_skill` with `content` (no real skill directory) must grade
    /// `contains` and report `file_exists`/`file_contains` as `skipped`
    /// naming the missing directory — not as a pass, and not as an error.
    #[test]
    fn eval_skill_with_content_skips_file_assertions_naming_missing_directory() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ADEPT_MODEL");
        std::env::remove_var("ADEPT_BASE_URL");
        std::env::remove_var("ADEPT_API_KEY");

        let arguments = json!({
            "content": SAMPLE_SKILL,
            "results": [ { "case": 1, "response": "it is ok" } ],
            "evals": "does-not-matter.jsonl",
        });

        // Write the referenced dataset next to nothing real: use a temp dir
        // so the dataset itself is readable, but `content` mode still has no
        // real skill directory for `cwd`-relative file assertions.
        let dir = tempfile::tempdir().unwrap();
        let dataset_path = dir.path().join("evals.jsonl");
        std::fs::write(
            &dataset_path,
            "{\"schema_version\":1,\"prompt\":\"p\",\"assertions\":[\
             {\"kind\":\"contains\",\"value\":\"ok\"},\
             {\"kind\":\"file_exists\",\"path\":\"out.txt\"}]}\n",
        )
        .unwrap();
        let arguments = {
            let mut arguments = arguments;
            arguments["evals"] = json!(dataset_path.to_str().unwrap());
            arguments
        };

        let (text, is_error) = eval_skill_tool(&arguments);
        assert!(!is_error, "eval_skill with content failed: {text}");
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let evals = &parsed["evals"];
        assert_eq!(evals["assertions_checked"], 1);
        assert_eq!(evals["assertions_met"], 1);
        assert_eq!(evals["assertions_skipped"], 1);
        assert!(
            evals["skipped_reasons"]
                .as_object()
                .unwrap()
                .keys()
                .any(|reason| reason.contains("cwd")),
            "skipped reason must name the missing directory: {evals:?}"
        );
        // Not reported as a pass: the file assertion being skipped must not
        // silently look like it passed.
        assert_eq!(evals["cases"][0]["assertions"][1]["outcome"], "skipped");
    }
}
