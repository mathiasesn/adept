//! Pretty-printing a Markdown AST back to normalized CommonMark text.

use crate::config::FmtConfig;

use adept::markdown::ast::{Alignment, Block, Inline, ListItem};
use adept::markdown::MAX_NESTING_DEPTH;

/// A single reflow-able output token: either an atomic word (which may
/// itself be a whole inline code span, link, or image — never split
/// internally) or a forced hard line break.
enum Token {
    Word(String),
    Break,
}

/// Print a full sequence of top-level blocks to a document string, ending
/// in exactly one trailing newline.
pub fn print_document(blocks: &[Block], cfg: &FmtConfig) -> String {
    let lines = print_blocks(blocks, cfg, 0);
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn print_blocks(blocks: &[Block], cfg: &FmtConfig, depth: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(print_block(b, cfg, depth));
    }
    lines
}

/// Print a single block. `depth` counts container nesting (block quotes,
/// lists, footnote definitions) seen so far; once it reaches
/// [`MAX_NESTING_DEPTH`] we stop recursing into further containers
/// regardless of what the AST actually contains, so this function's own
/// call stack can never exceed that bound even given a pathologically
/// deep, hand-built [`Block`] tree (the normal `build` path never produces
/// one this deep in the first place, see [`super::build`]).
fn print_block(block: &Block, cfg: &FmtConfig, depth: usize) -> Vec<String> {
    match block {
        Block::Heading { level, inline } => {
            let level = (*level).clamp(1, 6);
            let text = flatten_words(inline, cfg);
            let hashes = "#".repeat(level as usize);
            if text.is_empty() {
                vec![hashes]
            } else {
                vec![format!("{hashes} {text}")]
            }
        }
        Block::Paragraph(inline) => wrap_paragraph(inline, cfg),
        Block::BlockQuote(inner) => {
            if depth >= MAX_NESTING_DEPTH {
                return vec!["> [nesting too deep, content omitted]".to_string()];
            }
            let content = print_blocks(inner, cfg, depth + 1);
            indent_block(&content, "> ", "> ")
        }
        Block::List {
            ordered,
            start,
            tight,
            items,
        } => {
            if depth >= MAX_NESTING_DEPTH {
                return vec!["- [nesting too deep, content omitted]".to_string()];
            }
            print_list(*ordered, *start, *tight, items, cfg, depth)
        }
        Block::CodeBlock { info, literal } => print_code_block(info, literal, cfg),
        Block::ThematicBreak => vec!["---".to_string()],
        Block::Table {
            alignments,
            header,
            rows,
        } => print_table(alignments, header, rows, cfg),
        Block::HtmlBlock(raw) => raw.lines().map(str::to_string).collect(),
        Block::Raw(raw) => raw.lines().map(str::to_string).collect(),
        Block::FootnoteDefinition { label, blocks } => {
            if depth >= MAX_NESTING_DEPTH {
                return vec![format!("[^{label}]: [nesting too deep, content omitted]")];
            }
            let content = print_blocks(blocks, cfg, depth + 1);
            let first_prefix = format!("[^{label}]: ");
            let rest_prefix = " ".repeat(first_prefix.chars().count());
            indent_block(&content, &first_prefix, &rest_prefix)
        }
    }
}

fn print_list(
    ordered: bool,
    start: u64,
    tight: bool,
    items: &[ListItem],
    cfg: &FmtConfig,
    depth: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    for (i, (num, item)) in (start..).zip(items.iter()).enumerate() {
        if i > 0 && !tight {
            out.push(String::new());
        }
        let first_prefix = if ordered {
            format!("{num}. ")
        } else {
            format!("{} ", cfg.bullet_marker.as_char())
        };
        let rest_prefix = " ".repeat(first_prefix.chars().count());

        let mut content = print_blocks(&item.blocks, cfg, depth + 1);
        if let Some(checked) = item.checked {
            let box_str = if checked { "[x] " } else { "[ ] " };
            if content.is_empty() {
                content.push(box_str.trim_end().to_string());
            } else {
                content[0] = format!("{box_str}{}", content[0]);
            }
        }
        out.extend(indent_block(&content, &first_prefix, &rest_prefix));
    }
    out
}

