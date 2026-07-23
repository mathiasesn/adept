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
use pulldown_cmark::{CodeBlockKind, Event, Tag};

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
    let mut out = Vec::new();
    let mut buf = String::new();

    fn flush(buf: &mut String, out: &mut Vec<String>) {
        let norm = buf.split_whitespace().collect::<Vec<_>>().join(" ");
        if !norm.is_empty() {
            out.push(format!("T:{norm}"));
        }
        buf.clear();
    }

    for event in adept::markdown::parser(source) {
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

/// Corpus of vendored real-world skills, shared with `crates/adept`'s own
/// corpus tests. Lives under `crates/adept` (not `adept_fmt`) because the
/// corpus lint snapshot test also needs it; resolved cross-crate rather than
/// duplicated. See `specs/vendored-skills-corpus-fixture.md`.
fn corpus_dir() -> std::path::PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../adept/tests/fixtures/corpus"
    ))
    .to_path_buf()
}

/// Skills whose `SKILL.md` is known not to round-trip idempotently under
/// `FmtConfig::default()` (`reflow_prose: true`). Previously this held
/// "algorithmic-art" and "claude-api", both hit by the "leaning toothpick"
/// class of bug where prose reflow could wrap a line so that its first token
/// became something CommonMark treats as block-starting syntax on re-parse
/// (a `-`/`+`/`*` bullet marker) even though it was mid-sentence punctuation
/// in the source. `wrap_tokens` now guards against dangerous line starts
/// (see `dangerous_line_start` in `adept_fmt::print`), so both are fixed and
/// this list is empty; kept as a named slot for any future entries.
const KNOWN_NON_IDEMPOTENT: &[&str] = &[];

/// `format(format(x)) == format(x)` for every vendored corpus `SKILL.md`,
/// under `FmtConfig::default()` — i.e. with `reflow_prose: true`, which is
/// what `adept fmt` actually does. This is the property real prose has never
/// been exercised against in a broad test (the loop above is reflow-disabled).
#[test]
fn idempotency_holds_for_corpus_with_prose_reflow_enabled() {
    let cfg = FmtConfig::default();
    assert!(cfg.reflow_prose, "this test only guards the reflow path");

    let corpus = corpus_dir();
    let mut skills: Vec<_> = fs::read_dir(&corpus)
        .expect("corpus dir should exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    skills.sort();
    assert!(
        !skills.is_empty(),
        "corpus should contain skill directories"
    );

    let mut checked = 0;
    for skill_dir in skills {
        let name = skill_dir
            .file_name()
            .and_then(|s| s.to_str())
            .expect("skill dir should have a UTF-8 name")
            .to_string();
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }
        if KNOWN_NON_IDEMPOTENT.contains(&name.as_str()) {
            continue;
        }
        checked += 1;
        let source = fs::read_to_string(&skill_md)
            .unwrap_or_else(|e| panic!("{name}/SKILL.md should be readable: {e}"));
        let formatted = format_str(&source, &cfg)
            .unwrap_or_else(|e| panic!("corpus skill {name} failed to format: {e}"));
        let formatted_twice = format_str(&formatted, &cfg)
            .unwrap_or_else(|e| panic!("corpus skill {name} failed on second pass: {e}"));
        assert_eq!(
            formatted, formatted_twice,
            "corpus skill {name} is not idempotent with reflow_prose = true"
        );
    }
    assert!(checked > 0, "no corpus skills were actually checked");
}

/// Build a full SKILL.md source with the given Markdown `body`.
fn skill_source(body: &str) -> String {
    format!(
        "---\nname: deep\ndescription: Use when testing deeply nested markdown to check parser robustness.\n---\n\n{body}"
    )
}

