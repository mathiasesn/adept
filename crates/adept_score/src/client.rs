//! The [`LlmClient`] trait — the seam between scoring logic and any actual
//! chat-completions backend — plus the real [`OpenAiCompatClient`]
//! implementation and the [`crate::mock::MockLlmClient`] test double.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capture::{CaptureSink, CapturedCall};

/// A `String` whose `Debug` and `Display` both render `****`.
///
/// The API key used to be a plain `String` inside a `#[derive(Debug)]`
/// config struct, which meant a single `{:?}` anywhere — a log line, a
/// panic message, an `assert_eq!` failure — would print it. Redaction is
/// enforced here *by type* rather than by remembering at every call site:
/// there is no way to render the inner value except by calling
/// [`RedactedString::expose_secret`], which is deliberately hard to type by
/// accident and easy to grep for.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedString(String);

impl RedactedString {
    /// Wrap a secret.
    #[must_use]
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Borrow the underlying secret. Every call site of this method is a
    /// place a secret can escape — keep them countable.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl From<String> for RedactedString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for RedactedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("****")
    }
}

impl fmt::Display for RedactedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("****")
    }
}

/// A single message in a chat-completion request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The message role (`"system"`, `"user"`, or `"assistant"`).
    pub role: ChatRole,
    /// The message content.
    pub content: String,
}

impl ChatMessage {
    /// Construct a `system` message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    /// Construct a `user` message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }
}

/// The role of a [`ChatMessage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// A system/instruction message.
    System,
    /// A user message.
    User,
    /// An assistant (model) message.
    Assistant,
}

/// A request to a chat-completions endpoint.
///
/// This is the single operation [`LlmClient`] abstracts: everything scoring
/// needs to send is expressed here, and everything a client needs to return
/// is a plain [`String`] of the top choice's text content (see
/// [`LlmClient::chat`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// The model identifier to request.
    pub model: String,
    /// The conversation so far, in order.
    pub messages: Vec<ChatMessage>,
    /// Sampling temperature. Scoring always uses `0.0` for the judge calls
    /// to minimize variance; prompt-generation calls may use a higher value.
    pub temperature: f32,
    /// An optional seed for reproducible sampling, where the backend
    /// supports it.
    pub seed: Option<u64>,
    /// Whether to request a JSON-formatted response (maps to
    /// `response_format: {"type": "json_object"}` on OpenAI-compatible
    /// backends that support it).
    pub json_response: bool,
}

impl ChatRequest {
    /// Construct a request with the given model and messages, temperature
    /// `0.0`, no seed, and no JSON response format requested.
    #[must_use]
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: 0.0,
            seed: None,
            json_response: false,
        }
    }

    /// Builder-style: set the temperature.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Builder-style: set the seed.
    #[must_use]
    pub fn with_seed(mut self, seed: Option<u64>) -> Self {
        self.seed = seed;
        self
    }

    /// Builder-style: request a JSON-formatted response.
    #[must_use]
    pub fn with_json_response(mut self, json_response: bool) -> Self {
        self.json_response = json_response;
        self
    }
}

/// The text content of the top choice returned by a chat-completions call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// The response text.
    pub content: String,
}

impl ChatResponse {
    /// Construct a response with the given content.
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

/// Errors that can occur while talking to an LLM backend.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The underlying HTTP request failed (connection error, DNS, etc).
    #[error("request failed: {0}")]
    Request(String),

    /// The request timed out.
    #[error("request timed out after {0:?}")]
    Timeout(Duration),

    /// The backend returned a non-2xx status code.
    #[error("backend returned status {status}: {body}")]
    Status {
        /// The HTTP status code.
        status: u16,
        /// The response body (truncated if very large).
        body: String,
    },

    /// The response body could not be parsed as the expected JSON shape.
    #[error("malformed response: {0}")]
    MalformedResponse(String),

    /// The response parsed as JSON but contained no choices.
    #[error("backend returned no choices")]
    EmptyChoices,

    /// The number of retries was exhausted without success.
    #[error("exhausted {0} retries against backend")]
    RetriesExhausted(u32),
}

