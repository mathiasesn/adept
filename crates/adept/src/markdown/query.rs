//! Positioned queries over a Markdown body.
//!
//! These are the linter's view of a document: flat lists of the constructs
//! the `SL1xx` rules care about, each carrying the line it starts on. They
//! share [`super::parser`] with the formatter's [`super::parse_document`],
//! so both agree on what a heading, a link, or a code block is.

use std::ops::Range;

use pulldown_cmark::{Event, Tag, TagEnd};

/// A value together with the line it starts on.
///
/// `line` is **1-based** and relative to the string passed to the query, so
/// callers working on a skill body must add `skill.body_line_offset`
/// themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located<T> {
    /// The located value.
    pub value: T,
    /// The 1-based line, relative to the queried string.
    pub line: usize,
}

/// A Markdown heading, as seen by the shared parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// Heading level, 1-6.
    pub level: u8,
    /// The heading's text content, with inline markup flattened away.
    pub text: String,
    /// Whether the heading was written in setext form (`Title` followed by
    /// `===`/`---`) rather than ATX form (`# Title`).
    pub is_setext: bool,
}

/// Byte offset → 1-based line number, for one document.
struct LineIndex {
    /// Byte offset of the start of each line, ascending; always starts at 0.
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(src: &str) -> Self {
        let mut starts = vec![0usize];
        starts.extend(
            src.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Self { starts }
    }

    /// The 1-based line containing `offset`. Byte offsets from
    /// `into_offset_iter` are always char boundaries, but this is correct
    /// for any offset since it only compares against line starts.
    fn line(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
}

/// Every event in `src`, paired with its source range and the 1-based line
/// its range starts on.
///
/// This is the one place that combines [`super::parser`] with a
/// [`LineIndex`], so each public query below is just a filter over it and
/// none of them can get the offset-to-line convention wrong on its own.
fn located_events(src: &str) -> impl Iterator<Item = (Event<'_>, Range<usize>, usize)> {
    let index = LineIndex::new(src);
    super::parser(src)
        .into_offset_iter()
        .map(move |(event, range)| {
            let line = index.line(range.start);
            (event, range, line)
        })
}

/// All headings in `src`, in document order.
///
/// Both ATX (`# Title`) and setext (`Title` / `=====`) headings are
/// reported; [`Heading::is_setext`] distinguishes them. Heading-like text
/// inside fenced or indented code blocks is not a heading and is not
/// reported.
pub fn headings(src: &str) -> Vec<Located<Heading>> {
    let mut out = Vec::new();
    // The heading currently being accumulated: level, start offset, start
    // line, and text.
    let mut open: Option<(u8, usize, usize, String)> = None;
    for (event, range, line) in located_events(src) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                open = Some((level as u8, range.start, line, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start, start_line, text)) = open.take() {
                    out.push(Located {
                        value: Heading {
                            level,
                            text: text.trim().to_string(),
                            is_setext: is_setext_source(&src[start..range.end.min(src.len())]),
                        },
                        line: start_line,
                    });
                }
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((.., text)) = open.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((.., text)) = open.as_mut() {
                    text.push(' ');
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether a heading's source is setext form. `pulldown-cmark` does not
/// expose the ATX/setext distinction, so it is derived from the heading's
/// source range.
///
/// The discriminator is the **end** of the range, not its start: an ATX
/// heading occupies exactly one line, whereas a setext heading spans at
/// least two and its last line is an underline of only `=` or only `-`.
/// Looking at the start instead would misread `#hashtag` (not an ATX
/// heading — CommonMark requires a space after the `#` run) underlined by
/// `====` as ATX.
///
/// Inside a container, `into_offset_iter` includes the container's marker
/// on continuation lines (`> =====`, `  =====`), so leading whitespace and
/// `>` markers are stripped before the underline is checked.
fn is_setext_source(heading_src: &str) -> bool {
    let trimmed = heading_src.trim_end();
    let Some((_, last)) = trimmed.rsplit_once('\n') else {
        // A single line: necessarily ATX.
        return false;
    };
    let underline = last.trim_matches(|c: char| c.is_whitespace() || c == '>');
    !underline.is_empty()
        && (underline.bytes().all(|b| b == b'=') || underline.bytes().all(|b| b == b'-'))
}

/// The destinations of every link and image in `src`, in document order.
///
/// Destinations inside fenced or indented code blocks are not reported:
/// `pulldown-cmark` emits code content as text, never as a link. Nested
/// parentheses in a destination are handled by the parser.
pub fn link_destinations(src: &str) -> Vec<Located<String>> {
    located_events(src)
        .filter_map(|(event, _, line)| {
            let dest = match event {
                Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => dest_url,
                _ => return None,
            };
            Some(Located {
                value: dest.to_string(),
                line,
            })
        })
        .collect()
}

/// The content of every inline code span (`` `like this` ``) in `src`, in
/// document order. Fenced and indented code *blocks* are not inline code
/// spans and are not reported.
pub fn inline_code_spans(src: &str) -> Vec<Located<String>> {
    located_events(src)
        .filter_map(|(event, _, line)| match event {
            Event::Code(code) => Some(Located {
                value: code.to_string(),
                line,
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_is_one_based_and_utf8_safe() {
        let src = "a\nbb\n\nccc";
        let index = LineIndex::new(src);
        assert_eq!(index.line(0), 1);
        assert_eq!(index.line(1), 1);
        assert_eq!(index.line(2), 2);
        assert_eq!(index.line(5), 3);
        assert_eq!(index.line(6), 4);
    }

    #[test]
    fn multibyte_char_before_heading_does_not_shift_line() {
        // "café — naïve" is 12 chars but 16 bytes; the heading must still
        // be reported on line 3, not on a byte-derived line.
        let src = "café — naïve\n\n# Título\n";
        let found = headings(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].value.text, "Título");
    }

    #[test]
    fn detects_atx_and_setext_headings() {
        let src = "Setext One\n==========\n\n## Atx Two\n\nSetext Two\n----------\n";
        let found = headings(src);
        let summary: Vec<_> = found
            .iter()
            .map(|h| {
                (
                    h.value.level,
                    h.value.text.as_str(),
                    h.value.is_setext,
                    h.line,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (1, "Setext One", true, 1),
                (2, "Atx Two", false, 4),
                (2, "Setext Two", true, 6),
            ]
        );
    }

    #[test]
    fn indented_atx_heading_is_still_atx() {
        // Up to three spaces of indentation still makes an ATX heading.
        let src = "   # Indented\n";
        let found = headings(src);
        assert_eq!(found.len(), 1);
        assert!(!found[0].value.is_setext);
    }

    #[test]
    fn hash_prefixed_text_underlined_is_setext_not_atx() {
        // `#hashtag` is not an ATX heading (CommonMark requires a space
        // after the `#` run), so this is a setext h1 whose text merely
        // starts with `#`.
        let src = "#hashtag start of setext\n========================\n";
        let found = headings(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value.level, 1);
        assert_eq!(found[0].value.text, "#hashtag start of setext");
        assert!(found[0].value.is_setext);
    }

    #[test]
    fn dash_underlined_heading_is_a_setext_h2() {
        let src = "Setext H2\n---------\n";
        let found = headings(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value.level, 2);
        assert!(found[0].value.is_setext);
    }

    #[test]
    fn escaped_hash_underlined_is_setext() {
        let src = "\\# Escaped\n=====\n";
        let found = headings(src);
        assert_eq!(found.len(), 1);
        assert!(found[0].value.is_setext);
    }

    #[test]
    fn setext_headings_inside_containers_are_setext() {
        // `into_offset_iter` includes the container marker on the
        // underline line, which the discriminator must tolerate.
        for src in [
            "> Quoted Setext\n> =============\n",
            "- List item setext\n  =================\n",
        ] {
            let found = headings(src);
            assert_eq!(found.len(), 1, "{src:?}");
            assert!(found[0].value.is_setext, "{src:?}");
        }
    }

    #[test]
    fn heading_text_flattens_inline_markup() {
        let src = "# A **bold** `code` [link](x.md)\n";
        let found = headings(src);
        assert_eq!(found[0].value.text, "A bold code link");
    }

    #[test]
    fn link_destination_with_nested_parentheses() {
        let src = "See [it](docs/file_(v2).md) here.\n";
        let found = link_destinations(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "docs/file_(v2).md");
        assert_eq!(found[0].line, 1);
    }

    #[test]
    fn image_destinations_are_included() {
        let src = "Text\n\n![alt](img/logo.png)\n";
        let found = link_destinations(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "img/logo.png");
        assert_eq!(found[0].line, 3);
    }

    #[test]
    fn links_inside_fenced_code_blocks_are_not_returned() {
        let src = "```md\n[a](inside.md)\n```\n\n[b](outside.md)\n";
        let found = link_destinations(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "outside.md");
        assert_eq!(found[0].line, 5);
    }

    #[test]
    fn links_inside_indented_code_blocks_are_not_returned() {
        let src = "Intro\n\n    [a](inside.md)\n\n[b](outside.md)\n";
        let found = link_destinations(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "outside.md");
    }

    #[test]
    fn headings_inside_code_blocks_are_not_returned() {
        let src = "```sh\n# not a heading\n```\n\n    # nor this\n\n# real\n";
        let found = headings(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value.text, "real");
    }

    #[test]
    fn inline_code_spans_are_located() {
        let src = "Use `foo.md` here.\n\n```\n`not inline`\n```\n\nAnd `bar.py`.\n";
        let found = inline_code_spans(src);
        let summary: Vec<_> = found.iter().map(|c| (c.value.as_str(), c.line)).collect();
        assert_eq!(summary, vec![("foo.md", 1), ("bar.py", 7)]);
    }
}
