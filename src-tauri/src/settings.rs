//! User settings persisted as JSON in the app config directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_SHORTCUT: &str = "Cmd+J";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub shortcut: String,
    pub model: Option<String>,
    /// Name of the `say` voice; `None` picks a Spanish voice.
    pub voice: Option<String>,
    /// Read finished answers aloud.
    pub autospeak: bool,
    /// "neural" or "say"; `None` uses neural when installed.
    pub tts_engine: Option<String>,
    /// Reference voice for the neural engine (file stem in voice-engine/models/voices).
    pub neural_voice: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { shortcut: DEFAULT_SHORTCUT.into(), model: None, voice: None, autospeak: false, tts_engine: None, neural_voice: None }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).expect("settings serialize"))
    }
}