/// Abstracts one operation — send a chat-completion request and get back
/// text — so that scoring logic can be tested offline against
/// [`crate::mock::MockLlmClient`] and run for real against
/// [`OpenAiCompatClient`] (or any other implementation).
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Send a chat-completion request and return the top choice's text.
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError>;
}

/// Where [`OpenAiCompatClient::from_env`] reads its configuration from.
pub const ENV_BASE_URL: &str = "ADEPT_BASE_URL";
/// See [`ENV_BASE_URL`].
pub const ENV_API_KEY: &str = "ADEPT_API_KEY";
/// See [`ENV_BASE_URL`].
pub const ENV_MODEL: &str = "ADEPT_MODEL";

/// The default OpenAI-compatible base URL, used when neither an explicit
/// override nor `ADEPT_BASE_URL` is set.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Explicit configuration overrides for [`OpenAiCompatClient::resolve`].
///
/// Any field left as `None` falls back to the corresponding `ADEPT_*`
/// environment variable, and then to a hardcoded default (base URL only —
/// `model` has no default and is a hard error if never resolved).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmConfig {
    /// Explicit base URL override.
    pub base_url: Option<String>,
    /// Explicit API key override.
    pub api_key: Option<String>,
    /// Explicit model override.
    pub model: Option<String>,
}

/// Fully resolved configuration for [`OpenAiCompatClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLlmConfig {
    /// The base URL of the OpenAI-compatible endpoint, e.g.
    /// `https://api.openai.com/v1`. Requests are sent to
    /// `{base_url}/chat/completions`.
    pub base_url: String,
    /// The API key to send as a bearer token, if any (some local servers
    /// require none). Redacted in `Debug`/`Display` — see
    /// [`RedactedString`]. [`LlmConfig::api_key`] stays a plain `String`
    /// (it is the *input* side, built from CLI flags and config files);
    /// [`LlmConfig::resolve`] is the single place the wrap happens, so
    /// everything downstream of resolution is redacted by construction.
    pub api_key: Option<RedactedString>,
    /// The model identifier to request.
    pub model: String,
}

/// Errors resolving [`OpenAiCompatClient`] configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// No model was given explicitly, via `ADEPT_MODEL`, or otherwise.
    #[error(
        "no model configured: set ADEPT_MODEL, pass --model, or set LlmConfig::model explicitly"
    )]
    MissingModel,
}

impl LlmConfig {
    /// Resolve this configuration against environment variables and
    /// defaults.
    ///
    /// Precedence, per field, highest to lowest:
    /// 1. The explicit value on `self` (e.g. a CLI flag).
    /// 2. The corresponding `ADEPT_*` environment variable.
    /// 3. A hardcoded default (`base_url` only; `api_key` defaults to
    ///    `None`; `model` has no default).
    ///
    /// # Errors
    /// Returns [`ConfigError::MissingModel`] if no model is resolved from
    /// any source.
    pub fn resolve(&self) -> Result<ResolvedLlmConfig, ConfigError> {
        let base_url = self
            .base_url
            .clone()
            .or_else(|| std::env::var(ENV_BASE_URL).ok())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let api_key = self
            .api_key
            .clone()
            .or_else(|| std::env::var(ENV_API_KEY).ok())
            .map(RedactedString::from);
        let model = self
            .model
            .clone()
            .or_else(|| std::env::var(ENV_MODEL).ok())
            .ok_or(ConfigError::MissingModel)?;
        Ok(ResolvedLlmConfig {
            base_url,
            api_key,
            model,
        })
    }
}

/// A real [`LlmClient`] backed by any OpenAI-compatible `/chat/completions`
/// endpoint (OpenAI itself, local servers such as Ollama/vLLM, or Anthropic
/// via its OpenAI-compatibility layer).
pub struct OpenAiCompatClient {
    http: reqwest::Client,
    config: ResolvedLlmConfig,
    max_retries: u32,
    timeout: Duration,
    /// Opt-in on-disk payload capture. `None` — the default — means the
    /// client behaves exactly as it did before capture existed: no extra
    /// syscalls, no extra output.
    capture: Option<Arc<CaptureSink>>,
}

