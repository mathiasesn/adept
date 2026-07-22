//! Positioned queries over a Markdown body.
//!
//! These are the linter's view of a document: flat lists of the constructs
//! the `SL1xx` rules care about, each carrying the line it starts on. They
//! share [`super::parser`] with the formatter's [`super::parse_document`],
//! so both agree on what a heading, a link, or a code block is.

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

/// All headings in `src`, in document order.
///
/// Both ATX (`# Title`) and setext (`Title` / `=====`) headings are
/// reported; [`Heading::is_setext`] distinguishes them. Heading-like text
/// inside fenced or indented code blocks is not a heading and is not
/// reported.
pub fn headings(src: &str) -> Vec<Located<Heading>> {
    let index = LineIndex::new(src);
    let mut out = Vec::new();
    // The heading currently being accumulated: level, start offset, text.
    let mut open: Option<(u8, usize, String)> = None;
    for (event, range) in super::parser(src).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                open = Some((level as u8, range.start, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start, text)) = open.take() {
                    out.push(Located {
                        value: Heading {
                            level,
                            text: text.trim().to_string(),
                            is_setext: !starts_with_hash(&src[start..range.end.min(src.len())]),
                        },
                        line: index.line(start),
                    });
                }
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, _, text)) = open.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, text)) = open.as_mut() {
                    text.push(' ');
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether a heading's source starts (after any indentation) with `#`,
/// i.e. is an ATX heading. `pulldown-cmark` does not expose the ATX/setext
/// distinction, so it is derived from the heading's source range.
fn starts_with_hash(heading_src: &str) -> bool {
    heading_src
        .bytes()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| b == b'#')
}

/// The destinations of every link and image in `src`, in document order.
///
/// Destinations inside fenced or indented code blocks are not reported:
/// `pulldown-cmark` emits code content as text, never as a link. Nested
/// parentheses in a destination are handled by the parser.
pub fn link_destinations(src: &str) -> Vec<Located<String>> {
    let index = LineIndex::new(src);
    super::parser(src)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            let dest = match event {
                Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => dest_url,
                _ => return None,
            };
            Some(Located {
                value: dest.to_string(),
                line: index.line(range.start),
            })
        })
        .collect()
}

/// The content of every inline code span (`` `like this` ``) in `src`, in
/// document order. Fenced and indented code *blocks* are not inline code
/// spans and are not reported.
pub fn inline_code_spans(src: &str) -> Vec<Located<String>> {
    let index = LineIndex::new(src);
    super::parser(src)
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Code(code) => Some(Located {
                value: code.to_string(),
                line: index.line(range.start),
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
            .map(|h| (h.value.level, h.value.text.as_str(), h.value.is_setext, h.line))
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
