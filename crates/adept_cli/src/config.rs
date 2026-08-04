//! Config file discovery and loading for `adept.toml`.
//!
//! Precedence (highest to lowest): CLI flag > config file value > built-in
//! default. `--config <path>` forces a specific file instead of walking up
//! from the target path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adept::LintConfig;
use adept_agent::{
    CaptureSink, ConfigError, LlmConfig, OpenAiCompatClient, ResolvedLlmConfig, RunMetadata,
    ENV_API_KEY, ENV_BASE_URL, ENV_MODEL,
};
use adept_fmt::FmtConfig;
use serde::Deserialize;

const CONFIG_FILE_NAME: &str = "adept.toml";

/// Resolve an LLM client and its resolved configuration for `adept eval` or
/// `adept fix`, given the CLI/config-file `base_url`/`model` overrides for
/// that command's own section.
///
/// `section` names the config-file table to mention in the error message
/// (`"eval"` or `"fix"`) — the two sections are resolved fully
/// independently (no cross-fallback between them), only sharing the
/// `ADEPT_*` environment variables via [`LlmConfig::resolve`].
///
/// On failure, prints guidance specific to the resolution error (naming
/// `section`) and returns `None`: the original three-line model guidance for
/// [`ConfigError::MissingModel`], or credential-specific guidance for
/// [`ConfigError::CredentialsInBaseUrl`].
#[must_use]
pub fn resolve_llm_client(
    section: &str,
    base_url: Option<String>,
    model: Option<String>,
) -> Option<(OpenAiCompatClient, ResolvedLlmConfig)> {
    let llm_config = LlmConfig {
        base_url,
        api_key: None,
        model,
    };
    let resolved = match llm_config.resolve() {
        Ok(resolved) => resolved,
        Err(ConfigError::MissingModel) => {
            eprintln!("adept: error: could not resolve an LLM model to {section} with.");
            eprintln!(
                "  set one of: --model <MODEL>, config file `[{section}] model = \"...\"`, or the {ENV_MODEL} environment variable"
            );
            eprintln!(
                "  optionally also set {ENV_BASE_URL} (defaults to the OpenAI API) and {ENV_API_KEY}"
            );
            return None;
        }
        Err(ConfigError::CredentialsInBaseUrl) => {
            eprintln!(
                "adept: error: base_url must not embed credentials (user:pass@host) for {section}."
            );
            eprintln!(
                "  set the credential separately: config file `[{section}] api_key = \"...\"`, or the {ENV_API_KEY} environment variable"
            );
            eprintln!("  then pass --base-url without the embedded userinfo");
            return None;
        }
    };
    let client = OpenAiCompatClient::new(resolved.clone());
    Some((client, resolved))
}

/// Where a resolved value came from, recorded in a capture run's
/// `run_metadata.json` so a captured run is self-describing (config
/// precedence is itself under test).
pub const SOURCE_FLAG: &str = "cli-flag";
/// See [`SOURCE_FLAG`]: the value came from `adept.toml`.
pub const SOURCE_CONFIG_FILE: &str = "adept.toml";
/// See [`SOURCE_FLAG`]: the value came from an `ADEPT_*` environment variable.
pub const SOURCE_ENV: &str = "env";
/// See [`SOURCE_FLAG`]: nothing supplied the value, so the built-in default applies.
pub const SOURCE_DEFAULT: &str = "default";

/// Classify which layer supplied a resolved value, for capture metadata.
///
/// `env_var` is the `ADEPT_*` variable consulted for this value, or `""`
/// for values with no environment layer (e.g. the tokenizer).
#[must_use]
pub fn value_source(from_flag: bool, from_config_file: bool, env_var: &str) -> &'static str {
    if from_flag {
        SOURCE_FLAG
    } else if from_config_file {
        SOURCE_CONFIG_FILE
    } else if !env_var.is_empty() && std::env::var_os(env_var).is_some() {
        SOURCE_ENV
    } else {
        SOURCE_DEFAULT
    }
}

