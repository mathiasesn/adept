//! `adept eval`: triggering accuracy, token-bloat, cross-skill overlap, and
//! eval-dataset grading, unified behind one command and one report.
//!
//! The triggering/token-bloat/overlap analyses are network-backed (via an
//! LLM) and only run when a model is configured. The `evals` analysis
//! (grading a `results.jsonl` sidecar against `evals/evals.jsonl`) is
//! offline and deterministic, and runs when `--results` is supplied.
//! Transport (`adept_agent::OpenAiCompatClient`) is constructed lazily —
//! only when at least one LLM-backed analysis is actually selected — so
//! `adept eval --results r.jsonl --select evals` never touches the network
//! and needs no `ADEPT_MODEL`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adept::evals::EvalBenchmarkReport;
use adept::{Skill, SkillSet};
use adept_agent::{
    CaptureSink, EvalOptions, EvalReport, LlmConfig, ResolvedLlmConfig, RunMetadata,
};

use crate::cli::{EvalArgs, OutputFormat};
use crate::config::{
    attach_capture, build_runtime, resolve_llm_client, shared_sources, value_source, AdeptConfig,
};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FINDINGS: i32 = 1;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// The four analyses `adept eval` can run, by their `--select`/`--ignore`
/// name. Shared with the MCP `eval_skill` tool's `select`/`ignore` arguments.
pub(crate) const ANALYSES: [&str; 4] = ["triggering", "token-bloat", "overlap", "evals"];

/// Run `adept eval`, building its own `tokio` runtime and a real
/// [`adept_agent::OpenAiCompatClient`] — but only if an LLM-backed analysis
/// was actually selected. Returns the process exit code.
pub fn run(args: &EvalArgs, config: &AdeptConfig) -> i32 {
    let base_url = args
        .base_url
        .clone()
        .or_else(|| config.eval.base_url.clone());
    let model = args.model.clone().or_else(|| config.eval.model.clone());
    let model_available = probe_model_available(base_url.clone(), model.clone());
    let results_available = args.results.is_some();

    let selection = match resolve_analyses(
        &args.select,
        &args.ignore,
        model_available,
        results_available,
    ) {
        Ok(selection) => selection,
        Err(message) => {
            eprintln!("adept: error: {message}");
            return EXIT_USAGE_ERROR;
        }
    };

    let (skill, skillset) = match load_skill_and_set(&args.path) {
        Ok(pair) => pair,
        Err(message) => {
            eprintln!("adept: error: {message}");
            return EXIT_USAGE_ERROR;
        }
    };

    let tokenizer = args
        .tokenizer
        .map(adept::Tokenizer::from)
        .or(config.eval.tokenizer)
        .unwrap_or_default();

    let needs_llm = selection.contains("triggering")
        || selection.contains("token-bloat")
        || selection.contains("overlap");

    let mut report = EvalReport::new(skill.frontmatter.name.clone());
    let mut sink: Option<Arc<CaptureSink>> = None;

    if needs_llm {
        let Some((client, resolved)) = resolve_llm_client("eval", base_url, model) else {
            return EXIT_USAGE_ERROR;
        };

        let mut options = build_options(args, &resolved.model, tokenizer);
        if !selection.contains("triggering") {
            options.triggering = None;
        }
        options.token_bloat = selection.contains("token-bloat");

        let (client, capture_sink) = match attach_capture(
            client,
            args.capture_dir.as_deref(),
            config.eval.capture_dir.as_deref(),
            config.origin_dir.as_deref(),
            |source| capture_metadata(args, config, &resolved, tokenizer, &options, source),
        ) {
            Ok(pair) => pair,
            Err(exit_code) => return exit_code,
        };
        sink = capture_sink;

        let Some(runtime) = build_runtime() else {
            finalize(&sink, EXIT_USAGE_ERROR);
            return EXIT_USAGE_ERROR;
        };

        let empty_skillset: Vec<Skill> = Vec::new();
        let llm_skillset: &[Skill] = if selection.contains("overlap") {
            &skillset
        } else {
            &empty_skillset
        };

        match runtime.block_on(adept_agent::eval_skill(
            &client,
            &skill,
            llm_skillset,
            &options,
        )) {
            Ok(llm_report) => {
                report.prompt_version = llm_report.prompt_version;
                report.triggering = llm_report.triggering;
                report.token_bloat = llm_report.token_bloat;
                report.overlaps = llm_report.overlaps;
            }
            Err(err) => {
                eprintln!("adept: error: eval failed: {err}");
                finalize(&sink, EXIT_USAGE_ERROR);
                return EXIT_USAGE_ERROR;
            }
        }
    }

    let mut findings = false;
    if selection.contains("evals") {
        match grade_from_args(args, &args.path) {
            Ok(benchmark) => {
                if benchmark.cases.iter().any(|case| !case.pass) {
                    findings = true;
                }
                report.evals = Some(benchmark);
            }
            Err(message) => {
                eprintln!("adept: error: {message}");
                finalize(&sink, EXIT_USAGE_ERROR);
                return EXIT_USAGE_ERROR;
            }
        }
    }

    let exit_code = if findings { EXIT_FINDINGS } else { EXIT_OK };

    match args.format {
        OutputFormat::Human => print!("{}", report.render()),
        OutputFormat::Json => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("adept: error: failed to render JSON: {err}");
                finalize(&sink, EXIT_USAGE_ERROR);
                return EXIT_USAGE_ERROR;
            }
        },
    }

    finalize(&sink, exit_code);
    exit_code
}

