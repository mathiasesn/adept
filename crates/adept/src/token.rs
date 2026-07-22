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
    bpe: CoreBPE,
    tokenizer: Tokenizer,
}

impl TokenCounter {
    /// Construct a counter for the given tokenizer.
    ///
    /// # Errors
    /// Returns [`AdeptError::TokenizerLoad`] if the underlying `tiktoken-rs`
    /// encoding tables fail to load.
    pub fn new(tokenizer: Tokenizer) -> Result<Self, AdeptError> {
        let bpe = match tokenizer {
            Tokenizer::O200kBase => tiktoken_rs::o200k_base(),
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base(),
        }
        .map_err(|e| AdeptError::TokenizerLoad {
            tokenizer,
            message: e.to_string(),
        })?;
        Ok(Self { bpe, tokenizer })
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
