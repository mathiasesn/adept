//! Token counting via `tiktoken-rs`, used for description/body token budget
//! rules.

use tiktoken_rs::CoreBPE;

use crate::error::AdeptError;

/// Which BPE encoding a [`TokenCounter`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tokenizer {
    /// The `o200k_base` encoding, used by GPT-4o and newer models. The
    /// default.
    #[default]
    O200kBase,
    /// The `cl100k_base` encoding, used by GPT-4/GPT-3.5-era models.
    Cl100kBase,
}

impl std::fmt::Display for Tokenizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Tokenizer::O200kBase => "o200k_base",
            Tokenizer::Cl100kBase => "cl100k_base",
        };
        f.write_str(s)
    }
}

/// Counts tokens in text using a specific tiktoken BPE encoding.
///
/// Defaults to `o200k_base`; construct with [`TokenCounter::new`] and
/// [`Tokenizer::Cl100kBase`] to count against an older encoding instead.
pub struct TokenCounter {
    bpe: &'static CoreBPE,
    tokenizer: Tokenizer,
}

/// Process-wide cache of the loaded BPE tables.
///
/// Loading `o200k_base` parses ~200k merge entries from an embedded blob, so
/// it is far too expensive to repeat per `TokenCounter`. Constructing a
/// counter is on the hot path for the long-lived MCP server (once per tool
/// call) and for `score` (once per skill), so the tables are loaded at most
/// once each and shared. The load `Result` is cached too, so a failure is
/// reported to every caller rather than being retried forever.
fn load_bpe(tokenizer: Tokenizer) -> Result<&'static CoreBPE, AdeptError> {
    use std::sync::OnceLock;

    static O200K: OnceLock<Result<CoreBPE, String>> = OnceLock::new();
    static CL100K: OnceLock<Result<CoreBPE, String>> = OnceLock::new();

    let cell = match tokenizer {
        Tokenizer::O200kBase => &O200K,
        Tokenizer::Cl100kBase => &CL100K,
    };
    let loaded = cell.get_or_init(|| {
        match tokenizer {
            Tokenizer::O200kBase => tiktoken_rs::o200k_base(),
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base(),
        }
        .map_err(|e| e.to_string())
    });
    loaded
        .as_ref()
        .map_err(|message| AdeptError::TokenizerLoad {
            tokenizer,
            message: message.clone(),
        })
}

impl TokenCounter {
    /// Construct a counter for the given tokenizer.
    ///
    /// # Errors
    /// Returns [`AdeptError::TokenizerLoad`] if the underlying `tiktoken-rs`
    /// encoding tables fail to load.
    pub fn new(tokenizer: Tokenizer) -> Result<Self, AdeptError> {
        Ok(Self {
            bpe: load_bpe(tokenizer)?,
            tokenizer,
        })
    }

    /// Which tokenizer this counter uses.
    #[must_use]
    pub fn tokenizer(&self) -> Tokenizer {
        self.tokenizer
    }

    /// Count the number of tokens in `text`.
    #[must_use]
    pub fn count(&self, text: &str) -> usize {
        self.bpe.encode_ordinary(text).len()
    }
}

impl Default for TokenCounter {
    /// The default counter uses `o200k_base`.
    ///
    /// # Panics
    /// Panics if the `o200k_base` encoding tables fail to load, which
    /// should not happen with a correctly built `tiktoken-rs`.
    fn default() -> Self {
        Self::new(Tokenizer::O200kBase).expect("o200k_base encoding should always load")
    }
}
