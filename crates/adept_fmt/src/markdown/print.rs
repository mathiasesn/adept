//! Pretty-printing a Markdown AST back to normalized CommonMark text.

use crate::config::FmtConfig;

use super::ast::{Alignment, Block, Inline, ListItem};

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
    let lines = print_blocks(blocks, cfg);
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn print_blocks(blocks: &[Block], cfg: &FmtConfig) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(print_block(b, cfg));
    }
    lines
}

fn print_block(block: &Block, cfg: &FmtConfig) -> Vec<String> {
    match block {
        Block::Heading { level, inline } => {
            let level = (*level).clamp(1, 6);
            let text = render_inline_single_line(inline, cfg);
            let hashes = "#".repeat(level as usize);
            if text.is_empty() {
                vec![hashes]
            } else {
                vec![format!("{hashes} {text}")]
            }
        }
        Block::Paragraph(inline) => wrap_paragraph(inline, cfg),
        Block::BlockQuote(inner) => {
            let content = print_blocks(inner, cfg);
            indent_block(&content, "> ", "> ")
        }
        Block::List {
            ordered,
            start,
            tight,
            items,
        } => print_list(*ordered, *start, *tight, items, cfg),
        Block::CodeBlock { info, literal } => print_code_block(info, literal, cfg),
        Block::ThematicBreak => vec!["---".to_string()],
        Block::Table {
            alignments,
            header,
            rows,
        } => print_table(alignments, header, rows, cfg),
        Block::HtmlBlock(raw) => raw.lines().map(str::to_string).collect(),
        Block::FootnoteDefinition { label, blocks } => {
            let content = print_blocks(blocks, cfg);
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

        let mut content = print_blocks(&item.blocks, cfg);
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
    if literal.is_empty() {
        // no content lines
    } else {
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
                    .map(|c| render_inline_single_line(c, cfg))
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
                    cur.push_str(w);
                } else if !reflow || cur.chars().count() + 1 + w.chars().count() <= width {
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

fn render_inline_single_line(inline: &[Inline], cfg: &FmtConfig) -> String {
    build_tokens(inline, cfg)
        .iter()
        .map(|t| match t {
            Token::Word(w) => w.clone(),
            Token::Break => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_tokens(items: &[Inline], cfg: &FmtConfig) -> Vec<Token> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Inline::Text(s) => {
                for w in s.split_whitespace() {
                    out.push(Token::Word(escape_text(w)));
                }
            }
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
    out
}

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
