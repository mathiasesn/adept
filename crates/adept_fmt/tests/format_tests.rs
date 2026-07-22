//! Golden-file, idempotency, and semantic round-trip tests for `adept_fmt`.
//!
//! Every fixture under `tests/fixtures/*.md` is a full SKILL.md file. For
//! each fixture we assert:
//! - a golden-file snapshot of the formatted output (`insta`),
//! - idempotency: `format(format(x)) == format(x)`,
//! - semantic round-trip: the frontmatter's well-known fields/extra values
//!   are unchanged, and the Markdown body's CommonMark event stream is
//!   unchanged modulo whitespace normalization.

use std::fs;
use std::path::Path;

use adept::{AnthropicSkillParser, SkillParser};
use adept_fmt::{format_str, FmtConfig};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn all_fixtures() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(fixtures_dir()).expect("fixtures dir should exist") {
        let entry = entry.expect("dir entry should be readable");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("fixture should have a UTF-8 stem")
                .to_string();
            let content = fs::read_to_string(&path).expect("fixture should be readable");
            out.push((name, content));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Render a `Tag` for semantic comparison, treating fenced and indented
/// code blocks with the same info string as equivalent (adept_fmt
/// intentionally normalizes indented code blocks to fenced ones, which is
/// not a meaning-changing transformation).
fn tag_repr(tag: &Tag<'_>) -> String {
    match tag {
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => format!("CodeBlock({info})"),
        Tag::CodeBlock(CodeBlockKind::Indented) => "CodeBlock()".to_string(),
        other => format!("{other:?}"),
    }
}

/// A simplified, whitespace-normalized representation of a Markdown
/// document's CommonMark event stream, used to assert that formatting
/// doesn't change a document's meaning.
fn semantic_events(source: &str) -> Vec<String> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut out = Vec::new();
    let mut buf = String::new();

    fn flush(buf: &mut String, out: &mut Vec<String>) {
        let norm = buf.split_whitespace().collect::<Vec<_>>().join(" ");
        if !norm.is_empty() {
            out.push(format!("T:{norm}"));
        }
        buf.clear();
    }

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Text(t) => buf.push_str(&t),
            Event::SoftBreak => buf.push(' '),
            Event::Code(t) => {
                flush(&mut buf, &mut out);
                out.push(format!("C:{t}"));
            }
            Event::HardBreak => {
                flush(&mut buf, &mut out);
                out.push("HB".to_string());
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                flush(&mut buf, &mut out);
                out.push(format!("H:{}", t.trim()));
            }
            Event::FootnoteReference(t) => {
                flush(&mut buf, &mut out);
                out.push(format!("FR:{t}"));
            }
            Event::Rule => {
                flush(&mut buf, &mut out);
                out.push("RULE".to_string());
            }
            Event::TaskListMarker(checked) => {
                flush(&mut buf, &mut out);
                out.push(format!("TL:{checked}"));
            }
            // Whether a list item's content is wrapped in an explicit
            // `Paragraph` reflects only CommonMark tight/loose rendering,
            // not the document's meaning (the spec defines tight/loose as
            // purely a rendering concern) — adept_fmt does not attempt to
            // reproduce tightness exactly for items with more than one
            // block, so paragraph wrapper events are excluded here.
            Event::Start(pulldown_cmark::Tag::Paragraph)
            | Event::End(pulldown_cmark::TagEnd::Paragraph) => {
                flush(&mut buf, &mut out);
            }
            Event::Start(tag) => {
                flush(&mut buf, &mut out);
                out.push(format!("S:{}", tag_repr(&tag)));
            }
            Event::End(tag) => {
                flush(&mut buf, &mut out);
                out.push(format!("E:{tag:?}"));
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                flush(&mut buf, &mut out);
                out.push(format!("M:{t}"));
            }
        }
    }
    flush(&mut buf, &mut out);
    out
}

#[test]
fn fixtures_are_present() {
    assert!(
        all_fixtures().len() >= 10,
        "expected at least 10 fixtures covering the required constructs"
    );
}

