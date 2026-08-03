//! Building [`Block`]/[`Inline`] trees from a `pulldown-cmark` event stream.

use std::iter::Peekable;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};

use super::ast::{Alignment, Block, Inline, ListItem};
use super::MAX_NESTING_DEPTH;

/// Parse a Markdown document body into a sequence of top-level [`Block`]s.
///
/// This is the span-free view of a document, used by the formatter. See
/// [`super::parser`] for the shared parser construction, and
/// [`super::headings`] and friends for the positioned view.
pub fn parse_document(source: &str) -> Vec<Block> {
    let mut iter = super::parser(source).peekable();
    collect_blocks(&mut iter, 0)
}

type EventIter<'a> = Peekable<Parser<'a>>;

/// Consume events until the matching `Event::End` for a container whose
/// `Start` has already been consumed (`depth` starts at 1 for that
/// container), flattening all text-bearing events encountered into a
/// single string. Used once nesting exceeds [`MAX_NESTING_DEPTH`], as a
/// non-recursive fallback that can never overflow the stack regardless of
/// how deeply the input is (mis)nested.
fn collect_raw_until_end(iter: &mut EventIter<'_>) -> String {
    let mut depth = 1usize;
    let mut text = String::new();
    while depth > 0 {
        match iter.next() {
            None => break,
            Some(Event::Start(_)) => depth += 1,
            Some(Event::End(_)) => depth -= 1,
            Some(
                Event::Text(t)
                | Event::Code(t)
                | Event::InlineHtml(t)
                | Event::Html(t)
                | Event::InlineMath(t)
                | Event::DisplayMath(t),
            ) => {
                text.push_str(&t);
            }
            Some(Event::SoftBreak | Event::HardBreak) => text.push(' '),
            Some(Event::FootnoteReference(label)) => {
                text.push_str(&format!("[^{label}]"));
            }
            Some(Event::Rule | Event::TaskListMarker(_)) => {}
        }
    }
    text
}

fn collect_blocks(iter: &mut EventIter<'_>, depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    loop {
        match iter.peek() {
            None | Some(Event::End(_)) => break,
            Some(Event::Start(tag)) if is_inline_tag(tag) => {
                // Tight list items (and similar contexts) surface their
                // content as bare inline events, with no wrapping
                // `Tag::Paragraph`. Treat a run of such inline content as an
                // implicit paragraph.
                let inline = collect_inlines(iter);
                out.push(Block::Paragraph(inline));
            }
            Some(Event::Start(_)) => {
                let Some(Event::Start(tag)) = iter.next() else {
                    unreachable!()
                };
                if let Some(block) = build_block(tag, iter, depth) {
                    out.push(block);
                }
            }
            Some(Event::Rule) => {
                iter.next();
                out.push(Block::ThematicBreak);
            }
            Some(Event::Html(_)) => {
                let mut text = String::new();
                while let Some(Event::Html(s)) = iter.peek() {
                    text.push_str(s);
                    iter.next();
                }
                out.push(Block::HtmlBlock(text.trim_end().to_string()));
            }
            Some(
                Event::Text(_)
                | Event::Code(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_),
            ) => {
                // Same bare-inline-content case as above, for text/code/etc.
                // that isn't wrapped in an inline start tag.
                let inline = collect_inlines(iter);
                out.push(Block::Paragraph(inline));
            }
            // Stray inline-only events at block level (shouldn't normally
            // occur, but be defensive rather than panicking on odd input).
            Some(_) => {
                iter.next();
            }
        }
    }
    out
}

fn is_inline_tag(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }
    )
}

/// Collect a list item's blocks, additionally reporting whether the item's
/// own content is "tight" (see [`adept::markdown::ast::ListItem::content_tight`])
/// so the printer can preserve tight-list formatting instead of always
/// inserting blank lines.
///
/// `pulldown-cmark` signals a whole list's tightness uniformly across all
/// its items: either every item's leading paragraph is emitted as bare
/// inline events (list is tight, no blank line anywhere in it), or every
/// item's leading paragraph is wrapped in `Tag::Paragraph` (list is loose,
/// some item somewhere is blank-separated from a sibling or internally).
/// So `leading_bare_inline == true` already means "no blank line exists
/// anywhere in this list" — the only thing left to get right here is not
/// to manufacture a false "not tight" for an item that has more than one
/// block but whose extra blocks are just nested lists, which never
/// introduce a blank line of their own between this item's blocks.
fn collect_item_blocks(iter: &mut EventIter<'_>, depth: usize) -> (Vec<Block>, bool) {
    let mut blocks = Vec::new();
    let mut leading_bare_inline = false;
    match iter.peek() {
        Some(Event::Start(tag)) if is_inline_tag(tag) => {
            let inline = collect_inlines(iter);
            blocks.push(Block::Paragraph(inline));
            leading_bare_inline = true;
        }
        Some(
            Event::Text(_)
            | Event::Code(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_),
        ) => {
            let inline = collect_inlines(iter);
            blocks.push(Block::Paragraph(inline));
            leading_bare_inline = true;
        }
        _ => {}
    }
    blocks.extend(collect_blocks(iter, depth));
    let extra_blocks_are_lists = blocks
        .get(1..)
        .is_some_and(|rest| rest.iter().all(|b| matches!(b, Block::List { .. })));
    let tight = leading_bare_inline && (blocks.len() == 1 || extra_blocks_are_lists);
    (blocks, tight)
}