/// Minimized repro for the `algorithmic-art` corpus entry on
/// [`KNOWN_NON_IDEMPOTENT`]: a tight list item whose prose contains a
/// mid-sentence `-` reflows so the continuation line starts with `- `,
/// which would otherwise re-parse as a nested list item rather than
/// continuation text; `wrap_tokens` now guards against this.
/// Formats `body` (wrapped in a skill) twice and asserts the second pass is a
/// no-op, i.e. `format(format(x)) == format(x)` at `FmtConfig::default()`.
fn assert_body_idempotent(body: &str) {
    let cfg = FmtConfig::default();
    let formatted = format_str(&skill_source(body), &cfg).expect("should format");
    let formatted_twice = format_str(&formatted, &cfg).expect("should format twice");
    assert_eq!(
        formatted, formatted_twice,
        "reflow changed meaning on the second pass"
    );
}

#[test]
fn wrapped_line_starting_with_dash_is_not_reparsed_as_nested_list() {
    assert_body_idempotent(
        "- **PARAMETRIC EXPRESSION**: Ideas communicate through mathematical relationships, forces, behaviors - not static composition\n- **OTHER**: filler\n",
    );
}

/// Minimized repro for the `claude-api` corpus entry on
/// [`KNOWN_NON_IDEMPOTENT`]: a blockquote paragraph containing a mid-sentence
/// `+` reflows so the continuation line starts with `+ `, which would
/// otherwise re-parse as a list item inside the blockquote rather than
/// continuation text; `wrap_tokens` now guards against this.
#[test]
fn wrapped_line_starting_with_plus_is_not_reparsed_as_list_in_blockquote() {
    assert_body_idempotent(
        "> **Note:** Managed Agents is the right choice when you want Anthropic to run the agent loop *and* host the container where tools execute — file ops, bash, code execution all run in the per-session workspace. If you want to host the compute yourself or run your own custom tool runtime, Claude API + tool use is the right choice — use the tool runner for the agentic loop — its per-turn hooks still give you approval gates, logging, error interception, and conditional execution (see `shared/tool-use-concepts.md`) — or the manual loop when you want to own the entire loop yourself.\n",
    );
}

/// Formats `body` (wrapped in a skill) once, returning just the formatted
/// full source (frontmatter included). Used by the `marker_like` tests below
/// to inspect the actual wrapped output, not just its idempotency.
fn format_body(body: &str) -> String {
    let cfg = FmtConfig::default();
    format_str(&skill_source(body), &cfg).expect("should format")
}

/// Only the Markdown body portion of a formatted `SKILL.md` (i.e. everything
/// after the closing `---` of the frontmatter), so marker checks below don't
/// false-positive on the frontmatter's own `---` delimiters.
fn body_only(formatted: &str) -> &str {
    let mut parts = formatted.splitn(3, "---\n");
    parts.next(); // before opening ---
    parts.next(); // frontmatter content, up to closing ---
    parts.next().unwrap_or("")
}

/// None of the lines in the body of `formatted` may be exactly `marker` —
/// that would mean a marker-shaped token was emitted alone at a line start,
/// which re-parses as block syntax (thematic break / setext underline /
/// blockquote / etc.) on the next pass.
fn assert_no_bare_marker_line(formatted: &str, marker: &str) {
    assert!(
        !body_only(formatted).lines().any(|l| l == marker),
        "formatted output has a bare `{marker}` line, which reparses as block syntax:\n{formatted}"
    );
}

/// Each of these bodies places a marker-shaped token (`---`/`===`/`-`/`***`)
/// right after a hard line break (`  \n`), which is exactly the case
/// `wrap_tokens` cannot avoid by gluing onto a previous line — the buffer is
/// empty because the hard break just flushed it — so it must hit the escape
/// fallback in `escape_line_start` instead. This deterministically forces
/// the marker to a line start regardless of `line_width`, unlike relying on
/// width-based wrapping to land exactly there.
#[test]
fn wrapped_line_starting_with_thematic_break_dashes_is_not_reparsed_as_hr() {
    let body = "some text before the break  \n--- and then some trailing prose after the marker\n";
    let formatted = format_body(body);
    assert_no_bare_marker_line(&formatted, "---");
    let formatted_twice =
        format_str(&formatted, &FmtConfig::default()).expect("should format twice");
    assert_eq!(
        formatted, formatted_twice,
        "reflow changed meaning on the second pass"
    );
}