fn print_code_block(info: &str, literal: &str, cfg: &FmtConfig) -> Vec<String> {
    let longest_run = longest_run_of(literal, cfg.fence_char.as_char());
    let fence_len = (longest_run + 1).max(3);
    let fence: String = cfg.fence_char.as_char().to_string().repeat(fence_len);
    let mut out = Vec::new();
    out.push(format!("{fence}{info}"));
    if !literal.is_empty() {
        out.extend(literal.split('\n').map(str::to_string));
    }
    out.push(fence);
    out
}

fn longest_run_of(s: &str, c: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in s.chars() {
        if ch == c {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn print_table(
    alignments: &[Alignment],
    header: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    cfg: &FmtConfig,
) -> Vec<String> {
    let col_count = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0))
        .max(alignments.len());
    if col_count == 0 {
        return Vec::new();
    }

    let render_row = |cells: &[Vec<Inline>]| -> Vec<String> {
        (0..col_count)
            .map(|i| {
                cells
                    .get(i)
                    .map(|c| flatten_words(c, cfg))
                    .unwrap_or_default()
            })
            .collect()
    };

    let header_r = render_row(header);
    let rows_r: Vec<Vec<String>> = rows.iter().map(|r| render_row(r)).collect();

    let mut widths = vec![3usize; col_count];
    for (i, cell) in header_r.iter().enumerate() {
        widths[i] = widths[i].max(cell.chars().count());
    }
    for row in &rows_r {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let align_at =
        |i: usize| -> Alignment { alignments.get(i).copied().unwrap_or(Alignment::None) };

    let pad_cell = |s: &str, width: usize, align: Alignment| -> String {
        let len = s.chars().count();
        let pad = width.saturating_sub(len);
        match align {
            Alignment::Right => format!("{}{}", " ".repeat(pad), s),
            Alignment::Center => {
                let left = pad / 2;
                let right = pad - left;
                format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
            }
            Alignment::None | Alignment::Left => format!("{}{}", s, " ".repeat(pad)),
        }
    };

    let render_line = |cells: &[String]| -> String {
        let padded: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| pad_cell(c, widths[i], align_at(i)))
            .collect();
        format!("| {} |", padded.join(" | "))
    };

    let sep_cells: Vec<String> = (0..col_count)
        .map(|i| {
            let w = widths[i];
            match align_at(i) {
                Alignment::None => "-".repeat(w),
                Alignment::Left => format!(":{}", "-".repeat(w.saturating_sub(1))),
                Alignment::Right => format!("{}:", "-".repeat(w.saturating_sub(1))),
                Alignment::Center => format!(":{}:", "-".repeat(w.saturating_sub(2))),
            }
        })
        .collect();

    let mut out = Vec::new();
    out.push(render_line(&header_r));
    out.push(format!("| {} |", sep_cells.join(" | ")));
    for row in &rows_r {
        out.push(render_line(row));
    }
    out
}

fn indent_block(lines: &[String], first: &str, rest: &str) -> Vec<String> {
    if lines.is_empty() {
        return vec![first.trim_end().to_string()];
    }
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let prefix = if i == 0 { first } else { rest };
            if l.is_empty() {
                prefix.trim_end().to_string()
            } else {
                format!("{prefix}{l}")
            }
        })
        .collect()
}

fn wrap_paragraph(inline: &[Inline], cfg: &FmtConfig) -> Vec<String> {
    let tokens = build_tokens(inline, cfg);
    wrap_tokens(&tokens, cfg.line_width, cfg.reflow_prose)
}

