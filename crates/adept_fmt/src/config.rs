//! Configuration for `adept_fmt`.

use serde::{Deserialize, Serialize};

/// The bullet marker used for unordered list items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BulletMarker {
    /// `-` (the default).
    #[default]
    Dash,
    /// `*`
    Asterisk,
    /// `+`
    Plus,
}

impl BulletMarker {
    /// The literal character this marker prints as.
    #[must_use]
    pub fn as_char(self) -> char {
        match self {
            BulletMarker::Dash => '-',
            BulletMarker::Asterisk => '*',
            BulletMarker::Plus => '+',
        }
    }
}

/// The marker used for emphasis (`*em*` vs `_em_`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EmphasisMarker {
    /// `_em_` (the default).
    #[default]
    Underscore,
    /// `*em*`
    Asterisk,
}

impl EmphasisMarker {
    /// The literal delimiter this marker prints as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EmphasisMarker::Underscore => "_",
            EmphasisMarker::Asterisk => "*",
        }
    }
}

/// The marker used for strong emphasis. Only `**strong**` is currently
/// supported (CommonMark also allows `__strong__`, but prettier-style tools
/// conventionally normalize to asterisks); the field exists so config files
/// can still express intent and so future variants can be added without a
/// breaking API change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StrongMarker {
    /// `**strong**` (the default, and currently the only supported value).
    #[default]
    Asterisk,
}

impl StrongMarker {
    /// The literal delimiter this marker prints as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            StrongMarker::Asterisk => "**",
        }
    }
}

/// The heading style to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum HeadingStyle {
    /// `#`-prefixed headings (the default).
    #[default]
    Atx,
    /// `Heading\n=======` / `Heading\n-------` underlined headings, for h1
    /// and h2 only — h3 and deeper have no setext form and are always
    /// printed as ATX regardless of this setting.
    Setext,
}

/// The character used for fenced code blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FenceChar {
    /// `` ` `` (the default).
    #[default]
    Backtick,
    /// `~`
    Tilde,
}

impl FenceChar {
    /// The literal character this fence uses.
    #[must_use]
    pub fn as_char(self) -> char {
        match self {
            FenceChar::Backtick => '`',
            FenceChar::Tilde => '~',
        }
    }
}

/// Configuration for [`crate::format_str`] and [`crate::format_skill`].
///
/// Deserializable via `serde` (e.g. from a `[fmt]` table in a config file);
/// [`FmtConfig::default`] provides prettier-like sensible defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct FmtConfig {
    /// The target line width for prose reflow and table-friendly wrapping.
    /// Defaults to `100`.
    pub line_width: usize,
    /// The bullet marker used for unordered list items.
    pub bullet_marker: BulletMarker,
    /// The marker used for emphasis (`_em_` vs `*em*`).
    pub emphasis_marker: EmphasisMarker,
    /// The marker used for strong emphasis.
    pub strong_marker: StrongMarker,
    /// The heading style to normalize headings to.
    pub heading_style: HeadingStyle,
    /// The character used for fenced code block delimiters.
    pub fence_char: FenceChar,
    /// Whether to reflow/wrap prose paragraphs to `line_width`. When
    /// `false`, paragraph text is still normalized (collapsed internal
    /// whitespace) but not wrapped to specific line lengths.
    pub reflow_prose: bool,
}

impl Default for FmtConfig {
    fn default() -> Self {
        Self {
            line_width: 100,
            bullet_marker: BulletMarker::default(),
            emphasis_marker: EmphasisMarker::default(),
            strong_marker: StrongMarker::default(),
            heading_style: HeadingStyle::default(),
            fence_char: FenceChar::default(),
            reflow_prose: true,
        }
    }
}
