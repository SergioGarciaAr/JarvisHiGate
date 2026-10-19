//! Typed representation of an interpreted `jarvis …` command.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subcommand {
    Piensa,
    Diagnostico,
    Conversa,
    Claw,
    Ayuda,
}

impl Subcommand {
    pub const ALL: [Subcommand; 5] = [
        Subcommand::Piensa,
        Subcommand::Diagnostico,
        Subcommand::Conversa,
        Subcommand::Claw,
        Subcommand::Ayuda,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Subcommand::Piensa => "piensa",
            Subcommand::Diagnostico => "diagnostico",
            Subcommand::Conversa => "conversa",
            Subcommand::Claw => "claw",
            Subcommand::Ayuda => "ayuda",
        }
    }

    /// Accepts accented and English aliases (`diagnóstico`, `help`).
    pub fn from_token(token: &str) -> Option<Self> {
        match token.to_lowercase().as_str() {
            "piensa" => Some(Subcommand::Piensa),
            "diagnostico" | "diagnóstico" => Some(Subcommand::Diagnostico),
            "conversa" => Some(Subcommand::Conversa),
            "claw" => Some(Subcommand::Claw),
            "ayuda" | "help" => Some(Subcommand::Ayuda),
            _ => None,
        }
    }

    /// Flags accepted by this subcommand (besides `--help`).
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            Subcommand::Piensa => &["--plan", "--comparar"],
            Subcommand::Diagnostico => &["--full", "--red", "--ram"],
            Subcommand::Conversa => &["--voz", "--manos-libres"],
            Subcommand::Claw => &["--task"],
            Subcommand::Ayuda => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiensaMode {
    Plan,
    Comparar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticoScope {
    /// No flag: system, storage and memory overview without network probes.
    General,
    Full,
    Red,
    Ram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversaMode {
    Voz,
    ManosLibres,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Piensa { mode: PiensaMode, prompt: String },
    Diagnostico { scope: DiagnosticoScope },
    Conversa { mode: ConversaMode },
    Claw { task: String },
    Ayuda { topic: Option<Subcommand> },
}

impl Command {
    pub fn subcommand(&self) -> Subcommand {
        match self {
            Command::Piensa { .. } => Subcommand::Piensa,
            Command::Diagnostico { .. } => Subcommand::Diagnostico,
            Command::Conversa { .. } => Subcommand::Conversa,
            Command::Claw { .. } => Subcommand::Claw,
            Command::Ayuda { .. } => Subcommand::Ayuda,
        }
    }
}
