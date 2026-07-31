//! `adept score`.

use adept::{Skill, SkillSet};
use adept_agent::{EvalOptions, OpenAiCompatClient, ResolvedLlmConfig, RunMetadata};

use crate::cli::{OutputFormat, ScoreArgs};
use crate::config::{
    attach_capture, build_runtime, resolve_llm_client, shared_sources, value_source, AdeptConfig,
};

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Run `adept score`, building its own `tokio` runtime and a real
/// [`adept_agent::OpenAiCompatClient`]. Returns the process exit code.
pub fn run(args: &ScoreArgs, config: &AdeptConfig) -> i32 {
    let base_url = args
        .base_url
        .clone()
        .or_else(|| config.score.base_url.clone());
    let model = args.model.clone().or_else(|| config.score.model.clone());
    let Some((client, resolved)) = resolve_llm_client("score", base_url, model) else {
        return EXIT_USAGE_ERROR;
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
        .or(config.score.tokenizer)
        .unwrap_or_default();
    let options = build_options(args, &resolved.model, tokenizer);

    let (client, sink) = match attach_capture(
        client,
        args.capture_dir.as_deref(),
        config.score.capture_dir.as_deref(),
        config.origin_dir.as_deref(),
        |source| capture_metadata(args, config, &resolved, tokenizer, &options, source),
    ) {
        Ok(pair) => pair,
        Err(exit_code) => return exit_code,
    };

    let exit_code = execute(args, &client, &skill, &skillset, &options);
    if let Some(sink) = &sink {
        sink.finalize(exit_code);
    }
    exit_code
}

/// The scoring call itself plus report rendering, split out of [`run`] so
/// the capture sink can be finalised with the actual exit code.
fn execute(
    args: &ScoreArgs,
    client: &OpenAiCompatClient,
    skill: &Skill,
    skillset: &[Skill],
    options: &EvalOptions,
) -> i32 {
    let Some(runtime) = build_runtime() else {
        return EXIT_USAGE_ERROR;
    };

    let report = runtime.block_on(adept_agent::eval_skill(client, skill, skillset, options));

    match report {
        Ok(report) => {
            match args.format {
                OutputFormat::Human => print!("{}", report.render()),
                OutputFormat::Json => match serde_json::to_string_pretty(&report) {
                    Ok(json) => println!("{json}"),
                    Err(err) => {
                        eprintln!("adept: error: failed to render JSON: {err}");
                        return EXIT_USAGE_ERROR;
                    }
                },
            }
            EXIT_OK
        }
        Err(err) => {
            eprintln!("adept: error: scoring failed: {err}");
            EXIT_USAGE_ERROR
        }
    }
}

/// Describe the run for `run_metadata.json`: the resolved options plus,
/// under `sources`, which layer supplied each of them. The API key is only
/// ever reported as a boolean — its value is never read here.
fn capture_metadata(
    args: &ScoreArgs,
    config: &AdeptConfig,
    resolved: &ResolvedLlmConfig,
    tokenizer: adept::Tokenizer,
    options: &EvalOptions,
    capture_dir_source: &'static str,
) -> RunMetadata {
    let mut metadata = RunMetadata::new("score");
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
        config.score.model.is_some(),
        args.base_url.is_some(),
        config.score.base_url.is_some(),
        args.tokenizer.is_some(),
        config.score.tokenizer.is_some(),
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

fn build_options(args: &ScoreArgs, model: &str, tokenizer: adept::Tokenizer) -> EvalOptions {
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

fn load_skill_and_set(path: &std::path::Path) -> Result<(Skill, Vec<Skill>), String> {
    if !path.exists() {
        return Err(format!("path not found: {}", path.display()));
    }
    let skill = adept::parse_skill(path).map_err(|err| err.to_string())?;

    // Discover sibling skills for overlap detection: walk the parent of the
    // skill's own directory, where siblings live.
    let search_root = adept::sibling_root(path);
    let skillset = SkillSet::discover(&search_root)
        .map(|set| set.skills)
        .unwrap_or_default();

    Ok((skill, skillset))
}

/// A thin wrapper so tests can drive `eval_skill` with an injected
/// [`LlmClient`] (e.g. [`adept_agent::MockLlmClient`]) instead of a real
/// network client, exercising the same options-building and
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

    #[test]
    fn build_options_applies_flag_overrides() {
        let args = ScoreArgs {
            path: std::path::PathBuf::from("SKILL.md"),
            format: OutputFormat::Human,
            model: None,
            base_url: None,
            num_prompts: Some(4),
            seed: Some(42),
            judge_samples: Some(3),
            tokenizer: None,
            capture_dir: None,
        };
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
        assert!(rendered.contains("Score report for skill: pdf-filler"));
        assert!(rendered.contains("Triggering accuracy"));
    }
}