/// Body sent to `/chat/completions`.
#[derive(Debug, Serialize)]
struct RawRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<RawResponseFormat>,
}

#[derive(Debug, Serialize)]
struct RawResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    choices: Vec<RawChoice>,
}

#[derive(Debug, Deserialize)]
struct RawChoice {
    message: RawResponseMessage,
}

#[derive(Debug, Deserialize)]
struct RawResponseMessage {
    content: Option<String>,
}

impl OpenAiCompatClient {
    /// Construct a client from fully resolved configuration.
    #[must_use]
    pub fn new(config: ResolvedLlmConfig) -> Self {
        Self::with_timeout_and_retries(config, Duration::from_secs(60), 3)
    }

    /// Construct a client with an explicit timeout and retry budget.
    #[must_use]
    pub fn with_timeout_and_retries(
        config: ResolvedLlmConfig,
        timeout: Duration,
        max_retries: u32,
    ) -> Self {
        // Building the underlying reqwest client cannot fail for the
        // options we use here (no TLS/proxy misconfiguration is possible
        // through this constructor), so `expect` is appropriate rather than
        // propagating a fallible constructor for an infallible case.
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client with default TLS backend should always build");
        Self {
            http,
            config,
            max_retries,
            timeout,
            capture: None,
        }
    }

    /// Builder-style: capture every request/response payload into `sink`.
    ///
    /// Opt-in by design — with no sink attached, stdout and stderr are
    /// byte-identical to a build without capture at all.
    #[must_use]
    pub fn with_capture(mut self, sink: Arc<CaptureSink>) -> Self {
        self.capture = Some(sink);
        self
    }

