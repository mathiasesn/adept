//! Parsing SKILL.md files into [`Skill`]s.
//!
//! Parsing is behind the [`SkillParser`] trait so that other Agent Skill
//! ecosystems (with different frontmatter conventions) can plug in their own
//! implementation later; [`AnthropicSkillParser`] is the default,
//! implementing the Anthropic SKILL.md convention (YAML frontmatter with
//! required `name` and `description`, optional `license`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::error::AdeptError;
use crate::frontmatter::{ExtraField, Frontmatter};
use crate::skill::Skill;

const DELIMITER: &str = "---";

/// Parses raw SKILL.md source text into a [`Skill`].
///
/// Implement this trait to support Agent Skill formats other than
/// Anthropic's; the CLI and rule engine only depend on this trait, not on
/// [`AnthropicSkillParser`] directly.
pub trait SkillParser {
    /// Read and parse the file at `path`.
    fn parse(&self, path: &Path) -> Result<Skill, AdeptError> {
        let source = fs::read_to_string(path).map_err(|source| AdeptError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        self.parse_str(path, &source)
    }

    /// Parse already-loaded `source` text, attributing diagnostics to `path`
    /// (which need not exist on disk, e.g. for tests).
    fn parse_str(&self, path: &Path, source: &str) -> Result<Skill, AdeptError>;
}

/// The default [`SkillParser`] implementation, for Anthropic-style
/// SKILL.md files: a `---`-delimited YAML frontmatter block with required
/// `name` and `description` keys, followed by a markdown body.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnthropicSkillParser;

impl SkillParser for AnthropicSkillParser {
    fn parse_str(&self, path: &Path, source: &str) -> Result<Skill, AdeptError> {
        // Split into lines on '\n' only, then strip a trailing '\r' from
        // each line, so this works uniformly for LF and CRLF files.
        let raw_lines: Vec<&str> = source.split('\n').collect();

        let first_line = raw_lines.first().map(|l| trim_cr(l));
        if first_line != Some(DELIMITER) {
            return Err(AdeptError::MissingFrontmatter {
                path: path.to_path_buf(),
            });
        }

        let end_idx = raw_lines
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, line)| trim_cr(line) == DELIMITER)
            .map(|(i, _)| i);

        let Some(end_idx) = end_idx else {
            return Err(AdeptError::UnterminatedFrontmatter {
                path: path.to_path_buf(),
            });
        };

        // Frontmatter content lines are raw_lines[1..end_idx], corresponding
        // to file lines 2..=end_idx (1-based).
        let fm_lines = &raw_lines[1..end_idx];
        let fm_text = fm_lines
            .iter()
            .map(|l| trim_cr(l))
            .collect::<Vec<_>>()
            .join("\n");

        let body_start_idx = end_idx + 1;
        let body_line_offset = body_start_idx + 1;
        let body = if body_start_idx < raw_lines.len() {
            raw_lines[body_start_idx..].join("\n")
        } else {
            String::new()
        };

        let frontmatter = parse_frontmatter(path, &fm_text, fm_lines)?;

        Ok(Skill {
            path: path.to_path_buf(),
            frontmatter,
            body,
            body_line_offset,
            source: source.to_string(),
        })
    }
}

fn trim_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// Find the 1-based file line number of a top-level `key:` in `fm_lines`,
/// where `fm_lines[0]` is file line `fm_first_file_line`.
fn find_key_line(fm_lines: &[&str], key: &str, fm_first_file_line: usize) -> Option<usize> {
    let prefix = format!("{key}:");
    fm_lines
        .iter()
        .position(|line| {
            let trimmed = trim_cr(line);
            // Top-level keys have no leading whitespace.
            !trimmed.starts_with(' ') && !trimmed.starts_with('\t') && trimmed.starts_with(&prefix)
        })
        .map(|i| fm_first_file_line + i)
}

fn parse_frontmatter(
    path: &Path,
    fm_text: &str,
    fm_lines: &[&str],
) -> Result<Frontmatter, AdeptError> {
    // Frontmatter content starts on file line 2 (line 1 is the opening
    // '---').
    const FM_FIRST_FILE_LINE: usize = 2;

    let value: serde_yaml::Value =
        serde_yaml::from_str(fm_text).map_err(|source| AdeptError::InvalidYaml {
            path: path.to_path_buf(),
            source,
        })?;

    let mapping = match value {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => serde_yaml::Mapping::new(),
        _ => {
            return Err(AdeptError::FrontmatterNotMapping {
                path: path.to_path_buf(),
            })
        }
    };

    let name = required_string_field(path, &mapping, "name")?;
    let name_line =
        find_key_line(fm_lines, "name", FM_FIRST_FILE_LINE).unwrap_or(FM_FIRST_FILE_LINE);

    let description = required_string_field(path, &mapping, "description")?;
    let description_line =
        find_key_line(fm_lines, "description", FM_FIRST_FILE_LINE).unwrap_or(FM_FIRST_FILE_LINE);

    let license = optional_string_field(path, &mapping, "license")?;
    let license_line = license
        .as_ref()
        .and_then(|_| find_key_line(fm_lines, "license", FM_FIRST_FILE_LINE));

    let known = ["name", "description", "license"];
    let mut extra = BTreeMap::new();
    for (k, v) in mapping.iter() {
        let serde_yaml::Value::String(key) = k else {
            continue;
        };
        if known.contains(&key.as_str()) {
            continue;
        }
        let line = find_key_line(fm_lines, key, FM_FIRST_FILE_LINE).unwrap_or(FM_FIRST_FILE_LINE);
        extra.insert(
            key.clone(),
            ExtraField {
                value: v.clone(),
                line,
            },
        );
    }

    Ok(Frontmatter {
        name,
        name_line,
        description,
        description_line,
        license,
        license_line,
        extra,
    })
}

fn required_string_field(
    path: &Path,
    mapping: &serde_yaml::Mapping,
    field: &'static str,
) -> Result<String, AdeptError> {
    match mapping.get(field) {
        Some(serde_yaml::Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(AdeptError::InvalidFieldType {
            path: path.to_path_buf(),
            field,
        }),
        None => Err(AdeptError::MissingField {
            path: path.to_path_buf(),
            field,
        }),
    }
}

fn optional_string_field(
    path: &Path,
    mapping: &serde_yaml::Mapping,
    field: &'static str,
) -> Result<Option<String>, AdeptError> {
    match mapping.get(field) {
        Some(serde_yaml::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(AdeptError::InvalidFieldType {
            path: path.to_path_buf(),
            field,
        }),
        None => Ok(None),
    }
}