/// True when `word` would be interpreted as CommonMark block-starting syntax
/// were it to appear as the first token on a line — a bullet/blockquote/ATX
/// heading marker, an ordered-list marker, a thematic break (`---`, `***`,
/// `___`), a setext underline (`===`, or a lone `-`), a GFM table row
/// (`|...`), an HTML block start (`<div>`, `<!--`, `</p>`, ...), a
/// link-reference definition (`[label]:...`), or a 4-space-indented code
/// block. Used by `wrap_tokens` to keep reflow idempotent: width-only line
/// breaking can otherwise strand a mid-sentence marker-shaped token (e.g. a
/// bare `-` or a `***` separator) at the start of a wrapped continuation
/// line, silently turning prose into a list/heading/rule/table/etc. on the
/// next pass (the "leaning toothpick" bug). Note this is intentionally
/// unconditional — it does not matter whether more content follows on the
/// line, since a paragraph's genuine first token can never itself be
/// marker-like (the source would have parsed as a list/blockquote/heading/
/// etc. instead, not a paragraph), so `wrap_tokens` only ever needs to keep
/// marker-like tokens off of *wrapped* line starts — forbidding them
/// unconditionally is simplest and only costs an occasional over-width
/// line, same as long words/URLs.
fn marker_like(word: &str) -> bool {
    if word == ">" {
        return true;
    }
    if !word.is_empty() && word.len() <= 6 && word.bytes().all(|b| b == b'#') {
        return true;
    }
    // GFM table row: a bare `|` opening a line reads as a table row (and,
    // paired with a plausible delimiter row, a whole table) on reparse.
    if word.starts_with('|') {
        return true;
    }
    // HTML block start: CommonMark HTML blocks (types 1-7) can interrupt a
    // paragraph, so a verbatim `Inline::Html` tag (emitted unescaped by
    // `build_tokens`, see its match arm) that gets wrapped to a line start
    // must not be reread as opening a block on the next pass. Matches an
    // opening tag (`<div`), a closing tag (`</div`), a comment (`<!--`), or
    // a processing instruction (`<?php`) — the CommonMark HTML-block
    // start conditions — not a bare `<` used as a comparison operator.
    if looks_like_html_start(word) {
        return true;
    }
    // Link-reference definition: `[label]: destination` at a line start
    // defines a reference rather than reading as inline text. `escape_text`
    // already backslash-escapes bare `[`/`]` in plain text (see below), and
    // no token `build_tokens` builds for a real link/image/footnote
    // reference has this shape (they read `[text](dest)` / `[^label]`, never
    // `[label]:`), so this arm is currently unreachable in practice — kept
    // as a defensive backstop, matching the `'*'` arm below.
    if word.starts_with('[') && word.contains("]:") {
        return true;
    }
    // 4-space indented code: `wrap_tokens` never itself emits leading
    // whitespace (words are glued with a single space or start a fresh line
    // with none) and `indent_block`'s container prefixes never exceed the
    // container's own established content width, so no token reaching this
    // function can currently start with real leading spaces — kept as a
    // defensive backstop, matching the `'*'` arm below.
    if word.starts_with("    ") {
        return true;
    }
    // Ordered-list marker: 1-9 ASCII digits followed by a single `.` or `)`.
    if let Some(rest) = word.strip_suffix('.').or_else(|| word.strip_suffix(')')) {
        if !rest.is_empty() && rest.len() <= 9 && rest.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    // A run of a single repeated character can be a bullet marker, a setext
    // underline, or a thematic break depending on the character and length.
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !chars.all(|c| c == first) {
        return false;
    }
    // Every char is `first`, and each arm below only matches ASCII markers,
    // so the byte length equals the char count — no second UTF-8 pass needed.
    let len = word.len();
    match first {
        '-' => true,     // bullet `-`, setext H2 `---`, thematic break
        '=' => true,     // setext H1 `===`
        '+' => len == 1, // bullet `+`
        // `escape_text` already backslash-escapes every `*` and `_` while
        // tokenizing, so these two arms are currently unreachable in
        // practice — kept as a defensive backstop.
        '*' => len == 1 || len >= 3,
        '_' => len >= 3,
        // `~` is NOT escaped by `escape_text`, so a bare `~~~`+ run of tildes
        // reaches `wrap_tokens` intact and opens a fenced code block if it
        // lands at a wrapped line start (`~~` len 2 is strikethrough, safe).
        '~' => len >= 3,
        _ => false,
    }
}

/// True when `word` opens like an HTML tag, closing tag, comment, or
/// processing instruction — the shapes CommonMark's HTML-block-start rule
/// recognizes — as opposed to a bare `<` used as a comparison operator
/// (`a < b`), which is not itself block-starting and shouldn't be forced off
/// a line start.
fn looks_like_html_start(word: &str) -> bool {
    let mut chars = word.chars();
    if chars.next() != Some('<') {
        return false;
    }
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
}

/// Backslash-escape the punctuation in `word` that would otherwise trigger
/// block-level reparsing if it started a line (see `marker_like`). All
/// escaped characters are CommonMark-escapable, so this is
/// meaning-preserving: the escaped form re-parses as the literal character,
/// e.g. `\-`, `\>`, `\##`, `\===` (escaping just the leading `=` already
/// stops the rest from reading as a setext underline), `\***`, `\___`,
/// `\|`, `\<div>`, `\[label]:`. The 4-space-indent backstop is the one
/// exception: a leading space is not itself CommonMark-escapable, but
/// prefixing it with `\` still defuses the hazard, since the line then
/// starts with a non-space character and no longer carries 4 columns of
/// leading indentation.
fn escape_line_start(word: &str) -> String {
    if let Some(rest) = word.strip_suffix('.').or_else(|| word.strip_suffix(')')) {
        let marker = &word[rest.len()..];
        return format!("{rest}\\{marker}");
    }
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => format!("\\{c}{}", chars.as_str()),
        None => word.to_string(),
    }
}