#[test]
fn snapshots_and_invariants_hold_for_every_fixture() {
    let cfg = FmtConfig::default();
    for (name, source) in all_fixtures() {
        let formatted =
            format_str(&source, &cfg).unwrap_or_else(|e| panic!("fixture {name} failed: {e}"));

        // Golden-file snapshot.
        insta::assert_snapshot!(name.clone(), formatted);

        // Idempotency: formatting the output again must be a no-op.
        let formatted_twice = format_str(&formatted, &cfg)
            .unwrap_or_else(|e| panic!("fixture {name} failed on second pass: {e}"));
        assert_eq!(
            formatted, formatted_twice,
            "fixture {name} is not idempotent"
        );

        // Semantic round-trip: frontmatter fields and body meaning must be
        // preserved.
        let path = Path::new("SKILL.md");
        let original_skill = AnthropicSkillParser
            .parse_str(path, &source)
            .unwrap_or_else(|e| panic!("fixture {name} failed to parse original: {e}"));
        let formatted_skill = AnthropicSkillParser
            .parse_str(path, &formatted)
            .unwrap_or_else(|e| panic!("fixture {name} failed to parse formatted: {e}"));

        assert_eq!(
            original_skill.frontmatter.name, formatted_skill.frontmatter.name,
            "fixture {name}: `name` changed"
        );
        assert_eq!(
            original_skill.frontmatter.description, formatted_skill.frontmatter.description,
            "fixture {name}: `description` changed"
        );
        assert_eq!(
            original_skill.frontmatter.license, formatted_skill.frontmatter.license,
            "fixture {name}: `license` changed"
        );
        for (key, extra) in &original_skill.frontmatter.extra {
            let formatted_value = formatted_skill
                .frontmatter
                .extra
                .get(key)
                .unwrap_or_else(|| panic!("fixture {name}: extra key `{key}` was dropped"));
            assert_eq!(
                extra.value, formatted_value.value,
                "fixture {name}: extra key `{key}` changed value"
            );
        }

        let original_events = semantic_events(&original_skill.body);
        let formatted_events = semantic_events(&formatted_skill.body);
        assert_eq!(
            original_events, formatted_events,
            "fixture {name}: body meaning changed"
        );
    }
}

#[test]
fn already_formatted_fixture_is_byte_identical() {
    let path = fixtures_dir().join("already_formatted.md");
    let source = fs::read_to_string(&path).unwrap();
    let formatted = format_str(&source, &FmtConfig::default()).unwrap();
    assert_eq!(
        source, formatted,
        "already_formatted.md should format to itself byte-for-byte"
    );
}

#[test]
fn crlf_input_is_handled() {
    let lf = fs::read_to_string(fixtures_dir().join("headings.md")).unwrap();
    let crlf = lf.replace('\n', "\r\n");
    let cfg = FmtConfig::default();

    let formatted_from_crlf = format_str(&crlf, &cfg).expect("CRLF input should format");
    let formatted_from_lf = format_str(&lf, &cfg).expect("LF input should format");

    assert_eq!(
        formatted_from_crlf, formatted_from_lf,
        "CRLF and LF input should format identically"
    );
    assert!(
        !formatted_from_crlf.contains('\r'),
        "formatted output should use LF line endings"
    );
}

#[test]
fn idempotency_holds_for_every_fixture_with_prose_reflow_disabled() {
    let cfg = FmtConfig {
        reflow_prose: false,
        ..FmtConfig::default()
    };
    for (name, source) in all_fixtures() {
        let formatted = format_str(&source, &cfg)
            .unwrap_or_else(|e| panic!("fixture {name} failed (no reflow): {e}"));
        let formatted_twice = format_str(&formatted, &cfg)
            .unwrap_or_else(|e| panic!("fixture {name} failed on second pass (no reflow): {e}"));
        assert_eq!(
            formatted, formatted_twice,
            "fixture {name} is not idempotent with reflow_prose = false"
        );
    }
}
