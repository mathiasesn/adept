//! [`MockLlmClient`], a scripted, offline [`LlmClient`] for tests.
//!
//! Every test in this crate (and in `adept_cli`, if it drives scoring in
//! its own tests) must use this rather than [`crate::client::OpenAiCompatClient`]
//! — no test may perform network I/O.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::client::{ChatRequest, ChatResponse, LlmClient, LlmError};

/// A scripted, offline [`LlmClient`].
///
/// Responses are served in FIFO order from a queue seeded via
/// [`MockLlmClient::new`] or [`MockLlmClient::push_response`]; every request
/// made is recorded and retrievable via [`MockLlmClient::calls`] so tests
/// can assert on what was actually sent (model, prompt content, etc).
///
/// If the queue is exhausted, `chat` returns [`LlmError::EmptyChoices`]
/// rather than panicking, so a test with an unexpectedly-large number of
/// calls fails with a normal `Result`-shaped assertion instead of a panic
/// deep in scoring logic.
pub struct MockLlmClient {
    responses: Mutex<std::collections::VecDeque<Result<ChatResponse, LlmError>>>,
    calls: Mutex<Vec<ChatRequest>>,
}

impl MockLlmClient {
    /// Construct a mock that serves `responses` in order, one per call.
    pub fn new(responses: Vec<Result<ChatResponse, LlmError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Construct a mock that serves the given strings, each wrapped as a
    /// successful [`ChatResponse`], in order.
    pub fn with_texts(texts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::new(
            texts
                .into_iter()
                .map(|t| Ok(ChatResponse::new(t.into())))
                .collect(),
        )
    }

    /// Push another scripted response onto the end of the queue.
    pub fn push_response(&self, response: Result<ChatResponse, LlmError>) {
        self.responses
            .lock()
            .expect("mock mutex poisoned")
            .push_back(response);
    }

    /// All requests made to this client so far, in order.
    pub fn calls(&self) -> Vec<ChatRequest> {
        self.calls.lock().expect("mock mutex poisoned").clone()
    }

    /// The number of requests made to this client so far.
    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("mock mutex poisoned").len()
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        self.calls
            .lock()
            .expect("mock mutex poisoned")
            .push(request);
        let mut responses = self.responses.lock().expect("mock mutex poisoned");
        match responses.pop_front() {
            Some(result) => result,
            None => Err(LlmError::EmptyChoices),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_responses_in_order_and_records_calls() {
        let mock = MockLlmClient::with_texts(vec!["first", "second"]);
        let r1 = mock.chat(ChatRequest::new("m", vec![])).await.unwrap();
        let r2 = mock.chat(ChatRequest::new("m", vec![])).await.unwrap();
        assert_eq!(r1.content, "first");
        assert_eq!(r2.content, "second");
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn exhausted_queue_errors_instead_of_panicking() {
        let mock = MockLlmClient::new(vec![]);
        let err = mock.chat(ChatRequest::new("m", vec![])).await.unwrap_err();
        assert!(matches!(err, LlmError::EmptyChoices));
    }
}