#[test]
fn wrapped_line_starting_with_setext_h1_underline_is_not_reparsed_as_heading() {
    let body = "some text before the break  \n=== and then some trailing prose after the marker\n";
    let formatted = format_body(body);
    assert_no_bare_marker_line(&formatted, "===");
    let formatted_twice =
        format_str(&formatted, &FmtConfig::default()).expect("should format twice");
    assert_eq!(
        formatted, formatted_twice,
        "reflow changed meaning on the second pass"
    );
}

#[test]
fn wrapped_line_starting_with_lone_dash_is_not_reparsed_as_setext_or_hr() {
    let body = "some text before the break  \n- and then some trailing prose after the marker\n";
    let formatted = format_body(body);
    assert_no_bare_marker_line(&formatted, "-");
    let formatted_twice =
        format_str(&formatted, &FmtConfig::default()).expect("should format twice");
    assert_eq!(
        formatted, formatted_twice,
        "reflow changed meaning on the second pass"
    );
}

#[test]
fn wrapped_line_starting_with_thematic_break_stars_is_not_reparsed_as_hr() {
    let body = "some text before the break  \n*** and then some trailing prose after the marker\n";
    let formatted = format_body(body);
    assert_no_bare_marker_line(&formatted, "***");
    let formatted_twice =
        format_str(&formatted, &FmtConfig::default()).expect("should format twice");
    assert_eq!(
        formatted, formatted_twice,
        "reflow changed meaning on the second pass"
    );
}

/// Semantic-preservation check: the formatted body's CommonMark event stream
/// must not gain a `Rule` or `Heading` event that wasn't in the source —
/// i.e. the mid-paragraph `***` truly stays inline text and doesn't get
/// reinterpreted as a thematic break by the round trip.
#[test]
fn wrapped_thematic_break_token_does_not_introduce_rule_event() {
    let body = "some text before the break  \n*** and then some trailing prose after the marker\n";
    let source = skill_source(body);
    let formatted = format_str(&source, &FmtConfig::default()).expect("should format");

    // Both `source` and `formatted` include the `---` frontmatter fence,
    // which the raw CommonMark parser (not being frontmatter-aware) reads as
    // a thematic break in its own right — so compare *counts*, not just
    // presence, to isolate whether the mid-paragraph `***` specifically
    // introduced a new one.
    let source_rules = adept::markdown::parser(&source)
        .filter(|e| matches!(e, Event::Rule))
        .count();
    let formatted_rules = adept::markdown::parser(&formatted)
        .filter(|e| matches!(e, Event::Rule))
        .count();
    assert_eq!(
        source_rules, formatted_rules,
        "formatting changed the number of Rule events (mid-paragraph `***` was reinterpreted as a thematic break):\n{formatted}"
    );

    let source_has_heading =
        adept::markdown::parser(&source).any(|e| matches!(e, Event::Start(Tag::Heading { .. })));
    let formatted_has_heading =
        adept::markdown::parser(&formatted).any(|e| matches!(e, Event::Start(Tag::Heading { .. })));
    assert_eq!(
        source_has_heading, formatted_has_heading,
        "formatting changed whether the body contains a Heading event"
    );
}

#[test]
fn wrapped_line_starting_with_blockquote_marker_is_not_reparsed_as_nested_quote() {
    // The source escapes `>` (`\>`) so pulldown-cmark parses it as a literal
    // `>` text token rather than a nested blockquote start; the hard break
    // right before it then forces that literal `>` to be the very first
    // token `wrap_tokens` sees on a fresh line, exactly the case the escape
    // fallback in `escape_line_start` exists for.
    let body =
        "> some text before the break  \n> \\> and then some trailing prose after the marker\n";
    let formatted = format_body(body);
    // Inside a blockquote every physical line is prefixed with `> `, so a
    // bare `>` token that lands at a wrapped line start would itself read
    // back as `> >`, i.e. a nested blockquote — check for that shape too.
    let body_text = body_only(&formatted);
    assert!(
        !body_text.lines().any(|l| l == ">" || l == "> >"),
        "formatted output has a line that reparses as a nested blockquote:\n{formatted}"
    );
    let formatted_twice =
        format_str(&formatted, &FmtConfig::default()).expect("should format twice");
    assert_eq!(
        formatted, formatted_twice,
        "reflow changed meaning on the second pass"
    );
}

