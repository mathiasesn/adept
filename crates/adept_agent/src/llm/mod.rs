//! LLM transport: the [`LlmClient`] seam, the real
//! [`OpenAiCompatClient`] implementation, the [`MockLlmClient`] offline test
//! double, and [`CaptureSink`] for recording calls to disk.
//!
//! Every capability that talks to a model — [`crate::eval`], `fix`, and
//! `create` — goes through a `&dyn LlmClient`, so callers can pass
//! [`OpenAiCompatClient`] for real requests or [`MockLlmClient`] for fully
//! offline tests.

mod capture;
mod client;
mod mock;

pub use capture::{CaptureSink, CapturedCall, RunMetadata};
pub use client::{
    llm_available, ChatMessage, ChatRequest, ChatResponse, ChatRole, ConfigError, LlmClient,
    LlmConfig, LlmError, OpenAiCompatClient, RedactedString, ResolvedLlmConfig, ScrubbedBody,
    DEFAULT_BASE_URL, ENV_API_KEY, ENV_BASE_URL, ENV_MODEL,
};
pub use mock::MockLlmClient;
