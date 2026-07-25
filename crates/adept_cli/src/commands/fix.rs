//! `adept fix`.

use adept::{Skill, SkillSet};
use adept_fix::{fix_skill, write_all_transactionally, FixOptions, FixReport, DEFAULT_MAX_ROUNDS};
use adept_score::{LlmConfig, OpenAiCompatClient, ENV_API_KEY, ENV_BASE_URL, ENV_MODEL};

use crate::cli::{FixArgs, OutputFormat};
use crate::commands::check::apply_select_ignore;
use crate::config::AdeptConfig;

/// Exit code contract: 0 = clean/no pending changes, 1 = changes pending
/// (`--check`), 2 = usage/I/O error.
pub const EXIT_OK: i32 = 0;
pub const EXIT_CHANGES_PENDING: i32 = 1;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Run `adept fix`, building its own `tokio` runtime and a real
/// [`OpenAiCompatClient`]. Returns the process exit code.
pub fn run(args: &FixArgs, config: &AdeptConfig, quiet: bool) -> i32 {
    let llm_config = LlmConfig {
        base_url: args
            .base_url
            .clone()
            .or_else(|| config.fix.base_url.clone()),
        api_key: None,
        model: args.model.clone().or_else(|| config.fix.model.clone()),
    };

    let resolved = match llm_config.resolve() {
        Ok(resolved) => resolved,
        Err(_) => {
            eprintln!("adept: error: could not resolve an LLM model to fix with.");
            eprintln!(
                "  set one of: --model <MODEL>, config file `[fix] model = \"...\"`, or the {ENV_MODEL} environment variable"
            );
            eprintln!(
                "  optionally also set {ENV_BASE_URL} (defaults to the OpenAI API) and {ENV_API_KEY}"
            );
            return EXIT_USAGE_ERROR;
        }
    };
    let client = OpenAiCompatClient::new(resolved.clone());

    let tokenizer = args
        .tokenizer
        .map(adept::Tokenizer::from)
        .or(config.fix.tokenizer)
        .unwrap_or_default();

    let options = build_options(args, config, &resolved.model, tokenizer);

    let mut skills: Vec<Skill> = Vec::new();
    let mut had_error = false;
    for path in &args.paths {
        if !path.exists() {
            eprintln!("adept: error: path not found: {}", path.display());
            had_error = true;
            continue;
        }
        match SkillSet::discover(path) {
            Ok(set) => {
                for (err_path, err) in &set.errors {
                    eprintln!("adept: error: {}: {err}", err_path.display());
                    had_error = true;
                }
                skills.extend(set.skills);
            }
            Err(err) => {
                eprintln!("adept: error: {err}");
                had_error = true;
            }
        }
    }

    if had_error {
        return EXIT_USAGE_ERROR;
    }

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("adept: error: failed to start async runtime: {err}");
            return EXIT_USAGE_ERROR;
        }
    };

    let mut reports: Vec<FixReport> = Vec::new();
    for skill in &skills {
        let report = runtime.block_on(fix_skill(&client, skill, &options));
        match report {
            Ok(report) => reports.push(report),
            Err(err) => {
                eprintln!("adept: error: fixing {}: {err}", skill.frontmatter.name);
                return EXIT_USAGE_ERROR;
            }
        }
    }

    let mut any_pending = false;
    let mut written = 0usize;

    for report in &reports {
        if !report.files.is_empty() {
            any_pending = true;
        }

        match args.format {
            OutputFormat::Human => {
                if args.diff {
                    print!("{}", report.diff);
                } else if !args.check {
                    print!("{}", report.render());
                }
            }
            OutputFormat::Json => {
                if !args.check {
                    match serde_json::to_string_pretty(report) {
                        Ok(json) => println!("{json}"),
                        Err(err) => {
                            eprintln!("adept: error: failed to render JSON: {err}");
                            return EXIT_USAGE_ERROR;
                        }
                    }
                }
            }
        }

        if args.write && !report.files.is_empty() {
            if let Err(err) = write_all_transactionally(&report.files) {
                eprintln!(
                    "adept: error: failed to write fixes for {}: {err}",
                    report.skill_name
                );
                return EXIT_USAGE_ERROR;
            }
            written += 1;
        }
    }

    if args.write && !quiet {
        println!(
            "{written} skill{} fixed, {} unchanged",
            if written == 1 { "" } else { "s" },
            reports.len() - written,
        );
    }

    if args.check {
        return if any_pending {
            EXIT_CHANGES_PENDING
        } else {
            EXIT_OK
        };
    }

    EXIT_OK
}