/// Resolve the effective capture directory for `eval` or `fix`.
///
/// Precedence is the documented **CLI flag > `adept.toml` > off**. The two
/// layers differ in how a *relative* path is anchored: a `--capture-dir`
/// relative path resolves against the process CWD (returned as-is, for the
/// OS to resolve), while a `capture_dir` from `adept.toml` resolves against
/// `origin_dir`, the directory holding that file.
///
/// Returns the directory paired with the [`SOURCE_FLAG`]/
/// [`SOURCE_CONFIG_FILE`] label for capture metadata, or `None` when
/// capture is off.
#[must_use]
pub fn resolve_capture_dir(
    flag: Option<&Path>,
    file_value: Option<&Path>,
    origin_dir: Option<&Path>,
) -> Option<(PathBuf, &'static str)> {
    if let Some(path) = flag {
        return Some((path.to_path_buf(), SOURCE_FLAG));
    }
    let path = file_value?;
    if path.is_absolute() {
        return Some((path.to_path_buf(), SOURCE_CONFIG_FILE));
    }
    let anchored = match origin_dir {
        Some(dir) if dir.as_os_str().is_empty() => path.to_path_buf(),
        Some(dir) => dir.join(path),
        None => path.to_path_buf(),
    };
    Some((anchored, SOURCE_CONFIG_FILE))
}

/// Resolve the capture directory for `eval`/`fix` and, if capture is on,
/// create the sink and attach it to `client`.
///
/// Returns the (possibly capture-enabled) client alongside the sink the
/// caller must [`adept_agent::CaptureSink::finalize`] with its exit code, or
/// `Err(2)` — the usage-error exit code — when the directory cannot be
/// created. Capture is opt-in and requested explicitly, so failing to create
/// the directory is a usage error rather than a silent skip.
///
/// `metadata` is a closure rather than a value because the run metadata
/// records *which layer* supplied `capture_dir`, and that is only known once
/// [`resolve_capture_dir`] has run.
pub fn attach_capture(
    client: OpenAiCompatClient,
    flag: Option<&Path>,
    file_value: Option<&Path>,
    origin_dir: Option<&Path>,
    metadata: impl FnOnce(&'static str) -> RunMetadata,
) -> Result<(OpenAiCompatClient, Option<Arc<CaptureSink>>), i32> {
    let Some((dir, source)) = resolve_capture_dir(flag, file_value, origin_dir) else {
        return Ok((client, None));
    };
    match CaptureSink::new(&dir, metadata(source)) {
        Ok(sink) => {
            let sink = Arc::new(sink);
            Ok((client.with_capture(Arc::clone(&sink)), Some(sink)))
        }
        Err(err) => {
            eprintln!(
                "adept: error: failed to create capture directory {}: {err}",
                dir.display()
            );
            Err(2)
        }
    }
}

/// The provenance entries `score` and `fix` record identically: the model,
/// base URL, and tokenizer, each labelled with the layer that supplied it.
/// Each command extends the returned map with its own keys.
#[must_use]
pub fn shared_sources(
    model_from_flag: bool,
    model_from_file: bool,
    base_url_from_flag: bool,
    base_url_from_file: bool,
    tokenizer_from_flag: bool,
    tokenizer_from_file: bool,
) -> BTreeMap<String, &'static str> {
    BTreeMap::from([
        (
            "model".to_string(),
            value_source(model_from_flag, model_from_file, ENV_MODEL),
        ),
        (
            "base_url".to_string(),
            value_source(base_url_from_flag, base_url_from_file, ENV_BASE_URL),
        ),
        (
            "tokenizer".to_string(),
            value_source(tokenizer_from_flag, tokenizer_from_file, ""),
        ),
    ])
}

