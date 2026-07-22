//! `SL1xx` structure rules: checks on the markdown body of a SKILL.md file.

use crate::diagnostic::{Diagnostic, Severity};
use crate::markdown;
use crate::skill::Skill;

use super::{impl_rule, LintConfig, Rule, SkillRule};

/// `SL101` `empty-body`: the markdown body (everything after the
/// frontmatter) is empty or whitespace-only.
pub struct EmptyBody;

impl_rule!(EmptyBody, "SL101", "empty-body", Error);

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

/// `SL102` `missing-h1`: the body has no top-level (`h1`) heading. Both
/// ATX (`# Title`) and setext (`Title` over `=====`) headings count.
pub struct MissingH1;

impl_rule!(MissingH1, "SL102", "missing-h1", Warning);

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
        let has_h1 = markdown::headings(&skill.body)
            .iter()
            .any(|h| h.value.level == 1);
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

impl_rule!(HeadingLevelSkip, "SL103", "heading-skip", Warning);

impl SkillRule for HeadingLevelSkip {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut max_seen = 0u8;
        for located in markdown::headings(&skill.body) {
            let h = located.value;
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
                        skill.body_line_offset + located.line - 1,
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

impl_rule!(BrokenFileReference, "SL104", "broken-file-reference", Error);

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

        // Candidate targets, in document order: every link/image
        // destination, plus backtick-quoted spans that look like an explicit
        // path. Fenced and indented code blocks are excluded by the parser.
        let mut candidates: Vec<(String, usize)> = markdown::link_destinations(&skill.body)
            .into_iter()
            .map(|d| (d.value, d.line))
            .collect();
        candidates.extend(
            markdown::inline_code_spans(&skill.body)
                .into_iter()
                .filter(|c| looks_like_explicit_path(c.value.trim()))
                .map(|c| (c.value.trim().to_string(), c.line)),
        );
        // Stable, so links still precede code spans on the same line.
        candidates.sort_by_key(|(_, line)| *line);

        let mut diagnostics = Vec::new();
        for (target, line_in_body) in candidates {
            let target = target.as_str();
            if !is_intended_file_reference(target) {
                continue;
            }
            // Strip a trailing anchor/query before checking existence
            // (e.g. `notes.md#section`); the diagnostic still quotes
            // the original target.
            let path_part = target.split(['#', '?']).next().unwrap_or(target).trim();
            if !dir.join(path_part).exists() {
                diagnostics.push(
                    Diagnostic::new(
                        self.code(),
                        format!("referenced file \"{target}\" does not exist"),
                        self.default_severity(),
                        &skill.path,
                        skill.body_line_offset + line_in_body - 1,
                        1,
                    )
                    .with_fix_suggestion("fix the path, or add the missing file next to SKILL.md"),
                );
            }
        }
        diagnostics
    }
}

/// `SL105` `setext-heading`: a heading is written in setext form (`Title`
/// underlined with `===` or `---`) rather than ATX form (`# Title`).
pub struct SetextHeading;

impl_rule!(SetextHeading, "SL105", "setext-heading", Info);

impl SkillRule for SetextHeading {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        markdown::headings(&skill.body)
            .into_iter()
            .filter(|h| h.value.is_setext)
            .map(|h| {
                Diagnostic::new(
                    self.code(),
                    format!(
                        "heading \"{}\" uses setext form; `adept fmt` will rewrite it to ATX (h{})",
                        h.value.text, h.value.level
                    ),
                    self.default_severity(),
                    &skill.path,
                    skill.body_line_offset + h.line - 1,
                    1,
                )
                .with_fix_suggestion(format!(
                    "write it as `{} {}`, or run `adept fmt` to rewrite it",
                    "#".repeat(h.value.level as usize),
                    h.value.text
                ))
            })
            .collect()
    }
}

/// File extensions that make a bare backtick-quoted span (not inside a
/// markdown link) worth treating as a candidate file reference, e.g.
/// `` `notes.md` `` or `` `scripts/run.py` ``.
const KNOWN_EXTENSIONS: &[&str] = &[
    ".md",
    ".markdown",
    ".txt",
    ".py",
    ".js",
    ".ts",
    ".jsx",
    ".tsx",
    ".json",
    ".yaml",
    ".yml",
    ".toml",
    ".sh",
    ".bash",
    ".rs",
    ".go",
    ".rb",
    ".java",
    ".c",
    ".cpp",
    ".h",
    ".hpp",
    ".css",
    ".html",
    ".htm",
    ".csv",
    ".xml",
    ".sql",
    ".pdf",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".svg",
    ".ipynb",
    ".cfg",
    ".ini",
    ".env",
];

