//! Glue between Tauri and the neural TTS sidecar (`jarvis_core::voice::neural`).
//! Only events of the current utterance reach the UI, so audio or state from
//! a cancelled or superseded answer can never reappear.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use jarvis_core::voice::neural::{NeuralConfig, NeuralEvent, NeuralTts};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::VoiceEvent;

pub const MODEL_REPO: &str = "mlx-community/Qwen3-TTS-12Hz-1.7B-Base-8bit";
pub const DEFAULT_VOICE: &str = "jarvis-es-1";
/// The model (~3.5 GB) is unloaded after this long without speech and
/// reloaded on demand (~4 s) by the next answer read aloud.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);
const IDLE_CHECK_EVERY: Duration = Duration::from_secs(30);

/// Unload only a running engine that is silent and has been idle long enough.
pub fn should_unload(running: bool, playing: bool, idle_for: Duration, limit: Duration) -> bool {
    running && !playing && idle_for >= limit
}

/// Sidecar script from this source tree (local builds on this Mac).
const SERVER_SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../voice-engine/jarvis_tts/server.py");

#[derive(Serialize, Clone)]
pub struct EngineInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub available: bool,
    pub reason: Option<String>,
}

pub struct NeuralHost {
    base: PathBuf,
    hf_home: PathBuf,
    tts: Arc<RwLock<Arc<NeuralTts>>>,
    /// (utterance id, request id) currently allowed to reach the UI.
    current: Arc<Mutex<Option<(u64, String)>>>,
    next_id: AtomicU64,
    /// Last speech request or end of playback; drives the idle unload.
    last_activity: Arc<Mutex<Instant>>,
}

impl NeuralHost {
    pub fn new(app: &AppHandle, app_data: PathBuf, home: PathBuf, voice: &str) -> Arc<Self> {
        let base = app_data.join("voice-engine");
        let hf_home = home.join("Library/Caches/Jarvis/hf");
        let host = Arc::new(Self {
            tts: Arc::new(RwLock::new(NeuralTts::new(Self::config(&base, &hf_home, voice)))),
            base,
            hf_home,
            current: Arc::new(Mutex::new(None)),
            // Distinct from `say` utterance ids so the UI never confuses them.
            next_id: AtomicU64::new(1_000_000),
            last_activity: Arc::new(Mutex::new(Instant::now())),
        });
        host.spawn_forwarder(app);
        Self::spawn_idle_unloader(Arc::downgrade(&host));
        host
    }

    fn touch(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }

