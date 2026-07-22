//! `adept score`.

use adept::{Skill, SkillSet};
use adept_score::{
    LlmClient, LlmConfig, OpenAiCompatClient, ScoreOptions, TriggeringOptions, ENV_API_KEY,
    ENV_BASE_URL, ENV_MODEL,
};

use crate::cli::{OutputFormat, ScoreArgs};
use crate::config::AdeptConfig;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Run `adept score`, building its own `tokio` runtime and a real
/// [`OpenAiCompatClient`]. Returns the process exit code.
pub fn run(args: &ScoreArgs, config: &AdeptConfig) -> i32 {
    let llm_config = LlmConfig {
        base_url: args
            .base_url
            .clone()
            .or_else(|| config.score.base_url.clone()),
        api_key: None,
        model: args.model.clone().or_else(|| config.score.model.clone()),
    };

    let resolved = match llm_config.resolve() {
        Ok(resolved) => resolved,
        Err(_) => {
            eprintln!("adept: error: could not resolve an LLM model to score with.");
            eprintln!(
                "  set one of: --model <MODEL>, config file `[score] model = \"...\"`, or the {ENV_MODEL} environment variable"
            );
            eprintln!(
                "  optionally also set {ENV_BASE_URL} (defaults to the OpenAI API) and {ENV_API_KEY}"
            );
            return EXIT_USAGE_ERROR;
        }
    };
    let client = OpenAiCompatClient::new(resolved.clone());

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

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("adept: error: failed to start async runtime: {err}");
            return EXIT_USAGE_ERROR;
        }
    };

    let report = runtime.block_on(adept_score::score_skill(
        &client, &skill, &skillset, &options,
    ));

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

fn build_options(args: &ScoreArgs, model: &str, tokenizer: adept::Tokenizer) -> ScoreOptions {
    let mut triggering = TriggeringOptions {
        model: model.to_string(),
        ..TriggeringOptions::default()
    };
    if let Some(n) = args.num_prompts {
        triggering.num_prompts = n;
    }
    if let Some(seed) = args.seed {
        triggering.seed = Some(seed);
    }
    if let Some(samples) = args.judge_samples {
        triggering.judge_samples = samples;
    }

    ScoreOptions {
        model: model.to_string(),
        triggering: Some(triggering),
        token_bloat: true,
        overlap_similarity_threshold: adept_score::DEFAULT_SIMILARITY_THRESHOLD,
        tokenizer,
    }
}

fn load_skill_and_set(path: &std::path::Path) -> Result<(Skill, Vec<Skill>), String> {
    if !path.exists() {
        return Err(format!("path not found: {}", path.display()));
    }
    let skill = adept::parse_skill(path).map_err(|err| err.to_string())?;

    // Discover sibling skills for overlap detection: walk the parent
    // directory of the skill (or the given directory itself).
    let search_root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    let skillset = SkillSet::discover(&search_root)
        .map(|set| set.skills)
        .unwrap_or_default();

    Ok((skill, skillset))
}

/// A thin wrapper so tests can drive `score_skill` with an injected
/// [`LlmClient`] (e.g. [`adept_score::MockLlmClient`]) instead of a real
/// network client, exercising the same options-building and
/// report-rendering logic used by [`run`].
#[allow(dead_code)]
pub async fn run_with_client(
    client: &dyn LlmClient,
    skill: &Skill,
    skillset: &[Skill],
    options: &ScoreOptions,
) -> Result<String, adept_score::ScoreError> {
    let report = adept_score::score_skill(client, skill, skillset, options).await?;
    Ok(report.render())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adept::{AnthropicSkillParser, SkillParser};
    use adept_score::MockLlmClient;

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
        let mut triggering = TriggeringOptions {
            num_prompts: 2,
            ..Default::default()
        };
        triggering.model = "test-model".to_string();
        let options = ScoreOptions {
            model: "test-model".to_string(),
            triggering: Some(triggering),
            token_bloat: true,
            overlap_similarity_threshold: adept_score::DEFAULT_SIMILARITY_THRESHOLD,
            tokenizer: adept::Tokenizer::default(),
        };

        let rendered = run_with_client(&mock, &skill, &[], &options).await.unwrap();
        assert!(rendered.contains("Score report for skill: pdf-filler"));
        assert!(rendered.contains("Triggering accuracy"));
    }
}