fn finalize(sink: &Option<Arc<CaptureSink>>, exit_code: i32) {
    if let Some(sink) = sink {
        sink.finalize(exit_code);
    }
}

/// Probe whether an LLM model can be resolved from `base_url`/`model`
/// (CLI-flag/config-file values already merged in by the caller) plus the
/// `ADEPT_*` environment, without printing anything. Used only to decide
/// the *default* analysis selection — never gates a usage error by itself.
fn probe_model_available(base_url: Option<String>, model: Option<String>) -> bool {
    LlmConfig {
        base_url,
        api_key: None,
        model,
    }
    .resolve()
    .is_ok()
}

/// Map an analysis name (rule-code-vocabulary style: exact, case-sensitive)
/// to its canonical `&'static str`, or `None` if unrecognized.
fn canonical_analysis(name: &str) -> Option<&'static str> {
    ANALYSES
        .iter()
        .find(|&&candidate| candidate == name)
        .copied()
}

/// Resolve the set of analyses `adept eval` should run, given `--select`/
/// `--ignore` and whether a model and `--results` are available.
///
/// - Unknown names in either flag are a clear error.
/// - With `--select` empty, the default selection is everything whose
///   precondition is met (`evals` iff `results_available`; the three LLM
///   analyses iff `model_available`), then narrowed by `--ignore`.
/// - With `--select` non-empty, an explicitly selected analysis whose
///   precondition is missing is a usage error naming what's missing —
///   never a silent skip.
/// - An empty result (nothing selected, or nothing available at all) is an
///   error rather than an empty report.
pub(crate) fn resolve_analyses(
    select: &[String],
    ignore: &[String],
    model_available: bool,
    results_available: bool,
) -> Result<std::collections::BTreeSet<&'static str>, String> {
    for name in select.iter().chain(ignore.iter()) {
        if canonical_analysis(name).is_none() {
            return Err(format!(
                "unknown analysis '{name}': expected one of triggering, token-bloat, overlap, evals"
            ));
        }
    }

    let precondition_met = |name: &str| -> bool {
        if name == "evals" {
            results_available
        } else {
            model_available
        }
    };
    let missing_reason = |name: &str| -> &'static str {
        if name == "evals" {
            "no --results supplied"
        } else {
            "no model configured (set --model, ADEPT_MODEL, or `[eval] model` in adept.toml)"
        }
    };

    let mut selection: std::collections::BTreeSet<&'static str> = if select.is_empty() {
        ANALYSES
            .iter()
            .copied()
            .filter(|name| precondition_met(name))
            .collect()
    } else {
        let mut set = std::collections::BTreeSet::new();
        for name in select {
            let name = canonical_analysis(name).expect("validated above");
            if !precondition_met(name) {
                return Err(format!("--select {name} requires {}", missing_reason(name)));
            }
            set.insert(name);
        }
        set
    };

    for name in ignore {
        let name = canonical_analysis(name).expect("validated above");
        selection.remove(name);
    }

    if selection.is_empty() {
        return Err(
            "nothing to evaluate: no model configured (--model/ADEPT_MODEL/[eval] model) and no --results supplied"
                .to_string(),
        );
    }

    Ok(selection)
}