fn expect_end(iter: &mut EventIter<'_>) {
    if let Some(Event::End(_)) = iter.peek() {
        iter.next();
    }
}

fn build_block(tag: Tag<'_>, iter: &mut EventIter<'_>, depth: usize) -> Option<Block> {
    match tag {
        Tag::Paragraph => {
            let inline = collect_inlines(iter);
            expect_end(iter);
            Some(Block::Paragraph(inline))
        }
        Tag::Heading { level, .. } => {
            let inline = collect_inlines(iter);
            expect_end(iter);
            Some(Block::Heading {
                level: level as u8,
                inline,
            })
        }
        Tag::BlockQuote(_) => {
            if depth >= MAX_NESTING_DEPTH {
                return Some(Block::Raw(collect_raw_until_end(iter)));
            }
            let blocks = collect_blocks(iter, depth + 1);
            expect_end(iter);
            Some(Block::BlockQuote(blocks))
        }
        Tag::CodeBlock(kind) => {
            let mut literal = String::new();
            while let Some(Event::Text(t)) = iter.peek() {
                literal.push_str(t);
                iter.next();
            }
            expect_end(iter);
            let info = match kind {
                CodeBlockKind::Fenced(info) => info.to_string(),
                CodeBlockKind::Indented => String::new(),
            };
            // Strip a single trailing newline; the printer re-adds line
            // structure.
            let literal = literal.strip_suffix('\n').unwrap_or(&literal).to_string();
            Some(Block::CodeBlock { info, literal })
        }
        Tag::List(start) => {
            if depth >= MAX_NESTING_DEPTH {
                return Some(Block::Raw(collect_raw_until_end(iter)));
            }
            let ordered = start.is_some();
            let start_num = start.unwrap_or(1);
            let mut items = Vec::new();
            while let Some(Event::Start(Tag::Item)) = iter.peek() {
                iter.next();
                let checked = if let Some(Event::TaskListMarker(checked)) = iter.peek() {
                    let checked = *checked;
                    iter.next();
                    Some(checked)
                } else {
                    None
                };
                let (blocks, content_tight) = collect_item_blocks(iter, depth + 1);
                expect_end(iter);
                items.push(ListItem {
                    checked,
                    content_tight,
                    blocks,
                });
            }
            expect_end(iter);
            let tight = items.iter().all(|item| item.content_tight);
            Some(Block::List {
                ordered,
                start: start_num,
                tight,
                items,
            })
        }
        Tag::Table(aligns) => {
            let alignments = aligns.into_iter().map(Alignment::from).collect();
            let mut header = Vec::new();
            let mut rows = Vec::new();
            if let Some(Event::Start(Tag::TableHead)) = iter.peek() {
                iter.next();
                header = collect_table_row(iter);
                expect_end(iter);
            }
            while let Some(Event::Start(Tag::TableRow)) = iter.peek() {
                iter.next();
                rows.push(collect_table_row(iter));
                expect_end(iter);
            }
            expect_end(iter);
            Some(Block::Table {
                alignments,
                header,
                rows,
            })
        }
        Tag::FootnoteDefinition(label) => {
            if depth >= MAX_NESTING_DEPTH {
                return Some(Block::Raw(collect_raw_until_end(iter)));
            }
            let blocks = collect_blocks(iter, depth + 1);
            expect_end(iter);
            Some(Block::FootnoteDefinition {
                label: label.to_string(),
                blocks,
            })
        }
        Tag::HtmlBlock => {
            let mut text = String::new();
            while let Some(Event::Html(s)) = iter.peek() {
                text.push_str(s);
                iter.next();
            }
            expect_end(iter);
            Some(Block::HtmlBlock(text.trim_end().to_string()))
        }
        // Unsupported/unreachable-at-this-level containers: skip their
        // content but don't fail the whole document.
        _ => {
            if depth >= MAX_NESTING_DEPTH {
                let _ = collect_raw_until_end(iter);
            } else {
                let _ = collect_blocks(iter, depth + 1);
                expect_end(iter);
            }
            None
        }
    }
}

fn collect_table_row(iter: &mut EventIter<'_>) -> Vec<Vec<Inline>> {
    let mut cells = Vec::new();
    while let Some(Event::Start(Tag::TableCell)) = iter.peek() {
        iter.next();
        let inline = collect_inlines(iter);
        expect_end(iter);
        cells.push(inline);
    }
    cells
}