    /// Resolve configuration from explicit overrides, `ADEPT_*` environment
    /// variables, and defaults; see [`LlmConfig::resolve`] for precedence.
    ///
    /// # Errors
    /// Returns [`ConfigError::MissingModel`] if no model can be resolved.
    pub fn from_env(explicit: &LlmConfig) -> Result<Self, ConfigError> {
        Ok(Self::new(explicit.resolve()?))
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    /// Issue exactly one HTTP call. `attempt` is the zero-based retry index
    /// from [`LlmClient::chat`]; it is threaded in purely so that log events
    /// and capture artifacts can be attributed to the right try.
    ///
    /// This is the single funnel every LLM call in the workspace passes
    /// through — both `adept score` and `adept fix` — which is why it is the
    /// only instrumentation site.
    async fn send_once(
        &self,
        request: &ChatRequest,
        attempt: u32,
    ) -> Result<ChatResponse, LlmError> {
        let raw = RawRequest {
            model: &request.model,
            messages: &request.messages,
            temperature: request.temperature,
            seed: request.seed,
            response_format: request.json_response.then_some(RawResponseFormat {
                kind: "json_object",
            }),
        };

        let endpoint = self.endpoint();
        // Serialized once and reused for the log event and the capture
        // artifact, so what is recorded is provably what reqwest sends.
        let request_body = serde_json::to_string(&raw)
            .map_err(|e| LlmError::Request(format!("failed to serialize request: {e}")))?;

        tracing::debug!(
            endpoint = %endpoint,
            attempt,
            model = %request.model,
            body = %request_body,
            "sending chat-completions request"
        );
        tracing::trace!(
            endpoint = %endpoint,
            attempt,
            temperature = request.temperature,
            seed = ?request.seed,
            json_response = request.json_response,
            message_count = request.messages.len(),
            "chat-completions request detail"
        );

        // Collected for capture. `Authorization` is deliberately absent —
        // it is never put into this map, so it cannot leak from it.
        let request_headers =
            BTreeMap::from([("content-type".to_string(), "application/json".to_string())]);

        let mut builder = self
            .http
            .post(&endpoint)
            .header("content-type", "application/json")
            .body(request_body.clone());
        if let Some(key) = &self.config.api_key {
            builder = builder.bearer_auth(key.expose_secret());
        }

        let started_at = jiff::Timestamp::now().to_string();
        let clock = Instant::now();

        let response = match builder.send().await {
            Ok(response) => response,
            Err(e) => {
                let err = if e.is_timeout() {
                    LlmError::Timeout(self.timeout)
                } else {
                    LlmError::Request(e.to_string())
                };
                tracing::debug!(endpoint = %endpoint, attempt, error = %err, "chat-completions request failed");
                self.capture(CapturedCall {
                    attempt,
                    endpoint: endpoint.clone(),
                    status: None,
                    request_headers,
                    response_headers: BTreeMap::new(),
                    request_body,
                    response_body: String::new(),
                    started_at,
                    finished_at: jiff::Timestamp::now().to_string(),
                    duration_ms: clock.elapsed().as_millis() as u64,
                    outcome: self.outcome_of(&err, attempt),
                    step: None,
                });
                return Err(err);
            }
        };

        let status = response.status();
        let response_headers = header_map(response.headers());
        let success = status.is_success();
        // Read the body before deciding anything: a non-2xx body and a
        // malformed 2xx body are both payloads the reader needs on disk.
        let body_result = response.text().await;
        let finished_at = jiff::Timestamp::now().to_string();
        let duration_ms = clock.elapsed().as_millis() as u64;
        let body_text = match &body_result {
            Ok(text) => text.clone(),
            Err(_) if !success => String::new(),
            Err(e) => {
                let err = LlmError::MalformedResponse(e.to_string());
                tracing::debug!(endpoint = %endpoint, attempt, status = status.as_u16(), error = %err, "failed to read response body");
                self.capture(CapturedCall {
                    attempt,
                    endpoint: endpoint.clone(),
                    status: Some(status.as_u16()),
                    request_headers,
                    response_headers,
                    request_body,
                    response_body: String::new(),
                    started_at,
                    finished_at,
                    duration_ms,
                    outcome: self.outcome_of(&err, attempt),
                    step: None,
                });
                return Err(err);
            }
        };

        // Emitted before parsing, and verbatim: a body that fails
        // `parse_chat_response` is exactly the one worth reading.
        tracing::debug!(
            endpoint = %endpoint,
            attempt,
            status = status.as_u16(),
            body = %body_text,
            "received chat-completions response"
        );

        let outcome = if success {
            match parse_chat_response(&body_text) {
                Ok(response) => {
                    self.capture(CapturedCall {
                        attempt,
                        endpoint,
                        status: Some(status.as_u16()),
                        request_headers,
                        response_headers,
                        request_body,
                        response_body: body_text,
                        started_at,
                        finished_at,
                        duration_ms,
                        outcome: "ok".to_string(),
                        step: None,
                    });
                    return Ok(response);
                }
                Err(err) => err,
            }
        } else {
            tracing::debug!(
                endpoint = %endpoint,
                attempt,
                status = status.as_u16(),
                body = %body_text,
                "chat-completions returned a non-success status"
            );
            LlmError::Status {
                status: status.as_u16(),
                body: body_text.clone(),
            }
        };

        self.capture(CapturedCall {
            attempt,
            endpoint,
            status: Some(status.as_u16()),
            request_headers,
            response_headers,
            request_body,
            response_body: body_text,
            started_at,
            finished_at,
            duration_ms,
            outcome: self.outcome_of(&outcome, attempt),
            step: None,
        });
        Err(outcome)
    }

    /// Hand one call's evidence to the capture sink, if one is attached.
    fn capture(&self, call: CapturedCall) {
        if let Some(sink) = &self.capture {
            sink.record_call(&call);
        }
    }

    /// The `outcome` string recorded for a failed call: `"retried"` when
    /// [`LlmClient::chat`] is going to try again, otherwise the error
    /// variant's name.
    fn outcome_of(&self, err: &LlmError, attempt: u32) -> String {
        if Self::should_retry(err) && attempt < self.max_retries {
            return "retried".to_string();
        }
        match err {
            LlmError::Request(_) => "Request",
            LlmError::Timeout(_) => "Timeout",
            LlmError::Status { .. } => "Status",
            LlmError::MalformedResponse(_) => "MalformedResponse",
            LlmError::EmptyChoices => "EmptyChoices",
            LlmError::RetriesExhausted(_) => "RetriesExhausted",
        }
        .to_string()
    }

    fn should_retry(err: &LlmError) -> bool {
        matches!(
            err,
            LlmError::Status { status, .. } if *status == 429 || (500..600).contains(status)
        )
    }
}

/// Flatten reqwest's header map into something serializable.
///
/// Only ever applied to *response* headers; request headers are built by
/// hand so that `Authorization` is never collected in the first place.
fn header_map(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("<non-utf8>").to_string(),
            )
        })
        .collect()
}

