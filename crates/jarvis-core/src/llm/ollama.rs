//! Minimal Ollama HTTP client: health, model list, streaming chat with real
//! cancellation, and schema-constrained JSON responses.
//!
//! Endpoints used (verified against Ollama 0.40): `GET /api/version`,
//! `GET /api/tags`, `POST /api/chat` (NDJSON when streaming).

use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
/// How long Ollama keeps the model in memory after an answer (its default is
/// 5 minutes); shorter frees ~4 GB sooner at the cost of a reload later.
pub const KEEP_ALIVE: &str = "2m";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self { role, content: content.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl ModelInfo {
    pub fn supports_thinking(&self) -> bool {
        self.capabilities.iter().any(|c| c == "thinking")
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OllamaHealth {
    Available { version: String, models: Vec<ModelInfo> },
    NoModels { version: String, recovery: String },
    Unavailable { error: OllamaErrorInfo },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OllamaError {
    #[error("No se puede conectar con Ollama en {base_url}.")]
    Unreachable { base_url: String },
    #[error("Ollama no respondió a tiempo.")]
    Timeout,
    #[error("El modelo `{model}` no está instalado en Ollama.")]
    ModelNotFound { model: String },
    #[error("Ollama devolvió un error HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Ollama informó de un error durante la respuesta: {0}")]
    StreamError(String),
    #[error("La respuesta de Ollama se interrumpió antes de terminar.")]
    StreamInterrupted,
    #[error("Respuesta inesperada de Ollama: {0}")]
    InvalidResponse(String),
    #[error("Error de transporte: {0}")]
    Transport(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaErrorInfo {
    pub kind: &'static str,
    pub message: String,
    pub recovery: String,
    pub retryable: bool,
}

impl OllamaError {
    pub fn kind(&self) -> &'static str {
        match self {
            OllamaError::Unreachable { .. } => "unreachable",
            OllamaError::Timeout => "timeout",
            OllamaError::ModelNotFound { .. } => "model_not_found",
            OllamaError::Http { .. } => "http",
            OllamaError::StreamError(_) => "stream_error",
            OllamaError::StreamInterrupted => "stream_interrupted",
            OllamaError::InvalidResponse(_) => "invalid_response",
            OllamaError::Transport(_) => "transport",
        }
    }

    /// Transient failures worth an automatic retry.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            OllamaError::Timeout | OllamaError::StreamInterrupted | OllamaError::InvalidResponse(_)
        ) || matches!(self, OllamaError::Http { status, .. } if *status >= 500)
    }

    pub fn recovery(&self) -> String {
        match self {
            OllamaError::Unreachable { .. } => {
                "Abre la app Ollama o ejecuta `ollama serve` en una terminal y pulsa Reintentar.".into()
            }
            OllamaError::Timeout => {
                "El modelo puede estar cargándose en memoria. Espera unos segundos y reintenta.".into()
            }
            OllamaError::ModelNotFound { model } => {
                format!("Descárgalo con `ollama pull {model}` en una terminal o elige otro modelo en el selector.")
            }
            OllamaError::Http { status, .. } if *status >= 500 => {
                "Ollama tuvo un error interno. Reintenta; si persiste, reinicia Ollama.".into()
            }
            OllamaError::Http { .. } => "Revisa el modelo seleccionado y reintenta.".into(),
            OllamaError::StreamError(_) | OllamaError::StreamInterrupted => "Reintenta el mensaje.".into(),
            OllamaError::InvalidResponse(_) => "Reintenta; si persiste, prueba otro modelo.".into(),
            OllamaError::Transport(_) => "Comprueba que Ollama está en ejecución y reintenta.".into(),
        }
    }

    pub fn info(&self) -> OllamaErrorInfo {
        OllamaErrorInfo {
            kind: self.kind(),
            message: self.to_string(),
            recovery: self.recovery(),
            retryable: self.is_retryable(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    Done,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatOutcome {
    pub content: String,
    pub finish: Finish,
}

pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatMessage],
    /// `Some(false)` disables thinking on models that support it; `None` omits the field.
    pub think: Option<bool>,
    /// JSON schema to constrain the response (Ollama structured outputs).
    pub format: Option<serde_json::Value>,
}

impl ChatRequest<'_> {
    fn body(&self, stream: bool) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": self.messages,
            "stream": stream,
            "keep_alive": KEEP_ALIVE,
        });
        if let Some(think) = self.think {
            body["think"] = think.into();
        }
        if let Some(format) = &self.format {
            body["format"] = format.clone();
        }
        body
    }
}

#[derive(Deserialize)]
struct StreamLine {
    #[serde(default)]
    message: Option<LineMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct LineMessage {
    #[serde(default)]
    content: String,
}

#[derive(Clone)]
pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
    /// Max wait for response headers or between stream chunks.
    idle_timeout: Duration,
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_timeouts(base_url, Duration::from_secs(3), Duration::from_secs(120))
    }

    pub fn with_timeouts(base_url: impl Into<String>, connect: Duration, idle: Duration) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(connect)
            .build()
            .expect("reqwest client builds with static config");
        Self { base_url: base_url.into().trim_end_matches('/').to_string(), http, idle_timeout: idle }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn map_send_error(&self, e: reqwest::Error) -> OllamaError {
        if e.is_connect() {
            OllamaError::Unreachable { base_url: self.base_url.clone() }
        } else if e.is_timeout() {
            OllamaError::Timeout
        } else {
            OllamaError::Transport(e.to_string())
        }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, OllamaError> {
        let resp = tokio::time::timeout(self.idle_timeout, self.http.get(format!("{}{path}", self.base_url)).send())
            .await
            .map_err(|_| OllamaError::Timeout)?
            .map_err(|e| self.map_send_error(e))?;
        let resp = check_status(resp, None).await?;
        resp.json::<T>().await.map_err(|e| OllamaError::InvalidResponse(e.to_string()))
    }

    pub async fn version(&self) -> Result<String, OllamaError> {
        #[derive(Deserialize)]
        struct V {
            version: String,
        }
        Ok(self.get_json::<V>("/api/version").await?.version)
    }

    pub async fn models(&self) -> Result<Vec<ModelInfo>, OllamaError> {
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<ModelInfo>,
        }
        Ok(self.get_json::<Tags>("/api/tags").await?.models)
    }

    pub async fn health(&self) -> OllamaHealth {
        let version = match self.version().await {
            Ok(v) => v,
            Err(e) => return OllamaHealth::Unavailable { error: e.info() },
        };
        match self.models().await {
            Ok(models) if models.is_empty() => OllamaHealth::NoModels {
                version,
                recovery: "No hay modelos instalados. Descarga uno con `ollama pull <modelo>` en una terminal.".into(),
            },
            Ok(models) => OllamaHealth::Available { version, models },
            Err(e) => OllamaHealth::Unavailable { error: e.info() },
        }
    }

    async fn post_chat(&self, req: &ChatRequest<'_>, stream: bool, cancel: &CancellationToken) -> Result<Option<reqwest::Response>, OllamaError> {
        let send = self.http.post(format!("{}/api/chat", self.base_url)).json(&req.body(stream)).send();
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Ok(None),
            r = tokio::time::timeout(self.idle_timeout, send) => r
                .map_err(|_| OllamaError::Timeout)?
                .map_err(|e| self.map_send_error(e))?,
        };
        check_status(resp, Some(req.model)).await.map(Some)
    }

    /// Streams a chat completion, calling `on_delta` for every content chunk.
    /// Cancelling `cancel` drops the HTTP request, which makes Ollama stop
    /// generating; the partial content is returned with `Finish::Cancelled`.
    pub async fn chat_stream<F: FnMut(&str)>(
        &self,
        req: &ChatRequest<'_>,
        cancel: &CancellationToken,
        mut on_delta: F,
    ) -> Result<ChatOutcome, OllamaError> {
        let mut content = String::new();
        let Some(resp) = self.post_chat(req, true, cancel).await? else {
            return Ok(ChatOutcome { content, finish: Finish::Cancelled });
        };
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Ok(ChatOutcome { content, finish: Finish::Cancelled }),
                n = tokio::time::timeout(self.idle_timeout, stream.next()) => n,
            };
            let chunk = match next {
                Err(_) => return Err(OllamaError::Timeout),
                Ok(None) => break,
                Ok(Some(Err(_))) => return Err(OllamaError::StreamInterrupted),
                Ok(Some(Ok(bytes))) => bytes,
            };
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if handle_line(&line, &mut content, &mut on_delta)? {
                    return Ok(ChatOutcome { content, finish: Finish::Done });
                }
            }
        }
        if handle_line(&buf, &mut content, &mut on_delta)? {
            return Ok(ChatOutcome { content, finish: Finish::Done });
        }
        Err(OllamaError::StreamInterrupted)
    }

    /// Non-streaming chat, typically with `format` set to a JSON schema.
    pub async fn chat_once(&self, req: &ChatRequest<'_>, cancel: &CancellationToken) -> Result<ChatOutcome, OllamaError> {
        let Some(resp) = self.post_chat(req, false, cancel).await? else {
            return Ok(ChatOutcome { content: String::new(), finish: Finish::Cancelled });
        };
        let line: StreamLine = tokio::select! {
            _ = cancel.cancelled() => return Ok(ChatOutcome { content: String::new(), finish: Finish::Cancelled }),
            r = tokio::time::timeout(self.idle_timeout, resp.json::<StreamLine>()) => r
                .map_err(|_| OllamaError::Timeout)?
                .map_err(|e| OllamaError::InvalidResponse(e.to_string()))?,
        };
        if let Some(err) = line.error {
            return Err(OllamaError::StreamError(err));
        }
        let content = line.message.map(|m| m.content).unwrap_or_default();
        Ok(ChatOutcome { content, finish: Finish::Done })
    }
}