/// Build the `tokio` runtime `adept eval`/`adept fix` drive their single
/// async call from, printing the shared error message on failure.
#[must_use]
pub fn build_runtime() -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Runtime::new() {
        Ok(rt) => Some(rt),
        Err(err) => {
            eprintln!("adept: error: failed to start async runtime: {err}");
            None
        }
    }
}

/// LLM-related settings that can be set via config file, layered under CLI
/// flags and `ADEPT_*` environment variables by [`adept_agent::LlmConfig::resolve`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EvalFileConfig {
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// Which `tiktoken-rs` BPE encoding to use for token-bloat analysis.
    /// `None` falls back to [`adept::Tokenizer::default`] (`o200k_base`).
    pub tokenizer: Option<adept::Tokenizer>,
    /// Directory to write verbatim LLM call artifacts into. `None` (the
    /// default) disables capture. Overridden by `--capture-dir`; a relative
    /// path resolves against the directory holding this `adept.toml` — see
    /// [`resolve_capture_dir`].
    pub capture_dir: Option<PathBuf>,
}

/// LLM-related settings for `adept fix`, layered under CLI flags and
/// `ADEPT_*` environment variables by [`adept_agent::LlmConfig::resolve`].
/// Kept fully independent of [`EvalFileConfig`]: `[fix]` never falls back
/// to `[eval]` or vice versa — the only shared fallback is the `ADEPT_*`
/// environment variables.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FixFileConfig {
    /// The model identifier to request for fix rewrites, e.g.
    /// `"gpt-4o-mini"`. `None` falls back to `--model` or `ADEPT_MODEL`;
    /// with none of those set, `adept fix` exits 2.
    pub model: Option<String>,
    /// The base URL of the OpenAI-compatible endpoint to call. `None` falls
    /// back to `--base-url`, `ADEPT_BASE_URL`, or the OpenAI default.
    pub base_url: Option<String>,
    /// Which `tiktoken-rs` BPE encoding to use for token counting. `None`
    /// falls back to [`adept::Tokenizer::default`] (`o200k_base`).
    pub tokenizer: Option<adept::Tokenizer>,
    /// The maximum number of fix rounds to attempt before giving up.
    /// `None` falls back to [`adept_agent::DEFAULT_MAX_ROUNDS`].
    pub max_rounds: Option<usize>,
    /// Directory to write verbatim LLM call artifacts into. `None` (the
    /// default) disables capture. Overridden by `--capture-dir`; a relative
    /// path resolves against the directory holding this `adept.toml` — see
    /// [`resolve_capture_dir`]. Independent of `[eval] capture_dir`.
    pub capture_dir: Option<PathBuf>,
}

/// LLM-related settings for `adept create`, layered under CLI flags and
/// `ADEPT_*` environment variables by [`adept_agent::LlmConfig::resolve`].
/// Kept fully independent of [`EvalFileConfig`]/[`FixFileConfig`]: `[create]`
/// never falls back to `[eval]` or `[fix]` or vice versa — the only shared
/// fallback is the `ADEPT_*` environment variables.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CreateFileConfig {
    /// The model identifier to request for both the authoring and
    /// eval-generation calls, e.g. `"gpt-4o"`. `None` falls back to
    /// `--model` or `ADEPT_MODEL`; with none of those set, `adept create`
    /// exits 2.
    pub model: Option<String>,
    /// The base URL of the OpenAI-compatible endpoint to call. `None` falls
    /// back to `--base-url`, `ADEPT_BASE_URL`, or the OpenAI default.
    pub base_url: Option<String>,
    /// Which `tiktoken-rs` BPE encoding to use for token counting. `None`
    /// falls back to [`adept::Tokenizer::default`] (`o200k_base`).
    pub tokenizer: Option<adept::Tokenizer>,
    /// The maximum number of authoring rounds to attempt before giving up.
    /// `None` falls back to [`adept_agent::DEFAULT_MAX_ROUNDS`].
    pub max_rounds: Option<usize>,
    /// How many synthetic eval cases to generate. `None` falls back to
    /// [`adept_agent::create::DEFAULT_EVAL_CASES`]. No CLI flag.
    pub eval_cases: Option<usize>,
    /// Directory to write verbatim LLM call artifacts into. `None` (the
    /// default) disables capture. Overridden by `--capture-dir`; a relative
    /// path resolves against the directory holding this `adept.toml` — see
    /// [`resolve_capture_dir`]. Independent of `[eval]`/`[fix] capture_dir`.
    pub capture_dir: Option<PathBuf>,
}

