//! `adept mcp`: a Model Context Protocol server over stdio.
//!
//! Implementation choice: this implements the JSON-RPC 2.0 stdio transport
//! directly (newline-delimited JSON messages, per the MCP spec) rather than
//! depending on the `rmcp` SDK crate. `adept mcp` exposes five tools
//! (`check_skill`, `format_skill`, `score_skill`, `create_skill`,
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
use adept_fmt::{format_str, FmtConfig};
use adept_score::{LlmClient, LlmConfig, OpenAiCompatClient, ScoreOptions};
use serde_json::{json, Value};

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

/// How long `score_skill` will wait for the LLM backend before giving up.
const SCORE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `create_skill` will wait for the LLM backend before giving up.
/// Longer than [`SCORE_TIMEOUT`]: `create` may make several authoring/repair
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

    // Only advertise `score_skill`/`create_skill`/`generate_evals` when an
    // LLM backend can actually be resolved (network-backed; requires
    // `ADEPT_MODEL` etc.) so agents don't discover a tool that's guaranteed
    // to fail. Resolved once and shared: all three tools gate on the exact
    // same resolution.
    let llm_configured = LlmConfig::default().resolve().is_ok();

    if llm_configured {
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

    // `create_skill`/`generate_evals` are also network-backed (same
    // `ADEPT_MODEL` resolution as `score_skill`) and preview-only: neither
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
        "score_skill" => Ok(tool_result(score_skill_tool(&arguments))),
        "create_skill" => Ok(tool_result(create_skill_tool(&arguments))),
        "generate_evals" => Ok(tool_result(generate_evals_tool(&arguments))),
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

/// Run `future` to completion on a fresh single-use `tokio` runtime, bounded
/// by `timeout`. Shared by `create_skill_tool`/`generate_evals_tool` — the
/// two MCP tools added alongside this helper — so a runtime-start failure or
/// a timeout is reported identically for both rather than as two
/// independently hand-rolled error strings. `score_skill_tool` predates this
/// helper and keeps its own copy of the same idiom; left untouched here.
fn run_with_timeout<F, T>(
    tool_name: &str,
    timeout: Duration,
    future: F,
) -> Result<T, (String, bool)>
where
    F: std::future::Future<Output = Result<T, adept_agent::CreateError>>,
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
/// model-selection story is identical to `score_skill`'s.
fn resolve_llm_from_arguments(arguments: &Value) -> Result<adept_score::ResolvedLlmConfig, String> {
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
    llm_config.resolve().map_err(|err| {
        format!("no LLM model configured: {err} (set ADEPT_MODEL, or pass a `model` argument)")
    })
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
/// discovery `score_skill`'s `directory` argument already performs. Nothing
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
/// `adept_score::MockLlmClient` instead of a real network client.
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
/// `adept_score::MockLlmClient` instead of a real network client.
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
    use adept_score::MockLlmClient;
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
}