/// Describe the run for `run_metadata.json`: the resolved options plus,
/// under `sources`, which layer supplied each of them. The API key is only
/// ever reported as a boolean — its value is never read here.
fn capture_metadata(
    args: &EvalArgs,
    config: &AdeptConfig,
    resolved: &ResolvedLlmConfig,
    tokenizer: adept::Tokenizer,
    options: &EvalOptions,
    capture_dir_source: &'static str,
) -> RunMetadata {
    let mut metadata = RunMetadata::new("eval");
    metadata.model = Some(resolved.model.clone());
    metadata.base_url = Some(resolved.base_url.clone());
    metadata.tokenizer = Some(tokenizer.to_string());
    metadata.api_key_present = resolved.api_key.is_some();
    metadata.target_path = Some(args.path.display().to_string());
    if let Some(triggering) = options.triggering.as_ref() {
        metadata.seed = triggering.seed;
        metadata.num_prompts = Some(triggering.num_prompts);
        metadata.judge_samples = Some(triggering.judge_samples);
    }

    metadata.sources = shared_sources(
        args.model.is_some(),
        config.eval.model.is_some(),
        args.base_url.is_some(),
        config.eval.base_url.is_some(),
        args.tokenizer.is_some(),
        config.eval.tokenizer.is_some(),
    );
    metadata.sources.extend([
        (
            "num_prompts".to_string(),
            value_source(args.num_prompts.is_some(), false, ""),
        ),
        (
            "seed".to_string(),
            value_source(args.seed.is_some(), false, ""),
        ),
        (
            "judge_samples".to_string(),
            value_source(args.judge_samples.is_some(), false, ""),
        ),
        ("capture_dir".to_string(), capture_dir_source),
    ]);
    metadata
}

fn build_options(args: &EvalArgs, model: &str, tokenizer: adept::Tokenizer) -> EvalOptions {
    let mut options = EvalOptions::for_model(model, tokenizer);
    if let Some(triggering) = options.triggering.as_mut() {
        if let Some(n) = args.num_prompts {
            triggering.num_prompts = n;
        }
        if let Some(seed) = args.seed {
            triggering.seed = Some(seed);
        }
        if let Some(samples) = args.judge_samples {
            triggering.judge_samples = samples;
        }
    }
    options
}

fn load_skill_and_set(path: &Path) -> Result<(Skill, Vec<Skill>), String> {
    if !path.exists() {
        return Err(format!("path not found: {}", path.display()));
    }
    let skill_file = resolve_skill_file(path)?;
    let skill = adept::parse_skill(&skill_file).map_err(|err| err.to_string())?;

    // Discover sibling skills for overlap detection: walk the parent of the
    // skill's own directory, where siblings live.
    let search_root = adept::sibling_root(path);
    let skillset = SkillSet::discover(&search_root)
        .map(|set| set.skills)
        .unwrap_or_default();

    Ok((skill, skillset))
}

/// Resolve `path` (as accepted by `adept eval`) to the `SKILL.md` file to
/// parse: `path` itself if it already names a file, or `path/SKILL.md` if
/// `path` is a directory. A directory with no `SKILL.md` in it is a clear
/// usage error rather than the raw I/O error `adept::parse_skill` would
/// otherwise surface (`Is a directory (os error 21)`).
fn resolve_skill_file(path: &Path) -> Result<PathBuf, String> {
    if path.is_dir() {
        let candidate = path.join("SKILL.md");
        if !candidate.is_file() {
            return Err(format!("no SKILL.md found in directory {}", path.display()));
        }
        Ok(candidate)
    } else {
        Ok(path.to_path_buf())
    }
}

