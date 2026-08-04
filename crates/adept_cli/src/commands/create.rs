//! `adept create`.
//!
//! Wires `adept_agent::create::create_skill` into the CLI: input collection
//! (`--from-file` > stdin > interactive prompt), config/flag resolution,
//! preview-by-default rendering, and the transactional write.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use adept_agent::{create_skill, write_all_transactionally, CreateOptions, CreateOutcome};
use adept_agent::{ResolvedLlmConfig, RunMetadata};

use crate::cli::{CreateArgs, OutputFormat};
use crate::config::{
    attach_capture, build_runtime, resolve_llm_client, shared_sources, value_source, AdeptConfig,
};

/// Exit code contract: 0 = clean candidate, 1 = findings remain (best-effort
/// candidate still written under `--write`), 2 = usage/I/O error.
pub const EXIT_OK: i32 = 0;
pub const EXIT_FINDINGS: i32 = 1;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Run `adept create`, building its own `tokio` runtime and a real
/// [`adept_agent::OpenAiCompatClient`]. Returns the process exit code.
pub fn run(args: &CreateArgs, config: &AdeptConfig, quiet: bool) -> i32 {
    let brief = match resolve_brief(args) {
        Ok(brief) => brief,
        Err(code) => return code,
    };

    let out_dir = args.out.clone().unwrap_or_else(|| PathBuf::from("."));

    if out_dir.join("SKILL.md").is_file() && !args.overwrite {
        eprintln!(
            "adept: error: {} already contains a SKILL.md; pass --overwrite to replace it",
            out_dir.display()
        );
        return EXIT_USAGE_ERROR;
    }

    let base_url = args
        .base_url
        .clone()
        .or_else(|| config.create.base_url.clone());
    let model = args.model.clone().or_else(|| config.create.model.clone());
    let Some((client, resolved)) = resolve_llm_client("create", base_url, model) else {
        return EXIT_USAGE_ERROR;
    };

    let tokenizer = args
        .tokenizer
        .map(adept::Tokenizer::from)
        .or(config.create.tokenizer)
        .unwrap_or_default();

    let options = build_options(args, config, &resolved.model, tokenizer);

    let (client, sink) = match attach_capture(
        client,
        args.capture_dir.as_deref(),
        config.create.capture_dir.as_deref(),
        config.origin_dir.as_deref(),
        |source| capture_metadata(args, config, &resolved, tokenizer, &options, source),
    ) {
        Ok(pair) => pair,
        Err(exit_code) => return exit_code,
    };

    let exit_code = execute(args, quiet, &client, &brief, &out_dir, &options);
    if let Some(sink) = &sink {
        sink.finalize(exit_code);
    }
    exit_code
}

/// Collect the task brief, in precedence order: `--from-file`, then stdin
/// when it is not a TTY, then an interactive multi-line prompt. Returns
/// `Err(2)` (having already printed a usage message) when none is available.
fn resolve_brief(args: &CreateArgs) -> Result<String, i32> {
    if let Some(path) = &args.from_file {
        return std::fs::read_to_string(path).map_err(|err| {
            eprintln!(
                "adept: error: failed to read brief from {}: {err}",
                path.display()
            );
            EXIT_USAGE_ERROR
        });
    }

    if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        if let Err(err) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("adept: error: failed to read brief from stdin: {err}");
            return Err(EXIT_USAGE_ERROR);
        }
        if !buf.trim().is_empty() {
            return Ok(buf);
        }
        eprintln!(
            "adept: error: no task brief provided. Pass --from-file <path>, or pipe a brief on stdin."
        );
        return Err(EXIT_USAGE_ERROR);
    }

    read_brief_interactively()
}

/// The untested surface: an in-terminal, multi-line, Claude-Code-session-style
/// prompt for a brief, backed by `rustyline` for line editing. Ctrl-C aborts
/// with exit 2, writing nothing. Kept thin: it only collects a string and
/// hands it to the same code path `--from-file` uses.
fn read_brief_interactively() -> Result<String, i32> {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    eprintln!("adept create: enter your brief below.");
    eprintln!("Write as many lines as you like; submit with Ctrl-D, cancel with Ctrl-C.");

    let mut editor = DefaultEditor::new().map_err(|err| {
        eprintln!(
            "adept: error: failed to start interactive prompt: {err}. Pass --from-file <path>, or pipe a brief on stdin."
        );
        EXIT_USAGE_ERROR
    })?;

    let mut lines = Vec::new();
    loop {
        match editor.readline("> ") {
            Ok(line) => lines.push(line),
            Err(ReadlineError::Eof) => break,
            Err(ReadlineError::Interrupted) => return Err(EXIT_USAGE_ERROR),
            Err(err) => {
                eprintln!("adept: error: interactive prompt failed: {err}");
                return Err(EXIT_USAGE_ERROR);
            }
        }
    }

    let brief = lines.join("\n");
    if brief.trim().is_empty() {
        eprintln!("adept: error: no task brief provided.");
        return Err(EXIT_USAGE_ERROR);
    }
    Ok(brief)
}

