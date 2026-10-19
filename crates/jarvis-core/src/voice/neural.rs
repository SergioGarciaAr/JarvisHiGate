//! Neural TTS through a long-lived local sidecar (voice-engine/jarvis_tts/server.py).
//!
//! - Spawned once with a fixed interpreter, script and arguments; no shell.
//! - Runs offline: `HF_HUB_OFFLINE=1`, so the text never leaves the Mac and
//!   no model is downloaded at runtime.
//! - The model stays loaded between answers; audio plays through one
//!   continuous output stream inside the sidecar.
//! - Events carry the utterance id; callers ignore ids that are not current.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Mutex, broadcast, oneshot};

#[derive(Debug, Clone)]
pub struct NeuralConfig {
    pub python: PathBuf,
    pub script: PathBuf,
    pub voice: PathBuf,
    pub model_repo: String,
    /// Directory holding `hub/models--…` (Hugging Face cache layout).
    pub hf_home: PathBuf,
    pub gain: f32,
    pub ready_timeout: Duration,
}

impl NeuralConfig {
    fn model_dir(&self) -> PathBuf {
        self.hf_home.join("hub").join(format!("models--{}", self.model_repo.replace('/', "--")))
    }

    /// Read-only check of everything the sidecar needs. Never downloads.
    pub fn availability(&self) -> Result<(), String> {
        if !self.python.is_file() {
            return Err(format!("falta el entorno Python del motor neuronal ({})", self.python.display()));
        }
        if !self.script.is_file() {
            return Err(format!("falta el servidor del motor neuronal ({})", self.script.display()));
        }
        if !self.model_dir().is_dir() {
            return Err(format!("falta el modelo {} en {}", self.model_repo, self.hf_home.display()));
        }
        if !self.voice.is_file() || !self.voice.with_extension("txt").is_file() {
            return Err(format!("falta la voz de referencia {}", self.voice.display()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum NeuralEvent {
    Ready { load_ms: u64 },
    FirstAudio { id: u64, ms: i64 },
    Finished { id: u64, underruns: u64, underrun_ms: u64, audio_ms: u64 },
    Stopped { id: u64 },
    Error { id: Option<u64>, message: String },
    /// The process exited (crash, fatal error at start-up or shutdown).
    Exited { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NeuralError {
    #[error("motor neuronal no disponible: {reason}")]
    Unavailable { reason: String },
    #[error("no se pudo iniciar el motor neuronal: {message}")]
    Spawn { message: String },
    #[error("el motor neuronal no estuvo listo en {secs} s")]
    StartTimeout { secs: u64 },
    #[error("el motor neuronal terminó: {message}")]
    Exited { message: String },
    #[error("no se pudo comunicar con el motor neuronal: {message}")]
    Io { message: String },
}

struct Process {
    child: Child,
    stdin: ChildStdin,
}

pub struct NeuralTts {
    config: NeuralConfig,
    process: Mutex<Option<Process>>,
    events: broadcast::Sender<NeuralEvent>,
    stderr_tail: Arc<std::sync::Mutex<VecDeque<String>>>,
}

fn parse_event(line: &str) -> Option<NeuralEvent> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("event").and_then(|e| e.as_str()) == Some("fatal") {
        let message = value.get("message").and_then(|m| m.as_str()).unwrap_or_default().to_string();
        return Some(NeuralEvent::Exited { message });
    }
    serde_json::from_value(value).ok()
}

impl NeuralTts {
    pub fn new(config: NeuralConfig) -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        Arc::new(Self { config, process: Mutex::new(None), events, stderr_tail: Arc::new(std::sync::Mutex::new(VecDeque::new())) })
    }

    pub fn config(&self) -> &NeuralConfig {
        &self.config
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NeuralEvent> {
        self.events.subscribe()
    }

    pub async fn is_running(&self) -> bool {
        let mut guard = self.process.lock().await;
        match guard.as_mut() {
            Some(p) => matches!(p.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Starts the sidecar if needed and waits until the model is loaded.
    pub async fn ensure_started(self: &Arc<Self>) -> Result<(), NeuralError> {
        let mut guard = self.process.lock().await;
        if let Some(p) = guard.as_mut() {
            if matches!(p.child.try_wait(), Ok(None)) {
                return Ok(());
            }
            *guard = None;
        }
        self.config.availability().map_err(|reason| NeuralError::Unavailable { reason })?;
        let c = &self.config;
        let home = std::env::var("HOME").unwrap_or_default();
        let mut child = tokio::process::Command::new(&c.python)
            .arg(&c.script)
            .arg("--voice")
            .arg(&c.voice)
            .arg("--model")
            .arg(&c.model_repo)
            .arg("--gain")
            .arg(c.gain.to_string())
            .env_clear()
            .env("HOME", home)
            .env("PATH", "/usr/bin:/bin")
            .env("HF_HOME", &c.hf_home)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| NeuralError::Spawn { message: e.to_string() })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut ready_tx = Some(ready_tx);
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Some(ev) = parse_event(&line) else { continue };
                match &ev {
                    NeuralEvent::Ready { .. } => {
                        if let Some(tx) = ready_tx.take() {
                            let _ = tx.send(Ok(()));
                        }
                    }
                    NeuralEvent::Exited { message } => {
                        if let Some(tx) = ready_tx.take() {
                            let _ = tx.send(Err(message.clone()));
                        }
                    }
                    _ => {}
                }
                let _ = events.send(ev);
            }
            if let Some(tx) = ready_tx.take() {
                let _ = tx.send(Err("el proceso terminó antes de estar listo".into()));
            }
            let _ = events.send(NeuralEvent::Exited { message: "el proceso del motor neuronal terminó".into() });
        });
        let tail = self.stderr_tail.clone();
        tokio::spawn(async move {
            // Drained so the pipe never fills; only the last lines are kept for error reports.
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut t = tail.lock().expect("stderr tail");
                if t.len() >= 20 {
                    t.pop_front();
                }
                t.push_back(line);
            }
        });

        let failure = match tokio::time::timeout(c.ready_timeout, ready_rx).await {
            Ok(Ok(Ok(()))) => {
                *guard = Some(Process { child, stdin });
                return Ok(());
            }
            Ok(Ok(Err(message))) => message,
            Ok(Err(_)) => "el proceso terminó antes de estar listo".to_string(),
            Err(_) => {
                let _ = child.kill().await;
                return Err(NeuralError::StartTimeout { secs: c.ready_timeout.as_secs() });
            }
        };
        let _ = child.kill().await;
        let detail: Vec<String> = self.stderr_tail.lock().expect("stderr tail").iter().rev().take(3).cloned().collect();
        let message = if detail.is_empty() { failure } else { format!("{failure} · {}", detail.join(" | ")) };
        Err(NeuralError::Exited { message })
    }

    async fn send(self: &Arc<Self>, value: serde_json::Value) -> Result<(), NeuralError> {
        let mut guard = self.process.lock().await;
        let p = guard.as_mut().ok_or(NeuralError::Exited { message: "el motor no está en ejecución".into() })?;
        let mut line = value.to_string();
        line.push('\n');
        if let Err(e) = p.stdin.write_all(line.as_bytes()).await {
            *guard = None;
            return Err(NeuralError::Io { message: e.to_string() });
        }
        p.stdin.flush().await.map_err(|e| NeuralError::Io { message: e.to_string() })
    }

    /// Replaces any current utterance. Starts the sidecar if needed.
    pub async fn speak(self: &Arc<Self>, id: u64, text: &str) -> Result<(), NeuralError> {
        self.ensure_started().await?;
        self.send(serde_json::json!({ "cmd": "speak", "id": id, "text": text })).await
    }

    /// Stops generation and playback immediately (no-op if not running).
    pub async fn stop(self: &Arc<Self>) {
        if self.is_running().await {
            let _ = self.send(serde_json::json!({ "cmd": "stop" })).await;
        }
    }

    pub async fn shutdown(self: &Arc<Self>) {
        let _ = self.send(serde_json::json!({ "cmd": "shutdown" })).await;
        if let Some(mut p) = self.process.lock().await.take()
            && tokio::time::timeout(Duration::from_secs(3), p.child.wait()).await.is_err() {
                let _ = p.child.kill().await;
            }
    }
}