fn wrap_tokens(tokens: &[Token], width: usize, reflow: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for t in tokens {
        match t {
            Token::Break => {
                lines.push(format!("{cur}  "));
                cur.clear();
            }
            Token::Word(w) => {
                if cur.is_empty() {
                    // `w` is the very first token on this line. If this is
                    // the paragraph's genuine first token it can never be
                    // marker-like (see `marker_like`'s doc comment), so this
                    // escape path only actually fires right after a hard
                    // `Token::Break` — the fallback mechanism for when there
                    // is no previous line to force `w` onto instead.
                    if marker_like(w) {
                        cur.push_str(&escape_line_start(w));
                    } else {
                        cur.push_str(w);
                    }
                } else if !reflow
                    || cur.chars().count() + 1 + w.chars().count() <= width
                    // Primary mechanism: rather than start a new line with a
                    // token that would re-parse as block syntax, force a
                    // marker-like word onto the current line even past `width`
                    // (the formatter already tolerates over-width lines for
                    // long words/URLs).
                    || marker_like(w)
                {
                    cur.push(' ');
                    cur.push_str(w);
                } else {
                    lines.push(std::mem::take(&mut cur));
                    cur.push_str(w);
                }
            }
        }
    }
    lines.push(cur);
    lines
}

fn build_tokens(items: &[Inline], cfg: &FmtConfig) -> Vec<Token> {
    let mut out = Vec::new();
    // Adjacent `Inline::Text` nodes are not necessarily whitespace-separated
    // in the source: a backslash escape (e.g. `foo\_bar`) splits into
    // multiple `Text` events around the escaped character, with nothing
    // between them. Splitting each node on whitespace independently would
    // spuriously break such a run into separate words (`foo`, `_`, `bar`),
    // which then get rejoined with a space on the next reflow pass —
    // breaking idempotency. Coalesce a run of consecutive `Text` nodes
    // before splitting on whitespace so escape boundaries don't count as
    // word boundaries.
    let mut text_run = String::new();
    let flush_text_run = |run: &mut String, out: &mut Vec<Token>| {
        for w in run.split_whitespace() {
            out.push(Token::Word(escape_text(w)));
        }
        run.clear();
    };
    for item in items {
        // Every non-text inline ends the current text run before it is handled,
        // so an escape-induced `Text` boundary (see the comment above) never
        // counts as a word boundary. `Text` itself just extends the run.
        if !matches!(item, Inline::Text(_)) {
            flush_text_run(&mut text_run, &mut out);
        }
        match item {
            Inline::Text(s) => text_run.push_str(s),
            Inline::Code(s) => out.push(Token::Word(render_code_span(s))),
            Inline::Emphasis(children) => glue(
                &mut out,
                children,
                cfg,
                cfg.emphasis_marker.as_str(),
                cfg.emphasis_marker.as_str(),
            ),
            Inline::Strong(children) => glue(
                &mut out,
                children,
                cfg,
                cfg.strong_marker.as_str(),
                cfg.strong_marker.as_str(),
            ),
            Inline::Strikethrough(children) => glue(&mut out, children, cfg, "~~", "~~"),
            Inline::Link {
                dest,
                title,
                children,
            } => {
                let text = flatten_words(children, cfg);
                let t = title
                    .as_ref()
                    .map(|t| format!(" \"{t}\""))
                    .unwrap_or_default();
                out.push(Token::Word(format!("[{text}]({dest}{t})")));
            }
            Inline::Image { dest, title, alt } => {
                let t = title
                    .as_ref()
                    .map(|t| format!(" \"{t}\""))
                    .unwrap_or_default();
                out.push(Token::Word(format!("![{alt}]({dest}{t})")));
            }
            Inline::SoftBreak => {}
            Inline::HardBreak => out.push(Token::Break),
            Inline::Html(s) => out.push(Token::Word(s.clone())),
            Inline::FootnoteReference(s) => out.push(Token::Word(format!("[^{s}]"))),
        }
    }
    flush_text_run(&mut text_run, &mut out);
    out
}