fn collect_inlines(iter: &mut EventIter<'_>) -> Vec<Inline> {
    let mut out = Vec::new();
    loop {
        match iter.peek() {
            None | Some(Event::End(_)) => break,
            Some(Event::Text(_)) => {
                let Some(Event::Text(t)) = iter.next() else {
                    unreachable!()
                };
                out.push(Inline::Text(t.to_string()));
            }
            Some(Event::Code(_)) => {
                let Some(Event::Code(t)) = iter.next() else {
                    unreachable!()
                };
                out.push(Inline::Code(t.to_string()));
            }
            Some(Event::SoftBreak) => {
                iter.next();
                out.push(Inline::SoftBreak);
            }
            Some(Event::HardBreak) => {
                iter.next();
                out.push(Inline::HardBreak);
            }
            Some(Event::InlineHtml(_)) => {
                let Some(Event::InlineHtml(t)) = iter.next() else {
                    unreachable!()
                };
                out.push(Inline::Html(t.to_string()));
            }
            Some(Event::FootnoteReference(_)) => {
                let Some(Event::FootnoteReference(t)) = iter.next() else {
                    unreachable!()
                };
                out.push(Inline::FootnoteReference(t.to_string()));
            }
            Some(Event::Start(tag)) if !is_inline_tag(tag) => {
                // A block-level tag (e.g. a nested list following bare text
                // in a tight list item) starting mid-inline-stream: leave
                // it for the caller (`collect_blocks`) to handle as the
                // next block, rather than consuming it here.
                break;
            }
            Some(Event::Start(_)) => {
                let Some(Event::Start(tag)) = iter.next() else {
                    unreachable!()
                };
                match tag {
                    Tag::Emphasis => {
                        let children = collect_inlines(iter);
                        expect_end(iter);
                        out.push(Inline::Emphasis(children));
                    }
                    Tag::Strong => {
                        let children = collect_inlines(iter);
                        expect_end(iter);
                        out.push(Inline::Strong(children));
                    }
                    Tag::Strikethrough => {
                        let children = collect_inlines(iter);
                        expect_end(iter);
                        out.push(Inline::Strikethrough(children));
                    }
                    Tag::Link {
                        dest_url, title, ..
                    } => {
                        let children = collect_inlines(iter);
                        expect_end(iter);
                        let title = title.to_string();
                        out.push(Inline::Link {
                            dest: dest_url.to_string(),
                            title: if title.is_empty() { None } else { Some(title) },
                            children,
                        });
                    }
                    Tag::Image {
                        dest_url, title, ..
                    } => {
                        // Alt text is the flattened text content of the
                        // image's children.
                        let children = collect_inlines(iter);
                        expect_end(iter);
                        let alt = flatten_text(&children);
                        let title = title.to_string();
                        out.push(Inline::Image {
                            dest: dest_url.to_string(),
                            title: if title.is_empty() { None } else { Some(title) },
                            alt,
                        });
                    }
                    other => {
                        // Unexpected nesting; skip its content gracefully.
                        let _ = collect_inlines(iter);
                        expect_end(iter);
                        let _ = other;
                    }
                }
            }
            Some(_) => {
                // TaskListMarker, Rule, math, etc. at inline level: ignore.
                iter.next();
            }
        }
    }
    out
}

fn flatten_text(inline: &[Inline]) -> String {
    let mut out = String::new();
    for i in inline {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Html(t) => out.push_str(t),
            Inline::Emphasis(c) | Inline::Strong(c) | Inline::Strikethrough(c) => {
                out.push_str(&flatten_text(c));
            }
            Inline::Link { children, .. } => out.push_str(&flatten_text(children)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            Inline::FootnoteReference(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_of(source: &str) -> (bool, Vec<bool>) {
        let blocks = parse_document(source);
        match blocks.into_iter().next() {
            Some(Block::List { tight, items, .. }) => {
                (tight, items.iter().map(|i| i.content_tight).collect())
            }
            other => panic!("expected a top-level list, got {other:?}"),
        }
    }

    #[test]
    fn item_with_text_plus_nested_list_and_no_blank_line_stays_tight() {
        // A tight item mixing bare-inline text with a nested list (no blank
        // line separating them) must not be downgraded to loose, and must
        // not loosen its siblings either.
        let source = "- one\n  - nested\n- two\n";
        let (tight, item_tight) = list_of(source);
        assert!(tight, "list should stay tight");
        assert_eq!(item_tight, vec![true, true]);
    }

    #[test]
    fn item_with_two_blocks_separated_by_blank_line_makes_list_loose() {
        // Genuinely loose source (a blank line between two blocks inside one
        // item) is a whole-list property per CommonMark: pulldown-cmark
        // itself reports every item's leading paragraph as wrapped once any
        // item is loose, so the whole list (and every item.content_tight)
        // must report loose here — this is not the bug being fixed.
        let source = "- one\n\n  more\n- two\n";
        let (tight, item_tight) = list_of(source);
        assert!(!tight, "list should become loose");
        assert!(item_tight.iter().all(|t| !t));
    }

    #[test]
    fn plain_tight_list_is_unaffected() {
        let source = "- one\n- two\n- three\n";
        let (tight, item_tight) = list_of(source);
        assert!(tight);
        assert_eq!(item_tight, vec![true, true, true]);
    }
}
