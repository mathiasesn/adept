//! Token-bloat analysis: description/body/companion-file token counts via
//! the core [`adept::TokenCounter`], plus LLM-generated trimming
//! suggestions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adept::{Skill, TokenCounter};
use serde::{Deserialize, Serialize};

use crate::llm::{ChatMessage, ChatRequest, LlmClient};
use crate::eval::prompts::{
    render, TOKEN_BLOAT_SUGGESTIONS_SYSTEM, TOKEN_BLOAT_SUGGESTIONS_USER_TEMPLATE,
};
use crate::eval::EvalError;

/// Token-bloat analysis for one skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenBloatReport {
    /// Tokens in the frontmatter `description` field.
    pub description_tokens: usize,
    /// Tokens in the markdown body.
    pub body_tokens: usize,
    /// Tokens in each companion file, keyed by path relative to the skill
    /// directory.
    pub companion_file_tokens: BTreeMap<PathBuf, usize>,
    /// Sum of `description_tokens`, `body_tokens`, and every value in
    /// `companion_file_tokens`.
    pub total_tokens: usize,
    /// Concrete, LLM-generated trimming suggestions. May be empty if the
    /// skill is already lean.
    pub suggestions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawSuggestions {
    suggestions: Vec<String>,
}

/// Companion-file discovery, shared with `adept`'s `SL303` so the two can't
/// disagree about what counts as a companion file. Re-exported here because
/// this module's token-bloat analysis is its main consumer.
pub use adept::discover_companion_files;

fn relative_to(dir: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(dir).unwrap_or(path).to_path_buf()
}

/// Analyze token bloat for `skill`, discovering companion files via
/// [`discover_companion_files`] and asking the LLM for trimming
/// suggestions.
///
/// # Errors
/// Returns [`EvalError`] if the LLM client errors or its response cannot
/// be parsed as the expected JSON shape.
pub async fn analyze_token_bloat(
    client: &dyn LlmClient,
    skill: &Skill,
    counter: &TokenCounter,
    model: &str,
) -> Result<TokenBloatReport, EvalError> {
    let description_tokens = counter.count(&skill.frontmatter.description);
    let body_tokens = counter.count(&skill.body);

    let dir = skill.path.parent().unwrap_or(Path::new(""));
    let mut companion_file_tokens = BTreeMap::new();
    for path in discover_companion_files(skill) {
        if adept::is_eval_dataset(dir, &path) {
            // Synthetic eval datasets are not skill content. Note
            // `discover_companion_files` is non-recursive today, so a nested
            // `evals/` file is never discovered here in the first place;
            // this is defence-in-depth for if discovery ever becomes
            // recursive.
            continue;
        }
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let tokens = counter.count(&contents);
            companion_file_tokens.insert(relative_to(dir, &path), tokens);
        }
    }

    let total_tokens =
        description_tokens + body_tokens + companion_file_tokens.values().sum::<usize>();

    let companion_summary = if companion_file_tokens.is_empty() {
        "(none)".to_string()
    } else {
        companion_file_tokens
            .iter()
            .map(|(path, tokens)| format!("{}: {tokens} tokens", path.display()))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let user = render(
        TOKEN_BLOAT_SUGGESTIONS_USER_TEMPLATE,
        &[
            ("skill_name", &skill.frontmatter.name),
            ("description", &skill.frontmatter.description),
            ("body", &skill.body),
            ("description_tokens", &description_tokens.to_string()),
            ("body_tokens", &body_tokens.to_string()),
            ("companion_tokens_summary", &companion_summary),
        ],
    );
    let request = ChatRequest::new(
        model.to_string(),
        vec![
            ChatMessage::system(TOKEN_BLOAT_SUGGESTIONS_SYSTEM),
            ChatMessage::user(user),
        ],
    )
    .with_temperature(0.0)
    .with_json_response(true);

    let response = client.chat(request).await?;
    let parsed: RawSuggestions = serde_json::from_str(&response.content)
        .map_err(|e| EvalError::MalformedLlmJson(format!("token bloat suggestions: {e}")))?;

    Ok(TokenBloatReport {
        description_tokens,
        body_tokens,
        companion_file_tokens,
        total_tokens,
        suggestions: parsed.suggestions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlmClient;
    use adept::parse_skill;
    use std::io::Write;

    fn write_skill(dir: &Path, name: &str, description: &str, body: &str) -> PathBuf {
        let path = dir.join("SKILL.md");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "---\nname: {name}\ndescription: {description}\n---\n{body}"
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn counts_description_body_and_companions() {
        let dir = tempdir();
        write_skill(&dir, "demo", "A demo skill", "Some body text here.");
        std::fs::write(dir.join("reference.md"), "extra reference content").unwrap();

        let skill = parse_skill(dir.join("SKILL.md")).unwrap();
        let counter = TokenCounter::default();
        let mock = MockLlmClient::with_texts(vec![r#"{"suggestions": ["trim the preamble"]}"#]);

        let report = analyze_token_bloat(&mock, &skill, &counter, "test-model")
            .await
            .unwrap();

        assert!(report.description_tokens > 0);
        assert!(report.body_tokens > 0);
        assert_eq!(report.companion_file_tokens.len(), 1);
        assert_eq!(
            report.total_tokens,
            report.description_tokens
                + report.body_tokens
                + report.companion_file_tokens.values().sum::<usize>()
        );
        assert_eq!(report.suggestions, vec!["trim the preamble".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn malformed_json_is_a_score_error_not_a_panic() {
        let dir = tempdir();
        write_skill(&dir, "demo", "A demo skill", "Body");
        let skill = parse_skill(dir.join("SKILL.md")).unwrap();
        let counter = TokenCounter::default();
        let mock = MockLlmClient::with_texts(vec!["not json"]);

        let err = analyze_token_bloat(&mock, &skill, &counter, "test-model")
            .await
            .unwrap_err();
        assert!(matches!(err, EvalError::MalformedLlmJson(_)));

        std::fs::remove_dir_all(&dir).ok();
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "adept_agent_eval_tokens_test_{}_{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }
}
