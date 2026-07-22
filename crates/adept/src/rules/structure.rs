//! `SL1xx` structure rules: checks on the markdown body of a SKILL.md file.

use crate::diagnostic::{Diagnostic, Severity};
use crate::skill::Skill;

use super::{LintConfig, Rule, SkillRule};

/// `SL101` `empty-body`: the markdown body (everything after the
/// frontmatter) is empty or whitespace-only.
pub struct EmptyBody;

impl Rule for EmptyBody {
    fn code(&self) -> &'static str {
        "SL101"
    }
    fn name(&self) -> &'static str {
        "empty-body"
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
}

impl SkillRule for EmptyBody {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        if skill.body.trim().is_empty() {
            vec![Diagnostic::new(
                self.code(),
                "SKILL.md has no body content after the frontmatter",
                self.default_severity(),
                &skill.path,
                skill.body_line_offset,
                1,
            )
            .with_fix_suggestion("add instructions describing how to use the skill")]
        } else {
            Vec::new()
        }
    }
}

/// `SL102` `missing-h1`: the body has no top-level (`#`) heading.
pub struct MissingH1;

impl Rule for MissingH1 {
    fn code(&self) -> &'static str {
        "SL102"
    }
    fn name(&self) -> &'static str {
        "missing-h1"
    }
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
}

impl SkillRule for MissingH1 {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        if skill.body.trim().is_empty() {
            // Reported by SL101 instead.
            return Vec::new();
        }
        let has_h1 = headings(&skill.body).iter().any(|h| h.level == 1);
        if has_h1 {
            Vec::new()
        } else {
            vec![Diagnostic::new(
                self.code(),
                "SKILL.md body has no top-level `#` heading",
                self.default_severity(),
                &skill.path,
                skill.body_line_offset,
                1,
            )
            .with_fix_suggestion("add a single `# Title` heading near the top of the body")]
        }
    }
}

/// `SL103` `heading-skip`: a heading level jumps by more than one, e.g. an
/// `h1` followed directly by an `h3` with no intervening `h2`.
pub struct HeadingLevelSkip;

impl Rule for HeadingLevelSkip {
    fn code(&self) -> &'static str {
        "SL103"
    }
    fn name(&self) -> &'static str {
        "heading-skip"
    }
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
}

impl SkillRule for HeadingLevelSkip {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut max_seen = 0u8;
        for h in headings(&skill.body) {
            if h.level > max_seen + 1 && max_seen > 0 {
                diagnostics.push(
                    Diagnostic::new(
                        self.code(),
                        format!(
                            "heading level jumps from h{max_seen} to h{} (\"{}\") without an intervening heading",
                            h.level, h.text
                        ),
                        self.default_severity(),
                        &skill.path,
                        skill.body_line_offset + h.line_in_body - 1,
                        1,
                    )
                    .with_fix_suggestion(format!(
                        "use h{} instead, or add the missing intervening heading levels",
                        max_seen + 1
                    )),
                );
            }
            max_seen = max_seen.max(h.level);
        }
        diagnostics
    }
}

/// `SL104` `broken-file-reference`: a relative path or markdown link
/// mentioned in the body does not exist on disk next to SKILL.md.
pub struct BrokenFileReference;

impl Rule for BrokenFileReference {
    fn code(&self) -> &'static str {
        "SL104"
    }
    fn name(&self) -> &'static str {
        "broken-file-reference"
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
}

impl SkillRule for BrokenFileReference {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        let Some(dir) = skill.path.parent() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        for (line_in_body, line) in skill.body.lines().enumerate() {
            for target in extract_link_targets(line) {
                if !is_local_relative_path(target) {
                    continue;
                }
                if !dir.join(target).exists() {
                    diagnostics.push(
                        Diagnostic::new(
                            self.code(),
                            format!("referenced file \"{target}\" does not exist"),
                            self.default_severity(),
                            &skill.path,
                            skill.body_line_offset + line_in_body,
                            1,
                        )
                        .with_fix_suggestion(
                            "fix the path, or add the missing file next to SKILL.md",
                        ),
                    );
                }
            }
        }
        diagnostics
    }
}

struct Heading {
    level: u8,
    text: String,
    /// 1-based line number within `body`.
    line_in_body: usize,
}

/// Extract ATX-style (`#`) headings from `body`, skipping fenced code
/// blocks.
fn headings(body: &str) -> Vec<Heading> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for (idx, line) in body.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let level = trimmed.chars().take_while(|&c| c == '#').count();
        if level == 0 || level > 6 {
            continue;
        }
        // Must be followed by a space (or end of line) to count as ATX.
        let rest = &trimmed[level..];
        if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
            continue;
        }
        out.push(Heading {
            level: level as u8,
            text: rest.trim().to_string(),
            line_in_body: idx + 1,
        });
    }
    out
}

/// Extract link/image targets (the `(...)` part of `[text](target)` or
/// `![alt](target)`) and backtick-quoted path-like spans from a single line.
fn extract_link_targets(line: &str) -> Vec<&str> {
    let mut targets = Vec::new();

    // `[label](target)` / `![alt](target)`.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            if let Some(close) = line[i + 2..].find(')') {
                let target = &line[i + 2..i + 2 + close];
                targets.push(target);
                i += 2 + close;
                continue;
            }
        }
        i += 1;
    }

    // Backtick-quoted paths, e.g. `` `scripts/run.py` ``.
    // Every odd-indexed segment (0-based) is inside a pair of backticks.
    for (idx, part) in line.split('`').enumerate() {
        if idx % 2 == 1 && looks_like_path(part) {
            targets.push(part);
        }
    }

    targets
}

fn looks_like_path(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.contains('/')
        && !s.contains(' ')
        && !s.starts_with("http://")
        && !s.starts_with("https://")
}

fn is_local_relative_path(target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() {
        return false;
    }
    if target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.contains("://")
    {
        return false;
    }
    if target.starts_with('/') {
        // Absolute filesystem paths aren't "relative to SKILL.md".
        return false;
    }
    true
}