// --- Regression tests for the unbounded-recursion stack overflow (nested
// `Block::BlockQuote` / `Block::List` in `markdown::build`/`markdown::print`
// had no depth bound). These must not abort the process; a panic would also
// fail the test harness, so simply completing without crashing already
// demonstrates the fix, but we additionally assert on the `Result` shape.

#[test]
fn deeply_nested_blockquote_does_not_crash() {
    // 10,000 levels of nested blockquote is exactly the crashing repro from
    // the bug report; comfortably larger than any real document and far
    // beyond `MAX_NESTING_DEPTH`.
    let body = format!("{} hi\n", ">".repeat(10_000));
    let source = skill_source(&body);
    let cfg = FmtConfig::default();

    // The key assertion is simply that this call returns instead of
    // aborting the process with a stack overflow.
    let formatted = format_str(&source, &cfg).expect("depth-bombed blockquote should format");
    assert!(
        formatted.contains("hi"),
        "the deeply nested content should not be silently dropped"
    );

    let checked = adept_fmt::check_str(&source, &cfg).expect("check_str should also not crash");
    let _ = checked; // Ok(_) either way; not crashing is the assertion.
}

#[test]
fn deeply_nested_list_does_not_crash() {
    // A nested-list variant of the depth bomb: each level is one more
    // indented `- ` marker wrapping the next, terminating in a leaf item.
    let depth = 5_000;
    let mut body = String::new();
    for i in 0..depth {
        body.push_str(&"  ".repeat(i));
        body.push_str("- ");
    }
    body.push_str("leaf\n");
    let source = skill_source(&body);
    let cfg = FmtConfig::default();

    let formatted = format_str(&source, &cfg).expect("depth-bombed list should format");
    assert!(
        formatted.contains("leaf"),
        "the deeply nested content should not be silently dropped"
    );
}

#[test]
fn deeply_nested_but_within_limit_blockquote_is_correct_and_idempotent() {
    // Comfortably under `MAX_NESTING_DEPTH` (100): should still be parsed
    // into a fully structured, correctly indented nested-blockquote tree,
    // and remain idempotent.
    let depth = 40;
    let body = format!("{} hi\n", ">".repeat(depth));
    let source = skill_source(&body);
    let cfg = FmtConfig::default();

    let formatted = format_str(&source, &cfg).expect("within-limit blockquote should format");
    let expected_prefix = "> ".repeat(depth);
    assert!(
        formatted.contains(&format!("{expected_prefix}hi")),
        "expected a fully nested `> > > ... hi` line, got:\n{formatted}"
    );

    let formatted_twice = format_str(&formatted, &cfg).expect("re-formatting should also succeed");
    assert_eq!(
        formatted, formatted_twice,
        "within-limit deeply nested blockquote should be idempotent"
    );
}

#[test]
fn deeply_nested_but_within_limit_list_is_correct_and_idempotent() {
    let depth = 40;
    let mut body = String::new();
    for i in 0..depth {
        body.push_str(&"  ".repeat(i));
        body.push_str("- ");
    }
    body.push_str("leaf\n");
    let source = skill_source(&body);
    let cfg = FmtConfig::default();

    let formatted = format_str(&source, &cfg).expect("within-limit nested list should format");
    assert!(
        formatted.contains("leaf"),
        "leaf content should survive formatting"
    );

    let formatted_twice = format_str(&formatted, &cfg).expect("re-formatting should also succeed");
    assert_eq!(
        formatted, formatted_twice,
        "within-limit deeply nested list should be idempotent"
    );
}