    /// Frees the model's memory when voice has not been used for a while.
    fn spawn_idle_unloader(host: Weak<Self>) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(IDLE_CHECK_EVERY).await;
                let Some(host) = host.upgrade() else { return };
                let tts = host.tts();
                let playing = host.current.lock().unwrap().is_some();
                let idle_for = host.last_activity.lock().unwrap().elapsed();
                if should_unload(tts.is_running().await, playing, idle_for, IDLE_UNLOAD) {
                    tts.shutdown().await;
                }
            }
        });
    }

    fn config(base: &std::path::Path, hf_home: &std::path::Path, voice: &str) -> NeuralConfig {
        NeuralConfig {
            python: base.join("mlx-venv/bin/python"),
            script: PathBuf::from(SERVER_SCRIPT),
            voice: base.join("models/voices").join(format!("{voice}.wav")),
            model_repo: MODEL_REPO.into(),
            hf_home: hf_home.to_path_buf(),
            gain: 1.0,
            ready_timeout: Duration::from_secs(180),
        }
    }

    fn tts(&self) -> Arc<NeuralTts> {
        self.tts.read().expect("tts lock").clone()
    }

    /// Reference voices available for cloning (`<name>.wav` + `<name>.txt`).
    pub fn voices(&self) -> Vec<String> {
        let dir = self.base.join("models/voices");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|it| {
                it.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "wav") && p.with_extension("txt").is_file())
                    .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    pub fn info(&self) -> EngineInfo {
        let reason = self.tts().config().availability().err();
        EngineInfo { id: "neural", label: "Neuronal local · Qwen3-TTS", available: reason.is_none(), reason }
    }

    pub fn current_voice(&self) -> String {
        self.tts().config().voice.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Loads the model in the background so the first answer is quick.
    pub fn preload(self: &Arc<Self>) {
        let tts = self.tts();
        if tts.config().availability().is_ok() {
            tauri::async_runtime::spawn(async move {
                let _ = tts.ensure_started().await;
            });
        }
    }

    /// Starts speaking; returns the utterance id immediately. `Started` is
    /// emitted when audio actually reaches the device, not when requested.
    pub fn speak(self: &Arc<Self>, app: &AppHandle, request_id: String, text: String) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.touch();
        *self.current.lock().unwrap() = Some((id, request_id.clone()));
        let tts = self.tts();
        let current = self.current.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = tts.speak(id, &text).await {
                let mut cur = current.lock().unwrap();
                if cur.as_ref().is_some_and(|(c, _)| *c == id) {
                    *cur = None;
                    let _ = app.emit("jarvis://voice", VoiceEvent::Error { utterance: id, request_id, message: e.to_string() });
                }
            }
        });
        id
    }

    /// Stops generation and playback. Returns whether something was playing.
    pub fn stop(self: &Arc<Self>) -> bool {
        let had = self.current.lock().unwrap().is_some();
        let tts = self.tts();
        tauri::async_runtime::spawn(async move { tts.stop().await });
        had
    }

    /// Switches the reference voice. The new engine is preloaded only if
    /// `preload` (answers are read aloud); otherwise it loads on first use.
    pub fn set_voice(self: &Arc<Self>, app: &AppHandle, voice: &str, preload: bool) -> Result<(), String> {
        if !self.voices().iter().any(|v| v == voice) {
            return Err(format!("la voz «{voice}» no existe"));
        }
        let old = std::mem::replace(&mut *self.tts.write().unwrap(), NeuralTts::new(Self::config(&self.base, &self.hf_home, voice)));
        *self.current.lock().unwrap() = None;
        tauri::async_runtime::spawn(async move { old.shutdown().await });
        self.spawn_forwarder(app);
        if preload {
            self.preload();
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.tts().shutdown().await;
    }

    fn spawn_forwarder(&self, app: &AppHandle) {
        let mine = self.tts();
        let mut rx = mine.subscribe();
        let slot = self.tts.clone();
        let current = self.current.clone();
        let last_activity = self.last_activity.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok(ev) = rx.recv().await {
                // After a voice switch the old instance keeps emitting while it
                // shuts down; only the current instance may affect the UI.
                if !Arc::ptr_eq(&slot.read().expect("tts lock"), &mine) {
                    break;
                }
                let mut cur = current.lock().unwrap();
                let Some((cid, rid)) = cur.clone() else { continue };
                let out = match ev {
                    NeuralEvent::FirstAudio { id, .. } if id == cid => Some(VoiceEvent::Started { utterance: id, request_id: rid }),
                    NeuralEvent::Finished { id, .. } if id == cid => {
                        *cur = None;
                        Some(VoiceEvent::Finished { utterance: id, request_id: rid })
                    }
                    NeuralEvent::Stopped { id } if id == cid => {
                        *cur = None;
                        Some(VoiceEvent::Stopped { utterance: id, request_id: rid })
                    }
                    NeuralEvent::Error { id: Some(id), message } if id == cid => {
                        *cur = None;
                        Some(VoiceEvent::Error { utterance: id, request_id: rid, message })
                    }
                    NeuralEvent::Exited { message } => {
                        *cur = None;
                        Some(VoiceEvent::Error { utterance: cid, request_id: rid, message: format!("el motor neuronal se detuvo: {message}") })
                    }
                    _ => None,
                };
                drop(cur);
                if matches!(out, Some(VoiceEvent::Finished { .. } | VoiceEvent::Stopped { .. } | VoiceEvent::Error { .. })) {
                    *last_activity.lock().unwrap() = Instant::now();
                }
                if let Some(e) = out {
                    let _ = app.emit("jarvis://voice", e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn unloads_only_a_silent_engine_after_the_idle_limit() {
        assert!(should_unload(true, false, 10 * MIN, IDLE_UNLOAD));
        assert!(should_unload(true, false, 25 * MIN, IDLE_UNLOAD));
        assert!(!should_unload(true, false, 9 * MIN, IDLE_UNLOAD), "too early");
        assert!(!should_unload(true, true, 30 * MIN, IDLE_UNLOAD), "never while speaking");
        assert!(!should_unload(false, false, 30 * MIN, IDLE_UNLOAD), "nothing to unload");
        assert_eq!(IDLE_UNLOAD, 10 * MIN);
    }
}