/// Whether the content of an inline code span looks like it was *intended*
/// as a relative file path: an explicit `./`/`../` prefix, or a path (i.e.
/// containing a `/`) ending in a known file extension. It receives the
/// parsed content of a code span, never raw line text. This is deliberately
/// conservative — it is the signal that lets [`BrokenFileReference`] tell
/// `./notes.md` or `scripts/helper.py` apart from generic bare-word mentions
/// of common filenames (`package.json`, `README.md` used as a technology
/// marker in prose) or non-paths like `@anthropic-ai/sdk` and
/// `shared/managed-agents-*.md`. A bare filename with no directory
/// component (no `/`) is not extracted from backticks at all: markdown
/// links (`[notes](notes.md)`) are the intended way to reference those.
fn looks_like_explicit_path(s: &str) -> bool {
    if s.is_empty() || s.contains(' ') {
        return false;
    }
    if s.starts_with("./") || s.starts_with("../") {
        return true;
    }
    s.contains('/')
        && KNOWN_EXTENSIONS
            .iter()
            .any(|ext| s.to_lowercase().ends_with(ext))
}

/// Whether `target` (a parsed link/image destination, or a code span
/// already known to look like an explicit path) should actually be treated as a
/// repo-relative file reference worth checking for existence, as opposed to
/// a URL, template placeholder, glob pattern, shell/env variable, package
/// name, or absolute/home-relative path.
fn is_intended_file_reference(target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() {
        return false;
    }
    // Strip a trailing anchor/query before judging the path itself.
    let path_part = target.split(['#', '?']).next().unwrap_or(target).trim();
    if path_part.is_empty() {
        // A pure `#anchor` link.
        return false;
    }
    if target.starts_with('#') {
        return false; // in-page anchor
    }
    if target.contains("://") || target.starts_with("mailto:") {
        return false; // URL scheme
    }
    if target.starts_with('~') || target.starts_with('/') {
        return false; // home-relative or absolute, not relative to SKILL.md
    }
    if target.starts_with('@') {
        return false; // scoped package name, e.g. `@anthropic-ai/sdk`
    }
    if target.chars().any(|c| "*?[]{}<>$".contains(c)) {
        return false; // glob metacharacter or template placeholder (`{lang}`, `<VAR>`, `$VAR`)
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{AnthropicSkillParser, SkillParser};
    use std::path::Path;

    fn skill(body: &str) -> Skill {
        let source = format!("---\nname: demo\ndescription: A demo skill for tests.\n---\n{body}");
        AnthropicSkillParser
            .parse_str(Path::new("demo/SKILL.md"), &source)
            .expect("fixture parses")
    }

    fn run(rule: &dyn SkillRule, body: &str) -> Vec<Diagnostic> {
        let skill = skill(body);
        rule.check(
            &skill,
            &LintConfig::default(),
            &crate::token::TokenCounter::new(crate::token::Tokenizer::default()).unwrap(),
        )
    }

    #[test]
    fn setext_heading_is_reported_once_per_heading() {
        let found = run(&SetextHeading, "Title\n=====\n\n## Atx\n\nOther\n-----\n");
        assert_eq!(found.len(), 2);
        assert!(found[0].message.contains("Title"));
        assert!(found[1].message.contains("Other"));
        assert_eq!(found[0].severity, Severity::Info);
    }

    #[test]
    fn atx_only_body_reports_no_setext_headings() {
        assert!(run(&SetextHeading, "# Title\n\n## Sub\n").is_empty());
    }

    #[test]
    fn setext_h1_satisfies_missing_h1() {
        assert!(run(&MissingH1, "Title\n=====\n\nbody\n").is_empty());
    }

    #[test]
    fn broken_reference_line_points_at_the_link() {
        // Line 1 of the body is the file's line 5 (frontmatter is 4 lines).
        let found = run(&BrokenFileReference, "intro\n\n[docs](missing/file_(v2).md)\n");
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("missing/file_(v2).md"));
        assert_eq!(found[0].line, 7);
    }

    #[test]
    fn references_inside_code_blocks_are_ignored() {
        let found = run(
            &BrokenFileReference,
            "```sh\ncat missing/x.md\n~~~\n```\n\n    [a](indented/gone.md)\n",
        );
        assert!(found.is_empty(), "{found:?}");
    }
}