/// The full deserialized shape of an `adept.toml` config file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AdeptConfig {
    pub lint: LintConfig,
    pub fmt: FmtConfig,
    pub eval: EvalFileConfig,
    pub fix: FixFileConfig,
    pub create: CreateFileConfig,
    /// The directory containing the `adept.toml` this config was loaded
    /// from, or `None` when no file was found (built-in defaults).
    ///
    /// Not a config key — `#[serde(skip)]` keeps it unwritable from TOML.
    /// It exists because relative paths inside a config file are documented
    /// to resolve against the file's own directory rather than the process
    /// CWD, so every such key needs the origin at resolution time.
    #[serde(skip)]
    pub origin_dir: Option<PathBuf>,
}

/// Error loading or parsing a config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    /// The config file still has a `[score]` section, from before `adept
    /// score` was renamed to `adept eval`. Deliberately a hard error rather
    /// than silently ignored: `AdeptConfig` has no `deny_unknown_fields`, so
    /// a stale `[score]` table would otherwise parse, be dropped, and quietly
    /// stop applying the user's `model`/`capture_dir`/`num_prompts`.
    #[error("{path}: `[score]` is no longer read; rename it to `[eval]`")]
    LegacyScoreSection { path: PathBuf },
    /// `[fmt] heading-style` and `[lint] heading_style` are both present but
    /// name different styles. `adept` cannot depend on `adept_fmt`, so the
    /// setting is duplicated across the two sections (see
    /// `docs/ARCHI.md`); silently picking one would let `fmt` and `check`
    /// disagree about the house style, so this fails closed instead.
    #[error(
        "{path}: `[fmt] heading-style = \"{fmt_value}\"` and `[lint] heading_style = \"{lint_value}\"` disagree; make them match"
    )]
    HeadingStyleMismatch {
        path: PathBuf,
        fmt_value: String,
        lint_value: String,
    },
    /// Exactly one of `[fmt] heading-style` / `[lint] heading_style` is
    /// present. Rather than silently defaulting the other to ATX (which
    /// could disagree with the one the user did set), this is a hard error
    /// naming the exact line to add.
    #[error(
        "{path}: `[fmt] heading-style` and `[lint] heading_style` must both be set or both be absent; add `{missing_key} = \"{value}\"` to the `[{missing_section}]` section"
    )]
    HeadingStyleOnlyOnePresent {
        path: PathBuf,
        missing_section: &'static str,
        missing_key: &'static str,
        value: String,
    },
}

