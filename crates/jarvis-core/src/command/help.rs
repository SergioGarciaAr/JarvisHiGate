//! Built-in help and per-command availability in the current build.

use serde::Serialize;

use super::types::{Command, Subcommand};

pub fn help_text(topic: Option<Subcommand>) -> String {
    match topic {
        None => "\
Comandos de Jarvis

  jarvis piensa --plan <tema>        Plan estructurado sobre un tema
  jarvis piensa --comparar <tema>    Comparación de alternativas
  jarvis diagnostico [--full|--red|--ram]
  jarvis conversa [--voz|--manos-libres]
  jarvis claw --task \"<tarea>\"
  jarvis ayuda [comando]

Atajos
  -- <objetivo>    Análisis con panel de progreso
  Cualquier otro texto se envía como conversación normal al modelo."
            .into(),
        Some(Subcommand::Piensa) => "\
jarvis piensa — razonamiento estructurado con el modelo local

  --plan       Devuelve objetivo, pasos, riesgos y próximo paso
  --comparar   Devuelve opciones con ventajas, inconvenientes y recomendación

Ejemplo: jarvis piensa --comparar SQLite o Postgres para una app de escritorio
Se muestra un resumen verificable, no el razonamiento interno del modelo."
            .into(),
        Some(Subcommand::Diagnostico) => "\
jarvis diagnostico — diagnóstico de solo lectura (no cierra procesos ni cambia ajustes)

  (sin opción)  Resumen general: carga, almacenamiento, presión de memoria y swap
  --ram         Memoria física, presión, swap, compresión y procesos por consumo
  --red         Interfaces, ruta, DNS, router y conexiones de prueba con tiempo límite
  --full        Informe combinado: sistema, memoria, almacenamiento y red"
            .into(),
        Some(Subcommand::Conversa) => "\
jarvis conversa — sesión de voz

  --voz           Conversación por turnos (por defecto)
  --manos-libres  Detección automática de voz"
            .into(),
        Some(Subcommand::Claw) => "\
jarvis claw — delegar una tarea en OpenClaw

  --task \"<tarea>\"   Tarea a enviar (obligatoria)"
            .into(),
        Some(Subcommand::Ayuda) => "jarvis ayuda [comando] — muestra esta ayuda.".into(),
    }
}

/// Whether a parsed command can run in this build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Availability {
    Ready,
    NotYet { phase: u8, reason: String, alternative: String },
}

pub fn availability(cmd: &Command) -> Availability {
    match cmd {
        Command::Piensa { .. } | Command::Ayuda { .. } | Command::Diagnostico { .. } => Availability::Ready,
        Command::Conversa { .. } => Availability::NotYet {
            phase: 3,
            reason: "La conversación por voz se implementa en la fase 3; aún no hay captura de audio ni STT/TTS.".into(),
            alternative: "Puedes conversar por texto en este mismo chat.".into(),
        },
        Command::Claw { .. } => Availability::NotYet {
            phase: 4,
            reason: "OpenClaw no está instalado en este equipo y su integración se hará en la fase 4.".into(),
            alternative: "Usa `jarvis piensa --plan` para obtener un plan de la tarea con el modelo local.".into(),
        },
    }
}
