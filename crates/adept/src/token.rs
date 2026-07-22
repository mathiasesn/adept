//! Token counting via `tiktoken-rs`, used for description/body token budget
//! rules.

use tiktoken_rs::CoreBPE;

/// Which BPE encoding a [`TokenCounter`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tokenizer {
    /// The `o200k_base` encoding, used by GPT-4o and newer models. The
    /// default.
    #[default]
    O200kBase,
    /// The `cl100k_base` encoding, used by GPT-4/GPT-3.5-era models.
    Cl100kBase,
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
    /// Returns an error message if the underlying `tiktoken-rs` encoding
    /// tables fail to load.
    pub fn new(tokenizer: Tokenizer) -> Result<Self, String> {
        let bpe = match tokenizer {
            Tokenizer::O200kBase => tiktoken_rs::o200k_base(),
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base(),
        }
        .map_err(|e| e.to_string())?;
        Ok(Self { bpe, tokenizer })
    }

    /// Which tokenizer this counter uses.
    pub fn tokenizer(&self) -> Tokenizer {
        self.tokenizer
    }

    /// Count the number of tokens in `text`.
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