fn build_options(
    args: &CreateArgs,
    config: &AdeptConfig,
    model: &str,
    tokenizer: adept::Tokenizer,
) -> CreateOptions {
    let mut options = CreateOptions::for_model(model, tokenizer);
    options.max_rounds = args
        .max_rounds
        .or(config.create.max_rounds)
        .unwrap_or(adept_agent::create::DEFAULT_MAX_ROUNDS);
    options.eval_cases = config
        .create
        .eval_cases
        .unwrap_or(adept_agent::create::DEFAULT_EVAL_CASES);

    let mut lint_config = config.lint.clone();
    lint_config.tokenizer = tokenizer;
    options.lint_config = lint_config;
    options.fmt_config = config.fmt.clone();
    options.name_override = args.name.clone();

    options
}

fn execute(
    args: &CreateArgs,
    quiet: bool,
    client: &dyn adept_agent::LlmClient,
    brief: &str,
    out_dir: &Path,
    options: &CreateOptions,
) -> i32 {
    let Some(runtime) = build_runtime() else {
        return EXIT_USAGE_ERROR;
    };

    let report = match runtime.block_on(create_skill(client, brief, out_dir, options)) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("adept: error: create failed: {err}");
            return EXIT_USAGE_ERROR;
        }
    };

    match args.format {
        OutputFormat::Json => match render_json(&report) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("adept: error: failed to render JSON: {err}");
                return EXIT_USAGE_ERROR;
            }
        },
        OutputFormat::Human => print!("{}", render_human(&report)),
    }

    if args.write {
        if let Err(err) = write_all_transactionally(&report.files) {
            eprintln!("adept: error: failed to write generated skill: {err}");
            return EXIT_USAGE_ERROR;
        }
        if !quiet {
            println!(
                "wrote {} file{} to {}",
                report.files.len(),
                if report.files.len() == 1 { "" } else { "s" },
                out_dir.display()
            );
        }
    }

    // The failure-visibility invariant: a summary line at the very end of
    // output, never only diagnostics scrolled past above the write
    // confirmation.
    if !report.is_clean() {
        let total = report.candidate_diagnostics.len() + report.new_sibling_diagnostics.len();
        eprintln!(
            "adept create: {total} diagnostic{} remain on {} after {} round{} ({})",
            if total == 1 { "" } else { "s" },
            report.skill_name,
            report.rounds_used,
            if report.rounds_used == 1 { "" } else { "s" },
            if args.write {
                "written anyway"
            } else {
                "would be written under --write"
            },
        );
    }

    match report.outcome {
        CreateOutcome::Clean => EXIT_OK,
        CreateOutcome::BestEffort => EXIT_FINDINGS,
    }
}

fn render_human(report: &adept_agent::CreateReport) -> String {
    let mut out = format!("adept create: {}\n", report.skill_name);

    if !report.siblings_found {
        out.push_str("(no sibling skills found)\n");
    }

    out.push_str(&format!(
        "{} round{} used\n",
        report.rounds_used,
        if report.rounds_used == 1 { "" } else { "s" }
    ));

    if report.candidate_diagnostics.is_empty() && report.new_sibling_diagnostics.is_empty() {
        out.push_str("0 diagnostics remaining\n");
    } else {
        for d in &report.candidate_diagnostics {
            out.push_str(&format!("  {} ({}): {}\n", d.code, d.severity, d.message));
        }
        for d in &report.new_sibling_diagnostics {
            out.push_str(&format!(
                "  {} ({}) [sibling]: {}\n",
                d.code, d.severity, d.message
            ));
        }
    }

    out.push_str(&format!(
        "{} eval case(s) generated\n",
        report.eval_cases.len()
    ));
    if report.duplicate_eval_cases_dropped > 0 {
        out.push_str(&format!(
            "{} duplicate eval case(s) dropped\n",
            report.duplicate_eval_cases_dropped
        ));
    }
    for case in &report.eval_cases {
        let kinds: Vec<&str> = case
            .assertions
            .iter()
            .map(adept::evals::Assertion::kind)
            .collect();
        let kinds = if kinds.is_empty() {
            "no assertions".to_string()
        } else {
            kinds.join(", ")
        };
        out.push_str(&format!("  - {} [{}]\n", case.prompt, kinds));
    }
    out.push('\n');

    let empty: BTreeMap<PathBuf, String> = BTreeMap::new();
    let diff = adept_agent::diff::render_multi_file_diff(&empty, &report.files);
    out.push_str(&diff);

    out
}