/// Returns `Ok(true)` when the line marks the end of the response.
fn handle_line<F: FnMut(&str)>(line: &[u8], content: &mut String, on_delta: &mut F) -> Result<bool, OllamaError> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(false);
    }
    let parsed: StreamLine = serde_json::from_str(text).map_err(|e| OllamaError::InvalidResponse(e.to_string()))?;
    if let Some(err) = parsed.error {
        return Err(OllamaError::StreamError(err));
    }
    if let Some(msg) = parsed.message
        && !msg.content.is_empty() {
            content.push_str(&msg.content);
            on_delta(&msg.content);
        }
    Ok(parsed.done)
}

async fn check_status(resp: reqwest::Response, model: Option<&str>) -> Result<reqwest::Response, OllamaError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or(body);
    if status.as_u16() == 404 && message.contains("not found")
        && let Some(model) = model {
            return Err(OllamaError::ModelNotFound { model: model.to_string() });
        }
    Err(OllamaError::Http { status: status.as_u16(), message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_ask_ollama_to_free_the_model_early() {
        let messages = [ChatMessage::new(Role::User, "hola")];
        let req = ChatRequest { model: "m", messages: &messages, think: None, format: None };
        assert_eq!(req.body(true)["keep_alive"], KEEP_ALIVE);
        assert_eq!(req.body(false)["keep_alive"], "2m");
    }
}