/// Parse a raw `/chat/completions` JSON body into a [`ChatResponse`].
///
/// Extracted as a free function so response-parsing edge cases (malformed
/// JSON, empty `choices`, missing `content`) can be unit-tested without any
/// network access.
fn parse_chat_response(body_text: &str) -> Result<ChatResponse, LlmError> {
    let parsed: RawResponse =
        serde_json::from_str(body_text).map_err(|e| LlmError::MalformedResponse(e.to_string()))?;
    let first = parsed
        .choices
        .into_iter()
        .next()
        .ok_or(LlmError::EmptyChoices)?;
    let content = first.message.content.unwrap_or_default();
    Ok(ChatResponse::new(content))
}

#[async_trait]
impl LlmClient for OpenAiCompatClient {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            match self.send_once(&request, attempt).await {
                Ok(response) => return Ok(response),
                Err(err) if Self::should_retry(&err) && attempt < self.max_retries => {
                    let backoff = Duration::from_millis(200 * 2u64.pow(attempt));
                    tokio::time::sleep(backoff).await;
                    last_err = Some(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.unwrap_or(LlmError::RetriesExhausted(self.max_retries)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_response() {
        let body = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
        let response = parse_chat_response(body).unwrap();
        assert_eq!(response.content, "hello");
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse_chat_response("not json").unwrap_err();
        assert!(matches!(err, LlmError::MalformedResponse(_)));
    }

    #[test]
    fn rejects_empty_choices() {
        let body = r#"{"choices":[]}"#;
        let err = parse_chat_response(body).unwrap_err();
        assert!(matches!(err, LlmError::EmptyChoices));
    }

    #[test]
    fn missing_content_becomes_empty_string() {
        let body = r#"{"choices":[{"message":{}}]}"#;
        let response = parse_chat_response(body).unwrap();
        assert_eq!(response.content, "");
    }

    #[test]
    fn debug_of_resolved_config_never_leaks_the_key() {
        let resolved = ResolvedLlmConfig {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: Some(RedactedString::new("sk-super-secret-value")),
            model: "gpt-test".to_string(),
        };
        let rendered = format!("{resolved:?}");
        assert!(rendered.contains("****"), "got {rendered}");
        assert!(
            !rendered.contains("sk-super-secret-value"),
            "got {rendered}"
        );
        assert_eq!(
            format!("{}", resolved.api_key.as_ref().unwrap()),
            "****",
            "Display must redact too"
        );
        // The one sanctioned way out.
        assert_eq!(
            resolved.api_key.unwrap().expose_secret(),
            "sk-super-secret-value"
        );
    }

    // Environment variables are process-global, so run all env-touching
    // config tests as a single serialized test to avoid races with `cargo
    // test`'s default parallelism.
    #[test]
    fn config_resolution_precedence() {
        std::env::remove_var(ENV_MODEL);
        std::env::remove_var(ENV_BASE_URL);

        // No model anywhere -> error.
        assert!(matches!(
            LlmConfig::default().resolve(),
            Err(ConfigError::MissingModel)
        ));

        // Env var model, default base URL.
        std::env::set_var(ENV_MODEL, "env-model");
        let resolved = LlmConfig::default().resolve().unwrap();
        assert_eq!(resolved.model, "env-model");
        assert_eq!(resolved.base_url, DEFAULT_BASE_URL);

        // Explicit model overrides env model.
        let explicit = LlmConfig {
            model: Some("explicit-model".to_string()),
            ..Default::default()
        };
        let resolved = explicit.resolve().unwrap();
        assert_eq!(resolved.model, "explicit-model");

        std::env::remove_var(ENV_MODEL);
        std::env::remove_var(ENV_BASE_URL);
    }
}