pub(crate) fn render_json(report: &adept_agent::CreateReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

/// Describe the run for `run_metadata.json`.
fn capture_metadata(
    args: &CreateArgs,
    config: &AdeptConfig,
    resolved: &ResolvedLlmConfig,
    tokenizer: adept::Tokenizer,
    options: &CreateOptions,
    capture_dir_source: &'static str,
) -> RunMetadata {
    let mut metadata = RunMetadata::new("create");
    // `create` uses its own two prompts (authoring + eval generation), not
    // the `eval` analyses' prompts, so record both here rather than leaving the generic
    // `adept_agent::PROMPT_VERSION` `RunMetadata::new` stamped by default —
    // otherwise a captured `create` run would say which `score`/`fix` prompt
    // version was in effect, not which prompts actually produced it.
    metadata.prompt_version = format!(
        "create_authoring={}, create_eval={}",
        adept_agent::CREATE_AUTHORING_PROMPT_VERSION,
        adept_agent::CREATE_EVAL_PROMPT_VERSION,
    );
    metadata.model = Some(resolved.model.clone());
    metadata.base_url = Some(resolved.base_url.clone());
    metadata.tokenizer = Some(tokenizer.to_string());
    metadata.api_key_present = resolved.api_key.is_some();
    metadata.max_rounds = Some(options.max_rounds);
    metadata.target_path = Some(
        args.out
            .clone()
            .unwrap_or_else(|| PathBuf::from("."))
            .display()
            .to_string(),
    );

    metadata.sources = shared_sources(
        args.model.is_some(),
        config.create.model.is_some(),
        args.base_url.is_some(),
        config.create.base_url.is_some(),
        args.tokenizer.is_some(),
        config.create.tokenizer.is_some(),
    );
    metadata.sources.extend([
        (
            "max_rounds".to_string(),
            value_source(
                args.max_rounds.is_some(),
                config.create.max_rounds.is_some(),
                "",
            ),
        ),
        ("capture_dir".to_string(), capture_dir_source),
    ]);
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_options_precedence_flag_over_config_over_default() {
        let mut args = base_args();
        args.max_rounds = Some(7);
        let mut config = AdeptConfig::default();
        config.create.max_rounds = Some(3);
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.max_rounds, 7);

        let args = base_args();
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.max_rounds, 3);

        let config = AdeptConfig::default();
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.max_rounds, adept_agent::create::DEFAULT_MAX_ROUNDS);
    }

    #[test]
    fn build_options_eval_cases_defaults_and_config_override() {
        let args = base_args();
        let config = AdeptConfig::default();
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.eval_cases, adept_agent::create::DEFAULT_EVAL_CASES);

        let mut config = AdeptConfig::default();
        config.create.eval_cases = Some(3);
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.eval_cases, 3);
    }

    fn base_args() -> CreateArgs {
        CreateArgs {
            from_file: None,
            out: None,
            name: None,
            write: false,
            overwrite: false,
            model: None,
            base_url: None,
            tokenizer: None,
            max_rounds: None,
            format: OutputFormat::Human,
            capture_dir: None,
        }
    }

    use crate::test_fixtures::{
        clean_body, clean_description, valid_eval_json, valid_generate_json,
    };

    #[test]
    fn resolve_brief_reads_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("brief.md");
        std::fs::write(&path, "Do the thing.\n").unwrap();
        let mut args = base_args();
        args.from_file = Some(path);
        assert_eq!(resolve_brief(&args).unwrap(), "Do the thing.\n");
    }

    #[test]
    fn resolve_brief_from_file_missing_is_usage_error() {
        let mut args = base_args();
        args.from_file = Some(PathBuf::from("/nonexistent/brief.md"));
        assert_eq!(resolve_brief(&args), Err(EXIT_USAGE_ERROR));
    }

    #[test]
    fn execute_preview_writes_nothing_then_write_flag_writes_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let good = valid_generate_json("demo-skill", clean_description(), clean_body());
        let eval = valid_eval_json(10);
        let mock = adept_agent::MockLlmClient::with_texts(vec![good, eval]);
        let options = CreateOptions::for_model("test-model", adept::Tokenizer::O200kBase);
        let args = base_args();

        let code = execute(
            &args,
            false,
            &mock,
            "Extract PDF form data",
            &out_dir,
            &options,
        );
        assert_eq!(code, EXIT_OK);
        assert!(
            !out_dir.join("SKILL.md").exists(),
            "preview must write nothing"
        );

        let good = valid_generate_json("demo-skill", clean_description(), clean_body());
        let eval = valid_eval_json(10);
        let mock = adept_agent::MockLlmClient::with_texts(vec![good, eval]);
        let mut write_args = base_args();
        write_args.write = true;
        let code = execute(
            &write_args,
            false,
            &mock,
            "Extract PDF form data",
            &out_dir,
            &options,
        );
        assert_eq!(code, EXIT_OK);
        assert!(out_dir.join("SKILL.md").is_file());
        assert!(out_dir.join("evals").join("evals.jsonl").is_file());
    }

    #[test]
    fn never_passing_mock_still_writes_best_effort_and_exits_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let bad = valid_generate_json("demo-skill", "short", clean_body());
        let eval = valid_eval_json(10);
        let mock = adept_agent::MockLlmClient::with_texts(vec![bad.clone(), bad, eval]);
        let options = CreateOptions::for_model("test-model", adept::Tokenizer::O200kBase);
        let mut args = base_args();
        args.write = true;

        let code = execute(
            &args,
            false,
            &mock,
            "Extract PDF form data",
            &out_dir,
            &options,
        );
        assert_eq!(code, EXIT_FINDINGS);
        assert!(
            out_dir.join("SKILL.md").is_file(),
            "best-effort candidate must still be written under --write"
        );
    }

    #[tokio::test]
    async fn json_format_emits_parseable_json_with_expected_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let good = valid_generate_json("demo-skill", clean_description(), clean_body());
        let eval = valid_eval_json(10);
        let mock = adept_agent::MockLlmClient::with_texts(vec![good, eval]);
        let options = CreateOptions::for_model("test-model", adept::Tokenizer::O200kBase);
        let report = adept_agent::create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        let json = render_json(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["skill_name"], "demo-skill");
        assert_eq!(parsed["candidate_diagnostics"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["eval_cases"].as_array().unwrap().len(), 10);
        assert!(parsed["files"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.ends_with("SKILL.md")));
    }

    fn base_report() -> adept_agent::CreateReport {
        adept_agent::CreateReport {
            skill_name: "demo-skill".to_string(),
            rounds_used: 1,
            siblings_found: false,
            candidate_diagnostics: Vec::new(),
            new_sibling_diagnostics: Vec::new(),
            eval_cases: Vec::new(),
            duplicate_eval_cases_dropped: 0,
            files: std::collections::BTreeMap::new(),
            outcome: adept_agent::CreateOutcome::Clean,
        }
    }

    #[test]
    fn render_human_reports_duplicate_eval_cases_dropped() {
        let mut report = base_report();
        report.eval_cases = vec![adept::evals::EvalCase {
            schema_version: 2,
            id: "case-1".to_string(),
            prompt: "do the thing".to_string(),
            assertions: vec![],
        }];
        report.duplicate_eval_cases_dropped = 3;

        let out = render_human(&report);
        assert!(
            out.contains("3 duplicate eval case(s) dropped\n"),
            "expected drop count line, got:\n{out}"
        );
    }

    #[test]
    fn render_human_omits_drop_line_when_nothing_dropped() {
        let report = base_report();
        let out = render_human(&report);
        assert!(
            !out.contains("duplicate eval case(s) dropped"),
            "clean run must not print a drop line, got:\n{out}"
        );
    }

    #[test]
    fn eval_dataset_failing_validation_fails_before_any_write() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let good = valid_generate_json("demo-skill", clean_description(), clean_body());
        let empty_eval = serde_json::json!({ "cases": [] }).to_string();
        let mock = adept_agent::MockLlmClient::with_texts(vec![good, empty_eval]);
        let options = CreateOptions::for_model("test-model", adept::Tokenizer::O200kBase);
        let mut args = base_args();
        args.write = true;

        let code = execute(
            &args,
            false,
            &mock,
            "Extract PDF form data",
            &out_dir,
            &options,
        );
        assert_eq!(code, EXIT_USAGE_ERROR);
        assert!(
            !out_dir.exists(),
            "nothing should be written when the dataset fails validation"
        );
    }

    /// A multi-file candidate (SKILL.md + a companion + the eval dataset)
    /// writes all three files or none of them. Forces a failure that occurs
    /// *before* `write_all_transactionally`'s rename window — permission
    /// denied while staging the temp file for `evals/evals.jsonl` — and
    /// asserts the target directory is left exactly as it started: no
    /// `SKILL.md`, no companion, no eval dataset. This does not (and cannot)
    /// exercise the writer's documented rename-window limitation
    /// (`writer.rs`'s `write_all_transactionally` doc comment: a crash
    /// between the first and last `rename` can still leave a batch partially
    /// applied) — only a failure occurring during staging, which is what a
    /// permission error actually produces here.
    #[cfg(unix)]
    #[test]
    fn multi_file_candidate_writes_all_or_nothing_on_forced_mid_batch_failure() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");
        std::fs::create_dir_all(&out_dir).unwrap();
        // Pre-create the `evals/` directory and strip write permission, so
        // `write_all_transactionally` fails staging `evals/evals.jsonl`'s
        // temp file (permission denied), *after* `SKILL.md` and the
        // companion file (both under the writable `out_dir` itself, and
        // sorted before `evals/...` in the `BTreeMap`) have already staged
        // successfully. The writer's own cleanup-on-error path must then
        // remove those already-staged temp files before returning, and no
        // rename has happened for any of the three files.
        let evals_dir = out_dir.join("evals");
        std::fs::create_dir_all(&evals_dir).unwrap();
        std::fs::set_permissions(&evals_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let candidate = serde_json::json!({
            "name": "demo-skill",
            "description": clean_description(),
            "disable_model_invocation": false,
            "body": clean_body(),
            "companion_files": [
                {"path": "REFERENCE.md", "content": "Reference material.\n"}
            ],
        })
        .to_string();
        let eval = valid_eval_json(10);
        let mock = adept_agent::MockLlmClient::with_texts(vec![candidate, eval]);
        let options = CreateOptions::for_model("test-model", adept::Tokenizer::O200kBase);
        let mut args = base_args();
        args.write = true;

        let code = execute(
            &args,
            false,
            &mock,
            "Extract PDF form data",
            &out_dir,
            &options,
        );

        // Restore permissions before any assertion so tempdir cleanup on
        // drop cannot itself fail.
        std::fs::set_permissions(&evals_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(code, EXIT_USAGE_ERROR);
        assert!(
            !out_dir.join("SKILL.md").exists(),
            "SKILL.md must not be written when the batch fails"
        );
        assert!(
            !out_dir.join("REFERENCE.md").exists(),
            "the companion file must not be written when the batch fails"
        );
        assert!(
            !evals_dir.join("evals.jsonl").exists(),
            "the eval dataset must not be written when the batch fails"
        );
        // No leftover `.adept-tmp` staging files anywhere in the tree.
        for entry in walkdir_files(&out_dir) {
            assert!(
                !entry.to_string_lossy().contains("adept-tmp"),
                "leftover temp file: {}",
                entry.display()
            );
        }
    }

    /// Minimal recursive file listing, used only by the test above (the
    /// `walkdir` crate isn't a dependency here, and this doesn't need to be
    /// more than a leaf-file collector).
    #[cfg(unix)]
    fn walkdir_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir_files(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn out_dir_with_existing_skill_md_is_refused_without_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("existing-skill");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(
            out_dir.join("SKILL.md"),
            "---\nname: x\ndescription: y\n---\nz\n",
        )
        .unwrap();
        let before = std::fs::read_to_string(out_dir.join("SKILL.md")).unwrap();

        let brief_path = tmp.path().join("brief.md");
        std::fs::write(&brief_path, "Do the thing.\n").unwrap();

        let mut args = base_args();
        args.out = Some(out_dir.clone());
        args.from_file = Some(brief_path);

        let config = AdeptConfig::default();
        let code = run(&args, &config, false);

        assert_eq!(code, EXIT_USAGE_ERROR);
        assert_eq!(
            std::fs::read_to_string(out_dir.join("SKILL.md")).unwrap(),
            before,
            "the existing SKILL.md must be untouched"
        );
        // No stray companion/eval directory created by the refused run.
        assert!(!out_dir.join("evals").exists());
    }
}
