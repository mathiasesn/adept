//! A minimal Markdown AST, built from `pulldown-cmark`'s event stream, that
//! is easy to pretty-print deterministically.
//!
//! This is intentionally a small subset of full CommonMark structure: just
//! enough to represent everything the printer needs to normalize (see the
//! crate-level docs for what is fully supported vs. passed through
//! verbatim).

/// A single list item: its optional task-list checked state, and its
/// contained blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    /// `Some(checked)` if this item started with a GFM task-list marker
    /// (`- [ ]` / `- [x]`).
    pub checked: Option<bool>,
    /// Whether this item's own blocks were free of blank-line separation in
    /// the source: a single bare-inline paragraph, optionally followed by
    /// further blocks (e.g. a nested list) with no blank line between them.
    /// This is a property of the item itself ("within-item" spacing),
    /// distinct from [`Block::List::tight`], which records whether blank
    /// lines separate this item from its siblings ("between-item" spacing).
    /// CommonMark still derives a list's loose/tight rendering from the
    /// list as a whole, so one item recording `false` here can make the
    /// whole list loose (see `tight` on the containing `List`), but a
    /// `true` here never by itself forces siblings loose.
    pub content_tight: bool,
    /// The blocks contained in this item.
    pub blocks: Vec<Block>,
}

/// A block-level Markdown node.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// An ATX or Setext heading. Printed form is a function of the
    /// printer's configured heading style and `level`; the AST does not
    /// record which form the source used.
    Heading {
        /// Heading level, 1-6.
        level: u8,
        /// The heading's inline content.
        inline: Vec<Inline>,
    },
    /// A paragraph of inline content.
    Paragraph(Vec<Inline>),
    /// A block quote, containing further blocks.
    BlockQuote(Vec<Block>),
    /// An ordered or unordered list.
    List {
        /// Whether this is an ordered (numbered) list.
        ordered: bool,
        /// The starting number for ordered lists (ignored for unordered).
        start: u64,
        /// Whether the list is "tight" (no blank lines between items, the
        /// "between-item" spacing); tight lists print without blank-line
        /// separators between items. Per CommonMark this is a whole-list
        /// property: it is false if any item is blank-separated from a
        /// sibling, or any item's own [`ListItem::content_tight`] is false.
        tight: bool,
        /// The list's items.
        items: Vec<ListItem>,
    },
    /// A code block, fenced or indented in the source; always printed
    /// fenced. Content is preserved verbatim (never reflowed/reindented).
    CodeBlock {
        /// The fence info string (e.g. the language), empty if none.
        info: String,
        /// The raw code content, without a trailing newline.
        literal: String,
    },
    /// A thematic break (`---`).
    ThematicBreak,
    /// A table, with per-column alignment, a header row, and body rows.
    Table {
        /// Per-column alignment.
        alignments: Vec<Alignment>,
        /// The header row's cells.
        header: Vec<Vec<Inline>>,
        /// The body rows, each a list of cells.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// A raw HTML block, passed through verbatim.
    HtmlBlock(String),
    /// Content nested deeper than [`crate::markdown::MAX_NESTING_DEPTH`]
    /// (e.g. thousands of nested block quotes/lists), flattened to plain
    /// text and passed through verbatim rather than being parsed into a
    /// deeply nested tree. Never reformatted.
    Raw(String),
    /// A footnote definition, containing further blocks.
    FootnoteDefinition {
        /// The footnote's label (without the `^`/brackets).
        label: String,
        /// The blocks contained in the definition.
        blocks: Vec<Block>,
    },
}

/// A column's text alignment in a table, mirroring
/// [`pulldown_cmark::Alignment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// No explicit alignment.
    None,
    /// Left-aligned (`:---`).
    Left,
    /// Center-aligned (`:---:`).
    Center,
    /// Right-aligned (`---:`).
    Right,
}

impl From<pulldown_cmark::Alignment> for Alignment {
    fn from(a: pulldown_cmark::Alignment) -> Self {
        match a {
            pulldown_cmark::Alignment::None => Alignment::None,
            pulldown_cmark::Alignment::Left => Alignment::Left,
            pulldown_cmark::Alignment::Center => Alignment::Center,
            pulldown_cmark::Alignment::Right => Alignment::Right,
        }
    }
}

/// An inline-level Markdown node.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    /// Plain text (already unescaped from the source).
    Text(String),
    /// An inline code span, content preserved verbatim.
    Code(String),
    /// Emphasized (`_..._`) content.
    Emphasis(Vec<Inline>),
    /// Strongly emphasized (`**...**`) content.
    Strong(Vec<Inline>),
    /// Strikethrough (`~~...~~`) content.
    Strikethrough(Vec<Inline>),
    /// A link.
    Link {
        /// The link destination URL.
        dest: String,
        /// An optional link title.
        title: Option<String>,
        /// The link's display content.
        children: Vec<Inline>,
    },
    /// An image.
    Image {
        /// The image destination URL.
        dest: String,
        /// An optional image title.
        title: Option<String>,
        /// The image's alt text.
        alt: String,
    },
    /// A soft line break (rendered as a single space when reflowed).
    SoftBreak,
    /// A hard line break (two trailing spaces, or a backslash).
    HardBreak,
    /// Raw inline HTML, passed through verbatim.
    Html(String),
    /// A footnote reference (`[^label]`).
    FootnoteReference(String),
}
