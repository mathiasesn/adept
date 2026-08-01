//! `SL1xx` structure rules: checks on the markdown body of a SKILL.md file.

use crate::diagnostic::{Diagnostic, Severity};
use crate::markdown;
use crate::skill::Skill;

use super::{impl_rule, FixKind, LintConfig, Rule, SkillRule};

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
        let mut candidates = markdown::link_destinations(&skill.body);
        candidates.extend(
            markdown::inline_code_spans(&skill.body)
                .into_iter()
                .map(|c| markdown::Located {
                    value: c.value.trim().to_string(),
                    line: c.line,
                })
                .filter(|c| looks_like_explicit_path(&c.value)),
        );
        // Stable, so links still precede code spans on the same line.
        candidates.sort_by_key(|c| c.line);

        // Paths the skill *instructs the reader to create* are not broken
        // references — they legitimately don't exist next to SKILL.md yet.
        // If any occurrence of a path sits on a line phrased as a creation
        // instruction ("Save test cases to `evals/evals.json`"), treat every
        // reference to that same path as skill-authored, including later
        // read/update mentions ("if `evals/evals.json` already exists…").
        // Intent is line-granular: a broken reference sharing a line with an
        // unrelated creation instruction is not flagged. Binding the verb to
        // a specific path would need column tracking through the markdown
        // query layer; the co-occurrence is rare, so it is left as a bound.
        let body_lines: Vec<&str> = skill.body.lines().collect();
        let line_text = |line: usize| body_lines.get(line - 1).copied().unwrap_or("");
        let authored: std::collections::HashSet<&str> = candidates
            .iter()
            .filter(|c| is_intended_file_reference(&c.value))
            .filter(|c| has_creation_intent(line_text(c.line)))
            .map(|c| path_part(&c.value))
            .collect();

        let mut diagnostics = Vec::new();
        for candidate in &candidates {
            let target = &candidate.value;
            if !is_intended_file_reference(target) {
                continue;
            }
            if authored.contains(path_part(target)) {
                continue;
            }
            // Strip a trailing anchor/query before checking existence
            // (e.g. `notes.md#section`); the diagnostic still quotes
            // the original target.
            if !dir.join(path_part(target)).exists() {
                diagnostics.push(
                    Diagnostic::new(
                        self.code(),
                        format!("referenced file \"{target}\" does not exist"),
                        self.default_severity(),
                        &skill.path,
                        skill.body_line_offset + candidate.line - 1,
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
                .with_fix_suggestion({
                    let marker = "#".repeat(h.value.level as usize);
                    if h.value.text.starts_with('#') {
                        format!(
                            "write it as `{marker} {text}` (the leading `{marker}` is the ATX \
                             marker; the heading text itself starts with `#`), or run `adept fmt` \
                             to rewrite it",
                            marker = marker,
                            text = h.value.text,
                        )
                    } else {
                        format!(
                            "write it as `{marker} {text}`, or run `adept fmt` to rewrite it",
                            marker = marker,
                            text = h.value.text,
                        )
                    }
                })
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
    if !s.contains('/') {
        return false;
    }
    // Lowercased once, not once per extension.
    let lower = s.to_lowercase();
    KNOWN_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Verbs that mark a path as something the skill *authors* rather than
/// consumes: the reference is an instruction to write the file, so its
/// absence next to SKILL.md is expected, not a broken reference. Kept
/// deliberately small — every entry widens the class of references SL104
/// stops checking, so the bar is "unambiguously describes *producing* a
/// file". Modification verbs (`update`, `populate`, `store`) are excluded on
/// purpose: they imply the file already exists, so a missing target under
/// them is a genuine broken reference. A path that is genuinely authored
/// *and* later updated is still exempted at the update mention, because the
/// creation line propagates its exemption to every reference of that path.
const CREATION_VERBS: &[&str] = &[
    "create",
    "creates",
    "creating",
    "created",
    "write",
    "writes",
    "writing",
    "wrote",
    "written",
    "save",
    "saves",
    "saving",
    "saved",
    "generate",
    "generates",
    "generating",
    "generated",
    "output",
    "outputs",
    "produce",
    "produces",
    "draft",
    "drafts",
    "drafting",
    "drafted",
];

/// Whether a body line reads as an instruction to create/write a file, used
/// by [`BrokenFileReference`] to exempt skill-authored paths. Tokenizes via
/// the shared [`crate::text::words`] so it matches whole words the same way
/// the other rules do — `regenerate` does not spuriously match `generate`.
/// Intent is judged for the whole line, not bound to a column, so a creation
/// instruction and an unrelated broken reference sharing one line cannot be
/// told apart — an accepted narrow limitation (see the caller).
fn has_creation_intent(line: &str) -> bool {
    crate::text::words(line).any(|w| CREATION_VERBS.contains(&w.as_str()))
}

/// A target with any trailing `#anchor` / `?query` stripped — the part that
/// names a file on disk. Shared by [`is_intended_file_reference`], which
/// judges it, and [`BrokenFileReference`], which checks it for existence.
fn path_part(target: &str) -> &str {
    target.split(['#', '?']).next().unwrap_or(target).trim()
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
    if path_part(target).is_empty() {
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
    if is_archive_internal_path(path_part(target)) {
        return false; // OOXML part name, not a file next to SKILL.md
    }
    true
}

/// The reserved top-level part names of an Open Packaging Conventions (OPC)
/// container — the roots an OOXML file (`.docx`/`.pptx`/`.xlsx`) unzips to,
/// per ECMA-376 Part 1 (Office Open XML) and Part 2 (OPC, ISO/IEC 29500).
/// Skills that manipulate Office documents describe editing these internal
/// parts (`word/document.xml`, `ppt/slides/slideN.xml`), which are format
/// constants, not files bundled next to SKILL.md — so a reference to one is
/// not a broken reference. The set is closed by the standard, so this is
/// verifiable against the spec rather than against any particular skill.
const OPC_ROOTS: &[&str] = &["word", "ppt", "xl", "docProps", "_rels", "customXml"];

/// Whether `path` is an OOXML archive-internal part: a multi-segment path
/// whose first segment is a reserved [`OPC_ROOTS`] name *and* which ends in a
/// part extension (`.xml` or `.rels`), e.g. `word/document.xml`,
/// `ppt/slides/slideN.xml`, `xl/worksheets/sheet1.xml`. A bare `word` with no
/// `/` is not a part reference and is left alone.
///
/// The extension gate matters because the roots (`word`, `ppt`, `xl`) are
/// short, collision-prone directory names. Without it, a genuinely broken
/// reference to a non-part file a skill happens to bundle under such a
/// directory (a helper script `xl/helper.py`, a typo `word/nots.md`) would be
/// silently swallowed by this Error-severity rule. OOXML parts are XML or
/// relationship documents, so gating on `.xml`/`.rels` keeps those broken
/// references firing while still exempting the real part names.
fn is_archive_internal_path(path: &str) -> bool {
    let Some((root, _)) = path.split_once('/') else {
        return false; // no path segment; a bare `word` is not a part
    };
    if !OPC_ROOTS.contains(&root) {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".xml") || lower.ends_with(".rels")
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
        let found = run(
            &BrokenFileReference,
            "intro\n\n[docs](missing/file_(v2).md)\n",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("missing/file_(v2).md"));
        assert_eq!(found[0].line, 7);
    }

    #[test]
    fn creation_instruction_exempts_authored_path() {
        // The skill tells the reader to create the file; its absence is
        // expected, not a broken reference.
        let found = run(
            &BrokenFileReference,
            "Save test cases to `evals/evals.json`.\n",
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn authored_path_is_exempt_at_later_read_and_update_mentions() {
        // One creation instruction clears every reference to the same path,
        // including plain read/update mentions with no verb of their own.
        let body = "Save output to `data/out.json`.\n\n\
            If `data/out.json` already exists, review it.\n\n\
            Update `data/out.json` with the results.\n";
        assert!(run(&BrokenFileReference, body).is_empty());
    }

    #[test]
    fn non_authored_missing_reference_still_fires() {
        // No creation intent anywhere -> a genuinely broken reference is
        // still reported.
        let found = run(
            &BrokenFileReference,
            "See `scripts/helper.py` for details.\n",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("scripts/helper.py"));
    }

    #[test]
    fn modification_verb_alone_does_not_exempt_missing_file() {
        // "update"/"store" imply the file should already exist, so a missing
        // target under them is a genuine broken reference, not skill-authored.
        let found = run(
            &BrokenFileReference,
            "Update `docs/missing.md` with notes.\n",
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("docs/missing.md"));
        assert!(!has_creation_intent("Update the file"));
        assert!(!has_creation_intent("store results there"));
    }

    #[test]
    fn creation_intent_matches_whole_words_only() {
        // `regenerate` must not match `generate`, so an unrelated broken
        // reference on such a line is not spuriously exempted.
        assert!(!has_creation_intent("Nothing here references files"));
        assert!(has_creation_intent("Save to path"));
        assert!(!has_creation_intent("regenerated the report"));
    }

    #[test]
    fn ooxml_archive_internal_paths_are_not_broken_references() {
        // The two references that actually fire in the source-available
        // `docx`/`pptx` skills at the vendored corpus pin: OOXML part names,
        // which are format constants (ECMA-376), not files next to SKILL.md.
        assert!(run(&BrokenFileReference, "Edit `word/document.xml`.\n").is_empty());
        assert!(run(&BrokenFileReference, "Edit `ppt/slides/slideN.xml`.\n").is_empty());
        // Other reserved roots, for future-proofing.
        for path in [
            "xl/worksheets/sheet1.xml",
            "docProps/core.xml",
            "customXml/item1.xml",
        ] {
            assert!(
                run(&BrokenFileReference, &format!("See `{path}`.\n")).is_empty(),
                "{path} should be exempt as an OOXML part"
            );
        }
        // `_rels/.rels` is the real OPC relationships part (no `.xml`); it is
        // not a backtick code-span candidate, so assert the predicate directly.
        assert!(is_archive_internal_path("_rels/.rels"));
    }

    #[test]
    fn opc_root_lookalikes_still_fire() {
        // A path whose first segment merely resembles a part name is a normal
        // relative reference — the exemption is anchored to the exact reserved
        // roots, not a fuzzy prefix. `word` with no `/` is not a part either.
        let found = run(&BrokenFileReference, "See `words/glossary.md`.\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(is_archive_internal_path("word/document.xml"));
        assert!(!is_archive_internal_path("word")); // no path segment
        assert!(!is_archive_internal_path("words/x.md")); // not a reserved root
    }

    #[test]
    fn broken_non_part_under_opc_root_still_fires() {
        // A non-part file (helper script, doc) under an OOXML root is a genuine
        // broken reference, not an archive part: the exemption is gated on the
        // `.xml`/`.rels` part extension, so short collision-prone roots like
        // `xl/` cannot silently swallow a typo'd `.py`/`.md` reference.
        let found = run(&BrokenFileReference, "Run `xl/helper.py`.\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(!is_archive_internal_path("xl/helper.py"));
        assert!(!is_archive_internal_path("word/notes.md"));
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