fn build_options(
    args: &FixArgs,
    config: &AdeptConfig,
    model: &str,
    tokenizer: adept::Tokenizer,
) -> FixOptions {
    let mut options = FixOptions::for_model(model, tokenizer);
    options.max_rounds = args
        .max_rounds
        .or(config.fix.max_rounds)
        .unwrap_or(DEFAULT_MAX_ROUNDS);

    let mut lint_config = config.lint.clone();
    apply_select_ignore(&mut lint_config, &args.select, &args.ignore);
    lint_config.tokenizer = tokenizer;
    options.lint_config = lint_config;

    options.fmt_config = config.fmt.clone();
    options.select = args.select.clone();
    options.ignore = args.ignore.clone();

    options
}

/// A thin wrapper so tests can drive `fix_skill` with an injected
/// [`adept_score::LlmClient`] (e.g. [`adept_score::MockLlmClient`]) instead
/// of a real network client, exercising the same report-rendering logic
/// used by [`run`].
#[cfg(test)]
pub async fn run_with_client(
    client: &dyn adept_score::LlmClient,
    skill: &Skill,
    options: &FixOptions,
) -> Result<String, adept_fix::FixError> {
    let report = fix_skill(client, skill, options).await?;
    Ok(report.render())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adept::{AnthropicSkillParser, SkillParser};
    use adept_score::MockLlmClient;
    use std::path::PathBuf;

    fn sample_skill() -> Skill {
        let path = std::path::Path::new("SKILL.md");
        AnthropicSkillParser
            .parse_str(
                path,
                "---\nname: pdf-filler\ndescription: Fills PDF forms with user-supplied data. Use when the user asks to fill a form. Do not use for scanned images.\n---\nBody text.\n",
            )
            .unwrap()
    }

    fn base_args() -> FixArgs {
        FixArgs {
            paths: vec![PathBuf::from(".")],
            write: false,
            check: false,
            diff: false,
            select: Vec::new(),
            ignore: Vec::new(),
            model: None,
            base_url: None,
            max_rounds: None,
            tokenizer: None,
            format: OutputFormat::Human,
        }
    }

    #[test]
    fn build_options_precedence_flag_over_config_over_default() {
        // Flag wins.
        let mut args = base_args();
        args.max_rounds = Some(7);
        let mut config = AdeptConfig::default();
        config.fix.max_rounds = Some(3);
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.max_rounds, 7);

        // Config wins over default.
        let args = base_args();
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.max_rounds, 3);

        // Default when neither set.
        let config = AdeptConfig::default();
        let options = build_options(&args, &config, "test-model", adept::Tokenizer::O200kBase);
        assert_eq!(options.max_rounds, DEFAULT_MAX_ROUNDS);
    }

    #[test]
    fn build_options_uses_resolved_model_and_tokenizer() {
        let args = base_args();
        let config = AdeptConfig::default();
        let options = build_options(
            &args,
            &config,
            "resolved-model",
            adept::Tokenizer::Cl100kBase,
        );
        assert_eq!(options.model, "resolved-model");
        assert_eq!(options.tokenizer, adept::Tokenizer::Cl100kBase);
        assert_eq!(options.lint_config.tokenizer, adept::Tokenizer::Cl100kBase);
    }

    #[tokio::test]
    async fn run_with_client_renders_report_via_mock_llm() {
        let mock = MockLlmClient::with_texts(Vec::<String>::new());
        let skill = sample_skill();
        let options = FixOptions::for_model("test-model", adept::Tokenizer::default());

        let rendered = run_with_client(&mock, &skill, &options).await.unwrap();
        assert!(rendered.contains("adept fix: pdf-filler"));
        assert!(rendered.contains("no LLM-fixable diagnostics found"));
    }
}
