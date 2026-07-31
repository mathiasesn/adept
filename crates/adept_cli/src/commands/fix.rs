//! `adept fix`.

use adept::{Skill, SkillSet};
use adept_agent::{
    fix_skill, write_all_transactionally, FixOptions, FixReport, DEFAULT_MAX_ROUNDS,
};
use adept_score::{OpenAiCompatClient, ResolvedLlmConfig, RunMetadata};

use crate::cli::{FixArgs, OutputFormat};
use crate::commands::check::apply_select_ignore;
use crate::config::{
    attach_capture, build_runtime, resolve_llm_client, shared_sources, value_source, AdeptConfig,
};

/// Exit code contract: 0 = clean/no pending changes, 1 = changes pending
/// (`--check`), 2 = usage/I/O error.
pub const EXIT_OK: i32 = 0;
pub const EXIT_CHANGES_PENDING: i32 = 1;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Run `adept fix`, building its own `tokio` runtime and a real
/// [`adept_score::OpenAiCompatClient`]. Returns the process exit code.
pub fn run(args: &FixArgs, config: &AdeptConfig, quiet: bool) -> i32 {
    let base_url = args
        .base_url
        .clone()
        .or_else(|| config.fix.base_url.clone());
    let model = args.model.clone().or_else(|| config.fix.model.clone());
    let Some((client, resolved)) = resolve_llm_client("fix", base_url, model) else {
        return EXIT_USAGE_ERROR;
    };

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

    let (client, sink) = match attach_capture(
        client,
        args.capture_dir.as_deref(),
        config.fix.capture_dir.as_deref(),
        config.origin_dir.as_deref(),
        |source| capture_metadata(args, config, &resolved, tokenizer, &options, source),
    ) {
        Ok(pair) => pair,
        Err(exit_code) => return exit_code,
    };

    let exit_code = execute(args, quiet, &client, &skills, &options);
    if let Some(sink) = &sink {
        sink.finalize(exit_code);
    }
    exit_code
}

/// The fix rounds themselves plus report rendering and writing, split out
/// of [`run`] so the capture sink can be finalised with the actual exit
/// code.
fn execute(
    args: &FixArgs,
    quiet: bool,
    client: &OpenAiCompatClient,
    skills: &[Skill],
    options: &FixOptions,
) -> i32 {
    let Some(runtime) = build_runtime() else {
        return EXIT_USAGE_ERROR;
    };

    let mut reports: Vec<FixReport> = Vec::new();
    for skill in skills {
        let report = runtime.block_on(fix_skill(client, skill, options));
        match report {
            Ok(report) => reports.push(report),
            Err(err) => {
                eprintln!("adept: error: fixing {}: {err}", skill.frontmatter.name);
                return EXIT_USAGE_ERROR;
            }
        }
    }

    let mode = Mode::resolve(args);

    let mut any_pending = false;
    let mut written = 0usize;

    for report in &reports {
        let files = report.files();
        if files.is_some() {
            any_pending = true;
        }

        match (args.format, mode) {
            (OutputFormat::Human, Mode::Check | Mode::Diff) => print!("{}", report.diff),
            (OutputFormat::Human, Mode::Report | Mode::Write) => print!("{}", report.render()),
            (OutputFormat::Json, Mode::Check | Mode::Diff | Mode::Report | Mode::Write) => {
                match serde_json::to_string_pretty(report) {
                    Ok(json) => println!("{json}"),
                    Err(err) => {
                        eprintln!("adept: error: failed to render JSON: {err}");
                        return EXIT_USAGE_ERROR;
                    }
                }
            }
        }

        if args.write {
            if let Some(files) = files {
                if let Err(err) = write_all_transactionally(files) {
                    eprintln!(
                        "adept: error: failed to write fixes for {}: {err}",
                        report.skill_name
                    );
                    return EXIT_USAGE_ERROR;
                }
                written += 1;
            }
        }
    }

    if args.write && !quiet {
        println!(
            "{written} skill{} fixed, {} unchanged",
            if written == 1 { "" } else { "s" },
            reports.len() - written,
        );
    }

    match mode {
        Mode::Check if any_pending => EXIT_CHANGES_PENDING,
        _ => EXIT_OK,
    }
}

/// Which of `adept fix`'s mutually-exclusive display behaviors is active,
/// resolved once from `FixArgs`'s `write`/`check`/`diff` flags instead of
/// branching on the raw booleans (with `!args.check` as an implicit guard)
/// at three separate call sites.
///
/// `write` and `check` are mutually exclusive at the `clap` level
/// (`conflicts_with`); `diff` may combine with either. Precedence when it
/// does: `--check` drives the exit code and prints the same diff `--diff`
/// would, so `--diff --check` is simply `--check`; otherwise `--diff`
/// selects diff-only output; otherwise the
/// full report is printed (identically whether or not `--write` is also
/// set, since `--write` only changes whether files are written, not what is
/// printed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// No `--write`/`--check`/`--diff`: print the full report, exit 0.
    Report,
    /// `--diff` (without `--check`): print only the unified diff, exit 0.
    Diff,
    /// `--check`: print the unified diff; exit 1 iff any skill has pending
    /// changes, else 0. Never writes (`clap` forbids combining with
    /// `--write`). Printing the diff rather than staying silent matches
    /// `adept fmt --check`, so CI output shows *what* would change.
    Check,
    /// `--write` (without `--check`/`--diff`): print the full report, write
    /// pending files, exit 0 (2 on I/O error).
    Write,
}

impl Mode {
    fn resolve(args: &FixArgs) -> Self {
        if args.check {
            Self::Check
        } else if args.diff {
            Self::Diff
        } else if args.write {
            Self::Write
        } else {
            Self::Report
        }
    }
}

/// Describe the run for `run_metadata.json`: the resolved options plus,
/// under `sources`, which layer supplied each of them. The API key is only
/// ever reported as a boolean — its value is never read here.
fn capture_metadata(
    args: &FixArgs,
    config: &AdeptConfig,
    resolved: &ResolvedLlmConfig,
    tokenizer: adept::Tokenizer,
    options: &FixOptions,
    capture_dir_source: &'static str,
) -> RunMetadata {
    let mut metadata = RunMetadata::new("fix");
    metadata.model = Some(resolved.model.clone());
    metadata.base_url = Some(resolved.base_url.clone());
    metadata.tokenizer = Some(tokenizer.to_string());
    metadata.api_key_present = resolved.api_key.is_some();
    metadata.max_rounds = Some(options.max_rounds);
    metadata.target_path = Some(
        args.paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    );

    metadata.sources = shared_sources(
        args.model.is_some(),
        config.fix.model.is_some(),
        args.base_url.is_some(),
        config.fix.base_url.is_some(),
        args.tokenizer.is_some(),
        config.fix.tokenizer.is_some(),
    );
    metadata.sources.extend([
        (
            "max_rounds".to_string(),
            value_source(
                args.max_rounds.is_some(),
                config.fix.max_rounds.is_some(),
                "",
            ),
        ),
        ("capture_dir".to_string(), capture_dir_source),
    ]);
    metadata
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
) -> Result<String, adept_agent::FixError> {
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
            capture_dir: None,
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
