//! Speech-to-text integration point. Phase 3A ships no recognizer: this
//! module only reports, from read-only checks, what is missing, and defines
//! the interface a local engine (whisper.cpp or Apple SpeechTranscriber)
//! will implement in phase 3B.

use std::path::{Path, PathBuf};

use futures_util::future::BoxFuture;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineId {
    WhisperCpp,
    AppleSpeechTranscriber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    NotInstalled,
    ModelMissing,
    /// Present on the system but Jarvis has no adapter for it yet.
    NotIntegrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineStatus {
    pub engine: EngineId,
    pub status: EngineState,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SttStatus {
    /// Always false in phase 3A: there is no microphone capture yet.
    pub available: bool,
    pub reason: String,
    pub engines: Vec<EngineStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SttError {
    #[error("reconocimiento no disponible: {reason}")]
    Unavailable { reason: String },
    #[error("la transcripción falló: {message}")]
    Failed { message: String },
}

/// Interface for a local recognizer. Audio is 16 kHz mono f32 samples held in
/// memory; implementations must not persist it nor send it off the machine.
pub trait SpeechToText: Send + Sync {
    fn engine(&self) -> EngineId;
    fn transcribe(&self, samples: Vec<f32>, language: &str) -> BoxFuture<'_, Result<String, SttError>>;
}

/// Where `brew install whisper-cpp` would place the CLI (not installed by Jarvis).
pub const WHISPER_CLI_CANDIDATES: [&str; 2] = ["/opt/homebrew/bin/whisper-cli", "/usr/local/bin/whisper-cli"];

fn find_models(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("ggml-") && n.ends_with(".bin"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read-only detection: file existence checks, nothing is executed.
pub fn detect(models_dir: &Path, whisper_candidates: &[&str]) -> SttStatus {
    let cli = whisper_candidates.iter().find(|p| Path::new(p).is_file());
    let models = find_models(models_dir);
    let whisper = match (cli, models.is_empty()) {
        (None, _) => EngineStatus {
            engine: EngineId::WhisperCpp,
            status: EngineState::NotInstalled,
            detail: "whisper-cli no encontrado (requiere `brew install whisper-cpp`, pendiente de autorización)".into(),
        },
        (Some(path), true) => EngineStatus {
            engine: EngineId::WhisperCpp,
            status: EngineState::ModelMissing,
            detail: format!("{path} encontrado, pero no hay modelos ggml-*.bin en {}", models_dir.display()),
        },
        (Some(path), false) => EngineStatus {
            engine: EngineId::WhisperCpp,
            status: EngineState::NotIntegrated,
            detail: format!("{path} y {} modelo(s) encontrados; el adaptador llega en la fase 3B", models.len()),
        },
    };
    let apple = EngineStatus {
        engine: EngineId::AppleSpeechTranscriber,
        status: EngineState::NotIntegrated,
        detail: "SpeechTranscriber existe en macOS 26+ pero solo es accesible desde Swift; requiere un ayudante y que \
macOS descargue el modelo de español (fase 3B, pendiente de autorización)"
            .into(),
    };
    SttStatus {
        available: false,
        reason: "Reconocimiento de voz aún no disponible: falta la captura de micrófono y un motor local (fase 3B, \
pendiente de autorización). Puedes escribir y Jarvis puede leer las respuestas en voz alta."
            .into(),
        engines: vec![whisper, apple],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_installed_is_reported_honestly() {
        let s = detect(Path::new("/nonexistent/models"), &["/nonexistent/whisper-cli"]);
        assert!(!s.available);
        assert_eq!(s.engines[0].status, EngineState::NotInstalled);
        assert_eq!(s.engines[1].status, EngineState::NotIntegrated);
    }

    #[test]
    fn cli_without_model_and_with_model() {
        let dir = std::env::temp_dir().join(format!("jarvis-stt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cli = dir.join("whisper-cli");
        std::fs::write(&cli, "").unwrap();
        let cli_s = cli.to_str().unwrap();
        let models = dir.join("models");
        std::fs::create_dir_all(&models).unwrap();
        assert_eq!(detect(&models, &[cli_s]).engines[0].status, EngineState::ModelMissing);
        std::fs::write(models.join("ggml-small.bin"), "").unwrap();
        std::fs::write(models.join("notes.txt"), "").unwrap();
        let s = detect(&models, &[cli_s]);
        assert_eq!(s.engines[0].status, EngineState::NotIntegrated);
        assert!(!s.available, "still unavailable without capture and adapter");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
