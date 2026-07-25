//! Config file discovery and loading for `adept.toml`.
//!
//! Precedence (highest to lowest): CLI flag > config file value > built-in
//! default. `--config <path>` forces a specific file instead of walking up
//! from the target path.

use std::path::{Path, PathBuf};

use adept::LintConfig;
use adept_fmt::FmtConfig;
use serde::Deserialize;

const CONFIG_FILE_NAME: &str = "adept.toml";

/// LLM-related settings that can be set via config file, layered under CLI
/// flags and `ADEPT_*` environment variables by [`adept_score::LlmConfig::resolve`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScoreFileConfig {
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// Which `tiktoken-rs` BPE encoding to use for token-bloat analysis.
    /// `None` falls back to [`adept::Tokenizer::default`] (`o200k_base`).
    pub tokenizer: Option<adept::Tokenizer>,
}

/// LLM-related settings for `adept fix`, layered under CLI flags and
/// `ADEPT_*` environment variables by [`adept_score::LlmConfig::resolve`].
/// Kept fully independent of [`ScoreFileConfig`]: `[fix]` never falls back
/// to `[score]` or vice versa — the only shared fallback is the `ADEPT_*`
/// environment variables.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FixFileConfig {
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// Which `tiktoken-rs` BPE encoding to use for token counting. `None`
    /// falls back to [`adept::Tokenizer::default`] (`o200k_base`).
    pub tokenizer: Option<adept::Tokenizer>,
    /// The maximum number of fix rounds to attempt before giving up.
    /// `None` falls back to [`adept_fix::DEFAULT_MAX_ROUNDS`].
    pub max_rounds: Option<usize>,
}

/// The full deserialized shape of an `adept.toml` config file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AdeptConfig {
    pub lint: LintConfig,
    pub fmt: FmtConfig,
    pub score: ScoreFileConfig,
    pub fix: FixFileConfig,
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
    toml::from_str(&text).map_err(|source| ConfigLoadError::Parse {
        path: path.to_path_buf(),
        source,
    })
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
    fn fix_and_score_sections_are_parsed_independently() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("adept.toml"),
            "[score]\nmodel = \"score-model\"\n\n[fix]\nmodel = \"fix-model\"\nmax_rounds = 5\n",
        )
        .unwrap();

        let config = resolve_config(None, dir.path()).unwrap();
        assert_eq!(config.score.model.as_deref(), Some("score-model"));
        assert_eq!(config.fix.model.as_deref(), Some("fix-model"));
        assert_eq!(config.fix.max_rounds, Some(5));
        assert_eq!(config.score.base_url, None);
        assert_eq!(config.fix.base_url, None);
    }
}