/// Walk upward from `start` (a file or directory) looking for `adept.toml`,
/// returning the first one found.
pub fn discover_config_file(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    // Canonicalize best-effort so relative starting paths still walk up
    // correctly; fall back to the given path if canonicalization fails
    // (e.g. the path doesn't exist).
    if let Ok(canonical) = dir.canonicalize() {
        dir = canonical;
    }
    loop {
        let candidate = dir.join(CONFIG_FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Load and parse a config file from an explicit path.
pub fn load_config_file(path: &Path) -> Result<AdeptConfig, ConfigLoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigLoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    // The two checks below both need the raw document rather than the
    // deserialized `AdeptConfig`, so parse it once and share it. A
    // malformed document is not their concern: they see an empty table and
    // the `toml::from_str` below still reports the syntax error.
    let raw = match text.parse::<toml::Value>() {
        Ok(toml::Value::Table(table)) => table,
        _ => toml::Table::new(),
    };
    // `[score]` is the pre-rename section name. Checked here rather than
    // during deserialization because `AdeptConfig` has no
    // `deny_unknown_fields` and would otherwise parse a stale `[score]`
    // section, silently ignore it, and quietly drop the user's
    // `model`/`capture_dir`/`num_prompts`.
    if raw.contains_key("score") {
        return Err(ConfigLoadError::LegacyScoreSection {
            path: path.to_path_buf(),
        });
    }
    let (fmt_heading_style, lint_heading_style) = heading_style_values(&raw);
    match (fmt_heading_style, lint_heading_style) {
        (Some(fmt_value), Some(lint_value)) if fmt_value != lint_value => {
            return Err(ConfigLoadError::HeadingStyleMismatch {
                path: path.to_path_buf(),
                fmt_value,
                lint_value,
            });
        }
        (Some(value), None) => {
            return Err(ConfigLoadError::HeadingStyleOnlyOnePresent {
                path: path.to_path_buf(),
                missing_section: "lint",
                missing_key: "heading_style",
                value,
            });
        }
        (None, Some(value)) => {
            return Err(ConfigLoadError::HeadingStyleOnlyOnePresent {
                path: path.to_path_buf(),
                missing_section: "fmt",
                missing_key: "heading-style",
                value,
            });
        }
        _ => {}
    }
    let mut config: AdeptConfig =
        toml::from_str(&text).map_err(|source| ConfigLoadError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    config.origin_dir = path.parent().map(Path::to_path_buf);
    Ok(config)
}

/// The raw string values of `[fmt] heading-style` and `[lint] heading_style`
/// in an `adept.toml`, if present, read from the raw document rather than
/// the deserialized `AdeptConfig` — `#[serde(default)]` cannot distinguish
/// "absent" from "explicitly set to the default", which is exactly the
/// distinction the cross-validation above needs.
fn heading_style_values(table: &toml::Table) -> (Option<String>, Option<String>) {
    let fmt_value = table
        .get("fmt")
        .and_then(|v| v.get("heading-style"))
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let lint_value = table
        .get("lint")
        .and_then(|v| v.get("heading_style"))
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    (fmt_value, lint_value)
}

/// Resolve the effective config: `--config` forces a specific file;
/// otherwise walk up from `target` looking for `adept.toml`. Returns the
/// default config if none is found.
pub fn resolve_config(
    explicit: Option<&Path>,
    target: &Path,
) -> Result<AdeptConfig, ConfigLoadError> {
    if let Some(path) = explicit {
        return load_config_file(path);
    }
    match discover_config_file(target) {
        Some(path) => load_config_file(&path),
        None => Ok(AdeptConfig::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_config_walking_up_from_a_nested_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[lint]\nbody_max_tokens = 42\n",
        )
        .unwrap();
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();

        let found = discover_config_file(&nested).expect("should find adept.toml");
        assert_eq!(found, dir.path().join("adept.toml").canonicalize().unwrap());

        let config = load_config_file(&found).unwrap();
        assert_eq!(config.lint.body_max_tokens, 42);
    }

    #[test]
    fn missing_config_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = resolve_config(None, dir.path()).unwrap();
        assert_eq!(
            config.lint.body_max_tokens,
            LintConfig::default().body_max_tokens
        );
    }

    #[test]
    fn explicit_config_path_skips_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let explicit_path = dir.path().join("custom.toml");
        std::fs::write(&explicit_path, "[fmt]\nline-width = 60\n").unwrap();

        let config = resolve_config(Some(&explicit_path), dir.path()).unwrap();
        assert_eq!(config.fmt.line_width, 60);
    }

    #[test]
    fn explicit_missing_config_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        assert!(resolve_config(Some(&missing), dir.path()).is_err());
    }

    #[test]
    fn fix_and_eval_sections_are_parsed_independently() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[eval]\nmodel = \"eval-model\"\n\n[fix]\nmodel = \"fix-model\"\nmax_rounds = 5\n",
        )
        .unwrap();

        let config = resolve_config(None, dir.path()).unwrap();
        assert_eq!(config.eval.model.as_deref(), Some("eval-model"));
        assert_eq!(config.fix.model.as_deref(), Some("fix-model"));
        assert_eq!(config.fix.max_rounds, Some(5));
        assert_eq!(config.eval.base_url, None);
        assert_eq!(config.fix.base_url, None);
    }

    #[test]
    fn legacy_score_section_is_a_hard_error_naming_eval() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[score]\nmodel = \"old-model\"\n",
        )
        .unwrap();

        let err = resolve_config(None, dir.path()).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("[score]") && message.contains("[eval]"),
            "message should name both the removed and replacement sections: {message}"
        );
    }

    #[test]
    fn capture_dir_flag_wins_over_config_file() {
        let resolved = resolve_capture_dir(
            Some(Path::new("from-flag")),
            Some(Path::new("from-toml")),
            Some(Path::new("/cfg")),
        );
        assert_eq!(
            resolved,
            Some((PathBuf::from("from-flag"), SOURCE_FLAG)),
            "a --capture-dir relative path anchors on the CWD, not the config dir"
        );
    }

    #[test]
    fn capture_dir_from_config_file_anchors_on_the_config_directory() {
        let resolved =
            resolve_capture_dir(None, Some(Path::new("cap")), Some(Path::new("/cfg/dir")));
        assert_eq!(
            resolved,
            Some((PathBuf::from("/cfg/dir/cap"), SOURCE_CONFIG_FILE))
        );

        // An absolute config value is used verbatim.
        let resolved =
            resolve_capture_dir(None, Some(Path::new("/abs/cap")), Some(Path::new("/cfg")));
        assert_eq!(
            resolved,
            Some((PathBuf::from("/abs/cap"), SOURCE_CONFIG_FILE))
        );
    }

    #[test]
    fn capture_dir_defaults_to_off() {
        assert_eq!(
            resolve_capture_dir(None, None, Some(Path::new("/cfg"))),
            None
        );
    }

    #[test]
    fn capture_dir_sections_do_not_cross_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[eval]\ncapture_dir = \"eval-cap\"\n",
        )
        .unwrap();

        let config = resolve_config(None, dir.path()).unwrap();
        assert_eq!(config.eval.capture_dir, Some(PathBuf::from("eval-cap")));
        assert_eq!(
            config.fix.capture_dir, None,
            "[fix] must never inherit [eval] capture_dir"
        );

        let origin = config.origin_dir.clone();
        assert!(
            resolve_capture_dir(None, config.fix.capture_dir.as_deref(), origin.as_deref())
                .is_none()
        );
        let (eval_dir, source) =
            resolve_capture_dir(None, config.eval.capture_dir.as_deref(), origin.as_deref())
                .unwrap();
        assert_eq!(source, SOURCE_CONFIG_FILE);
        assert_eq!(eval_dir, origin.unwrap().join("eval-cap"));
    }

    #[test]
    fn origin_dir_is_the_config_files_directory_and_is_not_a_toml_key() {
        let dir = tempfile::tempdir().unwrap();
        // `origin_dir` is `#[serde(skip)]`, so a TOML key of that name is
        // ignored rather than honoured.
        std::fs::write(
            dir.path().join("adept.toml"),
            "origin_dir = \"/attacker/controlled\"\n[fix]\ncapture_dir = \"fix-cap\"\n",
        )
        .unwrap();

        let config = resolve_config(None, dir.path()).unwrap();
        let origin = config.origin_dir.clone().unwrap();
        assert_ne!(origin, PathBuf::from("/attacker/controlled"));
        assert_eq!(origin, dir.path().canonicalize().unwrap());
        assert_eq!(
            resolve_capture_dir(None, config.fix.capture_dir.as_deref(), Some(&origin)),
            Some((origin.join("fix-cap"), SOURCE_CONFIG_FILE))
        );
    }

    #[test]
    fn value_source_reports_the_winning_layer() {
        assert_eq!(value_source(true, true, ""), SOURCE_FLAG);
        assert_eq!(value_source(false, true, ""), SOURCE_CONFIG_FILE);
        assert_eq!(value_source(false, false, ""), SOURCE_DEFAULT);
        // Env is only consulted when neither flag nor file supplied a value.
        assert_eq!(
            value_source(false, false, "ADEPT_DEFINITELY_UNSET_FOR_TESTS"),
            SOURCE_DEFAULT
        );
    }

    #[test]
    fn heading_style_both_absent_defaults_to_atx_with_no_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[lint]\nbody_max_tokens = 42\n",
        )
        .unwrap();

        let config = resolve_config(None, dir.path()).unwrap();
        assert_eq!(config.lint.heading_style, adept::HeadingStyle::default());
        assert_eq!(config.fmt.heading_style, adept_fmt::HeadingStyle::default());
    }

    #[test]
    fn heading_style_both_present_and_equal_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[fmt]\nheading-style = \"setext\"\n[lint]\nheading_style = \"setext\"\n",
        )
        .unwrap();

        let config = resolve_config(None, dir.path()).unwrap();
        assert_eq!(config.lint.heading_style, adept::HeadingStyle::Setext);
        assert_eq!(config.fmt.heading_style, adept_fmt::HeadingStyle::Setext);
    }

    #[test]
    fn heading_style_present_and_different_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[fmt]\nheading-style = \"setext\"\n[lint]\nheading_style = \"atx\"\n",
        )
        .unwrap();

        let err = resolve_config(None, dir.path()).unwrap_err();
        match &err {
            ConfigLoadError::HeadingStyleMismatch {
                fmt_value,
                lint_value,
                ..
            } => {
                assert_eq!(fmt_value, "setext");
                assert_eq!(lint_value, "atx");
            }
            other => panic!("expected HeadingStyleMismatch, got {other:?}"),
        }
        let message = err.to_string();
        assert!(message.contains("heading-style"));
        assert!(message.contains("heading_style"));
        assert!(message.contains("setext"));
        assert!(message.contains("atx"));
    }

    #[test]
    fn heading_style_exactly_one_present_is_a_hard_error_naming_the_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[fmt]\nheading-style = \"setext\"\n",
        )
        .unwrap();

        let err = resolve_config(None, dir.path()).unwrap_err();
        match &err {
            ConfigLoadError::HeadingStyleOnlyOnePresent {
                missing_section,
                missing_key,
                value,
                ..
            } => {
                assert_eq!(*missing_section, "lint");
                assert_eq!(*missing_key, "heading_style");
                assert_eq!(value, "setext");
            }
            other => panic!("expected HeadingStyleOnlyOnePresent, got {other:?}"),
        }
        let message = err.to_string();
        assert!(message.contains("heading_style = \"setext\""));
        assert!(message.contains("[lint]"));

        // And the reverse direction: only `[lint]` set.
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(
            dir2.path().join("adept.toml"),
            "[lint]\nheading_style = \"setext\"\n",
        )
        .unwrap();
        let err2 = resolve_config(None, dir2.path()).unwrap_err();
        match &err2 {
            ConfigLoadError::HeadingStyleOnlyOnePresent {
                missing_section,
                missing_key,
                value,
                ..
            } => {
                assert_eq!(*missing_section, "fmt");
                assert_eq!(*missing_key, "heading-style");
                assert_eq!(value, "setext");
            }
            other => panic!("expected HeadingStyleOnlyOnePresent, got {other:?}"),
        }
    }
}
