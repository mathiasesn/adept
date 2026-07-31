//! Parsing the model's JSON fix response and assembling/sandboxing a
//! [`FixCandidate`].

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::fix::FixError;

/// One companion-file edit requested by the model: append
/// `appended_content` to the (possibly new) file at `path`, relative to the
/// skill's own directory.
#[derive(Debug, Clone, Deserialize)]
pub struct CompanionEdit {
    /// A relative path within the skill's own directory, e.g.
    /// `"REFERENCE.md"`. Validated (and resolved to an absolute path) by
    /// [`resolve_companion_path`] before use.
    pub path: String,
    /// The content to append to the (existing or newly created) file.
    pub appended_content: String,
}

/// The model's raw JSON response to a fix request.
///
/// Every field is optional because a description-scoped request only ever
/// sets `description`, and a body-scoped request only ever sets `body` and
/// (optionally) `companion_edits`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FixResponse {
    /// The rewritten description, if the request was description-scoped.
    pub description: Option<String>,
    /// The rewritten body, if the request was body-scoped.
    pub body: Option<String>,
    /// Companion-file relocations, if any.
    pub companion_edits: Option<Vec<CompanionEdit>>,
}

impl FixResponse {
    /// Parse a model response as a [`FixResponse`].
    ///
    /// Tolerates a markdown code fence (` ```json ... ``` ` or plain
    /// ` ``` ... ``` `) wrapped around the JSON, since some
    /// OpenAI-compatible backends do this even when a JSON response format
    /// was requested.
    ///
    /// # Errors
    /// Returns [`FixError::MalformedResponse`] if the (fence-stripped)
    /// content is not valid JSON in the expected shape.
    pub fn parse(content: &str) -> Result<Self, FixError> {
        let stripped = strip_code_fence(content.trim());
        serde_json::from_str(stripped)
            .map_err(|e| FixError::MalformedResponse(format!("fix response: {e}")))
    }
}

/// Strip a single leading/trailing markdown code fence, if present.
///
/// `adept_score` parses model JSON directly with no fence-stripping (its
/// prompts rely on `json_response: true` alone); there is no existing
/// helper of this kind to reuse, so this is a small purpose-built one.
fn strip_code_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    let rest = rest.trim_start_matches('\n');
    match rest.strip_suffix("```") {
        Some(body) => body.trim(),
        None => s,
    }
}

/// A fully assembled fix candidate: the canonicalized new SKILL.md source
/// (after `adept_fmt` formatting) and the full contents of every companion
/// file touched (existing content, if any, plus appended content), keyed by
/// absolute path.
#[derive(Debug, Clone, Default)]
pub struct FixCandidate {
    /// The canonicalized new SKILL.md source.
    pub skill_source: String,
    /// Full new contents of every touched companion file, keyed by
    /// absolute path.
    pub companions: BTreeMap<PathBuf, String>,
}

/// Validate and resolve a [`CompanionEdit::path`] against `skill_dir`,
/// rejecting anything that could let the model write outside the skill's
/// own directory or clobber SKILL.md itself.
///
/// Rejects: absolute paths, any path containing a `..` or root component,
/// any path whose resolved parent directory is not exactly `skill_dir`
/// (which also catches subdirectories), and a path that resolves to
/// `skill_md`. New files directly inside `skill_dir` ARE allowed.
///
/// # Errors
/// Returns [`FixError::UnsafeCompanionPath`] if `raw` fails any of the
/// above checks.
pub fn resolve_companion_path(
    skill_dir: &Path,
    raw: &str,
    skill_md: &Path,
) -> Result<PathBuf, FixError> {
    let reject = || FixError::UnsafeCompanionPath {
        path: raw.to_string(),
    };

    let candidate = Path::new(raw);
    if candidate.is_absolute() || raw.trim().is_empty() {
        return Err(reject());
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(reject());
            }
        }
    }

    let resolved = skill_dir.join(candidate);
    if resolved.parent() != Some(skill_dir) {
        return Err(reject());
    }
    if resolved == skill_md {
        return Err(reject());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let parsed = FixResponse::parse(r#"{"description": "hi"}"#).unwrap();
        assert_eq!(parsed.description.as_deref(), Some("hi"));
    }

    #[test]
    fn parses_fenced_json() {
        let parsed = FixResponse::parse("```json\n{\"description\": \"hi\"}\n```").unwrap();
        assert_eq!(parsed.description.as_deref(), Some("hi"));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(FixResponse::parse("not json").is_err());
    }

    #[test]
    fn resolves_plain_filename() {
        let dir = Path::new("/skills/demo");
        let skill_md = dir.join("SKILL.md");
        let resolved = resolve_companion_path(dir, "REFERENCE.md", &skill_md).unwrap();
        assert_eq!(resolved, dir.join("REFERENCE.md"));
    }

    #[test]
    fn rejects_parent_traversal() {
        let dir = Path::new("/skills/demo");
        let skill_md = dir.join("SKILL.md");
        assert!(matches!(
            resolve_companion_path(dir, "../evil.md", &skill_md),
            Err(FixError::UnsafeCompanionPath { .. })
        ));
    }

    #[test]
    fn rejects_absolute_path() {
        let dir = Path::new("/skills/demo");
        let skill_md = dir.join("SKILL.md");
        assert!(matches!(
            resolve_companion_path(dir, "/etc/passwd", &skill_md),
            Err(FixError::UnsafeCompanionPath { .. })
        ));
    }

    #[test]
    fn rejects_subdirectory() {
        let dir = Path::new("/skills/demo");
        let skill_md = dir.join("SKILL.md");
        assert!(matches!(
            resolve_companion_path(dir, "sub/REFERENCE.md", &skill_md),
            Err(FixError::UnsafeCompanionPath { .. })
        ));
    }

    #[test]
    fn rejects_skill_md_itself() {
        let dir = Path::new("/skills/demo");
        let skill_md = dir.join("SKILL.md");
        assert!(matches!(
            resolve_companion_path(dir, "SKILL.md", &skill_md),
            Err(FixError::UnsafeCompanionPath { .. })
        ));
    }
}