/// The skill's own directory, given a path to its `SKILL.md` file or the
/// skill's own directory: `path` itself if it's a directory, else its
/// parent (falling back to `.`). Mirrors `adept::skillset`'s private
/// `skill_directory` helper (not exported), which `evals/evals.jsonl`
/// discovery is relative to.
fn skill_directory(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Read `--results`, discover/read the eval dataset (`--evals` or
/// `evals/evals.jsonl` relative to the skill directory), and grade. Purely
/// offline: no LLM client is touched anywhere in this path.
fn grade_from_args(args: &EvalArgs, skill_path: &Path) -> Result<EvalBenchmarkReport, String> {
    let results_path = args
        .results
        .as_ref()
        .expect("the `evals` analysis is only selected when --results is present");
    let results_text = std::fs::read_to_string(results_path).map_err(|err| {
        format!(
            "failed to read results file {}: {err}",
            results_path.display()
        )
    })?;
    let results = adept::evals::parse_results_jsonl(&results_text).map_err(|err| {
        format!(
            "failed to parse results file {}: {err}",
            results_path.display()
        )
    })?;

    let dataset_path = args.evals.clone().unwrap_or_else(|| {
        skill_directory(skill_path)
            .join("evals")
            .join("evals.jsonl")
    });
    let dataset_text = std::fs::read_to_string(&dataset_path).map_err(|err| {
        format!(
            "failed to read eval dataset {}: {err}",
            dataset_path.display()
        )
    })?;
    adept::evals::validate(&dataset_text)
        .map_err(|err| format!("invalid eval dataset {}: {err}", dataset_path.display()))?;
    let cases = adept::evals::parse_jsonl(&dataset_text).map_err(|err| {
        format!(
            "failed to parse eval dataset {}: {err}",
            dataset_path.display()
        )
    })?;

    Ok(adept::evals::grade(&cases, &results))
}

/// A thin wrapper so tests can drive `eval_skill` with an injected
/// [`adept_agent::LlmClient`] (e.g. [`adept_agent::MockLlmClient`]) instead
/// of a real network client, exercising the same options-building and
/// report-rendering logic used by [`run`].
#[cfg(test)]
pub async fn run_with_client(
    client: &dyn adept_agent::LlmClient,
    skill: &Skill,
    skillset: &[Skill],
    options: &EvalOptions,
) -> Result<String, adept_agent::EvalError> {
    let report = adept_agent::eval_skill(client, skill, skillset, options).await?;
    Ok(report.render())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adept::{AnthropicSkillParser, SkillParser};
    use adept_agent::MockLlmClient;

    fn sample_skill() -> Skill {
        let path = std::path::Path::new("SKILL.md");
        AnthropicSkillParser
            .parse_str(
                path,
                "---\nname: pdf-filler\ndescription: Fills PDF forms with user-supplied data\n---\nBody text.\n",
            )
            .unwrap()
    }

    fn sample_args() -> EvalArgs {
        EvalArgs {
            path: std::path::PathBuf::from("SKILL.md"),
            format: OutputFormat::Human,
            model: None,
            base_url: None,
            num_prompts: Some(4),
            seed: Some(42),
            judge_samples: Some(3),
            tokenizer: None,
            capture_dir: None,
            results: None,
            evals: None,
            select: Vec::new(),
            ignore: Vec::new(),
        }
    }

    #[test]
    fn build_options_applies_flag_overrides() {
        let args = sample_args();
        let options = build_options(&args, "test-model", adept::Tokenizer::default());
        assert_eq!(options.model, "test-model");
        let triggering = options.triggering.unwrap();
        assert_eq!(triggering.num_prompts, 4);
        assert_eq!(triggering.seed, Some(42));
        assert_eq!(triggering.judge_samples, 3);
    }

    #[tokio::test]
    async fn run_with_client_renders_report_via_mock_llm() {
        let mock = MockLlmClient::with_texts(vec![
            r#"{"prompts": [{"text": "Fill out this W-9", "label": "positive"}, {"text": "What's the weather?", "label": "negative"}]}"#,
            r#"{"would_trigger": true, "reasoning": "matches"}"#,
            r#"{"would_trigger": false, "reasoning": "unrelated"}"#,
            r#"{"suggestions": []}"#,
        ]);

        let skill = sample_skill();
        let mut options = EvalOptions::for_model("test-model", adept::Tokenizer::default());
        options.triggering.as_mut().unwrap().num_prompts = 2;

        let rendered = run_with_client(&mock, &skill, &[], &options).await.unwrap();
        assert!(rendered.contains("Eval report for skill: pdf-filler"));
        assert!(rendered.contains("Triggering accuracy"));
    }

    #[test]
    fn unknown_analysis_name_is_rejected() {
        let err = resolve_analyses(&["bogus".to_string()], &[], true, true).unwrap_err();
        assert!(err.contains("unknown analysis"), "{err}");
        assert!(err.contains("bogus"), "{err}");
    }

    #[test]
    fn default_selection_is_derived_from_availability() {
        // Nothing available: an explicit error, not an empty report.
        let err = resolve_analyses(&[], &[], false, false).unwrap_err();
        assert!(err.contains("nothing to evaluate"), "{err}");

        // Only results available: only `evals` runs by default.
        let selection = resolve_analyses(&[], &[], false, true).unwrap();
        assert_eq!(selection, std::collections::BTreeSet::from(["evals"]));

        // Only a model available: the three LLM analyses run by default.
        let selection = resolve_analyses(&[], &[], true, false).unwrap();
        assert_eq!(
            selection,
            std::collections::BTreeSet::from(["triggering", "token-bloat", "overlap"])
        );

        // Both available: everything runs by default.
        let selection = resolve_analyses(&[], &[], true, true).unwrap();
        assert_eq!(selection, std::collections::BTreeSet::from(ANALYSES));
    }

    #[test]
    fn explicit_select_with_missing_precondition_is_a_usage_error_naming_it() {
        let err = resolve_analyses(&["triggering".to_string()], &[], false, true).unwrap_err();
        assert!(err.contains("triggering"), "{err}");
        assert!(err.contains("model"), "{err}");

        let err = resolve_analyses(&["evals".to_string()], &[], true, false).unwrap_err();
        assert!(err.contains("evals"), "{err}");
        assert!(err.contains("--results"), "{err}");
    }

    #[test]
    fn ignore_narrows_the_default_selection() {
        let selection = resolve_analyses(&[], &["overlap".to_string()], true, true).unwrap();
        assert!(!selection.contains("overlap"));
        assert!(selection.contains("triggering"));
        assert!(selection.contains("evals"));
    }

    /// Pins the network invariant: an offline `--select evals` run must
    /// never even attempt to resolve or construct an LLM client. This is
    /// checked at the `resolve_analyses` boundary `run` calls before ever
    /// touching `resolve_llm_client`/`OpenAiCompatClient` — `needs_llm` is
    /// false whenever the selection is exactly `{"evals"}`, which is what
    /// keeps `run` from ever reaching the transport-construction branch.
    #[test]
    fn evals_only_selection_needs_no_llm() {
        let selection = resolve_analyses(&["evals".to_string()], &[], false, true).unwrap();
        assert_eq!(selection, std::collections::BTreeSet::from(["evals"]));
        let needs_llm = selection.contains("triggering")
            || selection.contains("token-bloat")
            || selection.contains("overlap");
        assert!(!needs_llm, "an evals-only selection must not need an LLM");
    }

    #[test]
    fn grade_from_args_runs_fully_offline_with_no_model_configured() {
        std::env::remove_var("ADEPT_MODEL");
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("demo-skill");
        std::fs::create_dir_all(skill_dir.join("evals")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: does a demo thing. Use when demoing.\n---\nBody.\n",
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("evals").join("evals.jsonl"),
            r#"{"schema_version":1,"prompt":"demo","assertions":[{"kind":"contains","value":"ok"}]}"#.to_string() + "\n",
        )
        .unwrap();
        let results_path = dir.path().join("results.jsonl");
        std::fs::write(
            &results_path,
            r#"{"case":1,"response":"it is ok"}"#.to_string() + "\n",
        )
        .unwrap();

        let mut args = sample_args();
        args.path = skill_dir.join("SKILL.md");
        args.results = Some(results_path);

        let report = grade_from_args(&args, &args.path).unwrap();
        assert_eq!(report.cases.len(), 1);
        assert!(report.cases[0].pass);
    }
}
