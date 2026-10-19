//! Single entry point for user input from the chat, the quick panel and
//! (later) voice transcripts.

use serde::Serialize;

use crate::command::{self, Command, ParseErrorInfo};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "route", rename_all = "snake_case")]
pub enum Route {
    Empty,
    Command { command: Command },
    CommandError { error: ParseErrorInfo },
    /// `-- <objetivo>`: analysis shown with the progress panel.
    Agent { goal: String },
    Chat { text: String },
}

pub fn route_input(input: &str) -> Route {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Route::Empty;
    }
    if command::is_command(trimmed) {
        return match command::parse(trimmed) {
            Ok(command) => Route::Command { command },
            Err(e) => Route::CommandError { error: e.info() },
        };
    }
    if let Some(rest) = trimmed.strip_prefix("--") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            let goal = rest.trim();
            if goal.is_empty() {
                return Route::CommandError {
                    error: ParseErrorInfo {
                        kind: "missing_goal",
                        message: "Falta el objetivo después de `--`.".into(),
                        hint: "Ejemplo: `-- analiza por qué mi build tarda tanto`.".into(),
                    },
                };
            }
            return Route::Agent { goal: goal.to_string() };
        }
        let flag = trimmed.split_whitespace().next().unwrap_or(trimmed);
        return Route::CommandError {
            error: ParseErrorInfo {
                kind: "flag_without_command",
                message: format!("`{flag}` es una opción, pero falta el comando."),
                hint: "Escribe el comando completo, p. ej. `jarvis diagnostico --ram`, o `-- <objetivo>` para un análisis.".into(),
            },
        };
    }
    Route::Chat { text: trimmed.to_string() }
}