/// Render inline content onto a single line, with soft breaks collapsed to
/// spaces. Used wherever a construct can't be wrapped (headings, table
/// cells, link text).
fn flatten_words(children: &[Inline], cfg: &FmtConfig) -> String {
    build_tokens(children, cfg)
        .iter()
        .map(|t| match t {
            Token::Word(w) => w.clone(),
            Token::Break => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn glue(out: &mut Vec<Token>, children: &[Inline], cfg: &FmtConfig, open: &str, close: &str) {
    let mut toks = build_tokens(children, cfg);
    if toks.is_empty() {
        return;
    }
    if let Some(idx) = toks.iter().position(|t| matches!(t, Token::Word(_))) {
        if let Token::Word(w) = &mut toks[idx] {
            *w = format!("{open}{w}");
        }
    }
    if let Some(idx) = toks.iter().rposition(|t| matches!(t, Token::Word(_))) {
        if let Token::Word(w) = &mut toks[idx] {
            *w = format!("{w}{close}");
        }
    }
    out.extend(toks);
}

/// Escape characters in a word of plain text that would otherwise be
/// reinterpreted as Markdown syntax when re-parsed.
fn escape_text(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn render_code_span(s: &str) -> String {
    let longest = longest_run_of(s, '`');
    let fence: String = "`".repeat(longest + 1);
    let needs_pad = s.starts_with('`')
        || s.ends_with('`')
        || (s.starts_with(' ') && s.ends_with(' ') && !s.trim().is_empty());
    if needs_pad {
        format!("{fence} {s} {fence}")
    } else {
        format!("{fence}{s}{fence}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These two `marker_like` arms (4-space indent, link-reference
    // definition) are defensive backstops: no token `build_tokens` can
    // currently build has either shape (see the comments on those arms), so
    // there is no full `adept_fmt::format_str` repro to drive through
    // `crates/adept_fmt/tests/format_tests.rs`'s marker-guard helpers the
    // way the reachable constructs (table row, HTML block start) are
    // tested. These call `marker_like`/`escape_line_start` directly instead,
    // pinning the recognition and its escape so a future token producer
    // that *does* carry a leading-space or link-ref-def-shaped word is
    // covered from day one rather than silently falling through.

    #[test]
    fn four_space_indent_is_marker_like_and_escapes_to_non_space_start() {
        assert!(marker_like("    code"));
        let escaped = escape_line_start("    code");
        assert!(!escaped.starts_with(' '), "escaped form: {escaped}");
    }

    #[test]
    fn link_reference_definition_is_marker_like_and_escapes_to_non_bracket_start() {
        assert!(marker_like("[foo]:"));
        let escaped = escape_line_start("[foo]:");
        assert!(!escaped.starts_with('['), "escaped form: {escaped}");
    }

    #[test]
    fn bare_less_than_comparison_is_not_marker_like() {
        // `a < b` mid-prose is a plain comparison, not an HTML tag start;
        // `looks_like_html_start` must not flag a lone `<`.
        assert!(!marker_like("<"));
    }
}
