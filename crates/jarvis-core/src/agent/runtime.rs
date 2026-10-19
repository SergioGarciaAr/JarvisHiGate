//! Read-only analysis runs (`jarvis piensa`, `-- <objetivo>`).
//!
//! The model never executes anything: it returns schema-constrained JSON
//! that is parsed into typed results and shown as a summary.

use std::sync::Arc;
use std::time::Instant;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::limits::{Budget, LimitError, Limits};
use super::tools::{RiskLevel, Tool, ToolError, ToolRegistry, ToolSpec};
use crate::events::{EventKind, EventStatus, Reporter};
use crate::llm::{ChatMessage, ChatRequest, Finish, OllamaClient, Role};

pub const LLM_ASK: &str = "llm.ask";

/// Asks the local model for a JSON response matching a schema.
pub struct LlmAskTool {
    client: OllamaClient,
}

impl LlmAskTool {
    pub fn new(client: OllamaClient) -> Arc<Self> {
        Arc::new(Self { client })
    }
}

#[derive(Deserialize)]
struct LlmAskArgs {
    model: String,
    #[serde(default)]
    think: Option<bool>,
    system: String,
    prompt: String,
    schema: serde_json::Value,
}

impl Tool for LlmAskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: LLM_ASK,
            description: "Consulta al modelo local de Ollama con respuesta JSON estructurada",
            risk: RiskLevel::ReadOnly,
        }
    }

    fn invoke(&self, args: serde_json::Value) -> BoxFuture<'_, Result<serde_json::Value, ToolError>> {
        Box::pin(async move {
            let a: LlmAskArgs = serde_json::from_value(args)
                .map_err(|e| ToolError::InvalidArgs { tool: LLM_ASK, reason: e.to_string() })?;
            let messages = [ChatMessage::new(Role::System, a.system), ChatMessage::new(Role::User, a.prompt)];
            let req = ChatRequest { model: &a.model, messages: &messages, think: a.think, format: Some(a.schema) };
            // Cancellation is handled by the caller dropping this future.
            let out = self
                .client
                .chat_once(&req, &CancellationToken::new())
                .await
                .map_err(|e| ToolError::Failed { message: format!("{e} {}", e.recovery()), retryable: e.is_retryable() })?;
            match out.finish {
                Finish::Done => Ok(json!({ "content": out.content })),
                Finish::Cancelled => Err(ToolError::Cancelled),
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkMode {
    Plan,
    Comparar,
    Analisis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Paso {
    pub titulo: String,
    pub detalle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanResult {
    pub objetivo: String,
    pub resumen: String,
    pub pasos: Vec<Paso>,
    pub riesgos: Vec<String>,
    pub siguiente_paso: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Opcion {
    pub nombre: String,
    pub ventajas: Vec<String>,
    pub inconvenientes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompararResult {
    pub objetivo: String,
    pub opciones: Vec<Opcion>,
    pub recomendacion: String,
    pub justificacion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalisisResult {
    pub objetivo: String,
    pub resumen: String,
    pub hallazgos: Vec<String>,
    pub siguiente_paso: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ThinkResult {
    Plan(PlanResult),
    Comparar(CompararResult),
    Analisis(AnalisisResult),
}

impl ThinkResult {
    fn next_step(&self) -> Option<&str> {
        match self {
            ThinkResult::Plan(p) => Some(p.siguiente_paso.as_str()),
            ThinkResult::Analisis(a) => Some(a.siguiente_paso.as_str()),
            ThinkResult::Comparar(_) => None,
        }
        .filter(|s| !s.trim().is_empty())
    }
}

const STR_ARRAY: &str = r#"{"type":"array","items":{"type":"string"}}"#;

fn schema(mode: ThinkMode) -> serde_json::Value {
    let arr: serde_json::Value = serde_json::from_str(STR_ARRAY).expect("static schema");
    match mode {
        ThinkMode::Plan => json!({
            "type": "object",
            "properties": {
                "objetivo": {"type": "string"},
                "resumen": {"type": "string"},
                "pasos": {"type": "array", "items": {"type": "object",
                    "properties": {"titulo": {"type": "string"}, "detalle": {"type": "string"}},
                    "required": ["titulo", "detalle"]}},
                "riesgos": arr,
                "siguiente_paso": {"type": "string"}
            },
            "required": ["objetivo", "resumen", "pasos", "riesgos", "siguiente_paso"]
        }),
        ThinkMode::Comparar => json!({
            "type": "object",
            "properties": {
                "objetivo": {"type": "string"},
                "opciones": {"type": "array", "items": {"type": "object",
                    "properties": {"nombre": {"type": "string"}, "ventajas": arr, "inconvenientes": arr},
                    "required": ["nombre", "ventajas", "inconvenientes"]}},
                "recomendacion": {"type": "string"},
                "justificacion": {"type": "string"}
            },
            "required": ["objetivo", "opciones", "recomendacion", "justificacion"]
        }),
        ThinkMode::Analisis => json!({
            "type": "object",
            "properties": {
                "objetivo": {"type": "string"},
                "resumen": {"type": "string"},
                "hallazgos": arr,
                "siguiente_paso": {"type": "string"}
            },
            "required": ["objetivo", "resumen", "hallazgos", "siguiente_paso"]
        }),
    }
}

fn system_prompt(mode: ThinkMode) -> &'static str {
    match mode {
        ThinkMode::Plan => "Eres Jarvis. Elabora un plan práctico y conciso para el objetivo del usuario. \
Responde solo con JSON válido según el esquema, en el idioma del usuario. Entre 3 y 7 pasos.",
        ThinkMode::Comparar => "Eres Jarvis. Compara de forma equilibrada las alternativas que plantea el usuario \
(si no hay alternativas explícitas, propone 2 o 3 razonables). Responde solo con JSON válido según el esquema, \
en el idioma del usuario.",
        ThinkMode::Analisis => "Eres Jarvis. Analiza el objetivo del usuario y resume hallazgos concretos y un \
próximo paso accionable. No afirmes haber ejecutado nada ni consultado el sistema. Responde solo con JSON válido \
según el esquema, en el idioma del usuario.",
    }
}

pub fn parse_result(mode: ThinkMode, content: &str) -> Result<ThinkResult, serde_json::Error> {
    Ok(match mode {
        ThinkMode::Plan => ThinkResult::Plan(serde_json::from_str(content)?),
        ThinkMode::Comparar => ThinkResult::Comparar(serde_json::from_str(content)?),
        ThinkMode::Analisis => ThinkResult::Analisis(serde_json::from_str(content)?),
    })
}

pub struct ThinkRequest {
    pub mode: ThinkMode,
    pub prompt: String,
    pub model: String,
    pub think: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RunOutcome {
    Completed { result: ThinkResult },
    Cancelled,
    Failed { message: String },
}

pub async fn run_think(
    registry: &ToolRegistry,
    req: ThinkRequest,
    reporter: &Reporter,
    limits: Limits,
    cancel: CancellationToken,
) -> RunOutcome {
    use EventKind as K;
    use EventStatus as S;

    let mut budget = Budget::new(limits, cancel);
    reporter.emit(K::Goal, S::Ok, req.prompt.clone(), Some(json!({ "mode": req.mode })));
    reporter.emit(K::Phase, S::Running, format!("Consulta estructurada al modelo {}", req.model), None);

    let args = json!({
        "model": req.model,
        "think": req.think,
        "system": system_prompt(req.mode),
        "prompt": req.prompt,
        "schema": schema(req.mode),
    });

    let max_retries = budget.limits().max_retries;
    let mut attempt = 0;
    let result = loop {
        attempt += 1;
        reporter.emit(K::ToolCall, S::Running, LLM_ASK, Some(json!({ "tool": LLM_ASK, "model": req.model, "attempt": attempt })));
        let started = Instant::now();
        let failure = match budget.step(registry.invoke(LLM_ASK, args.clone())).await {
            Err(LimitError::Cancelled) | Ok(Err(ToolError::Cancelled)) => {
                reporter.emit(K::Cancelled, S::Cancelled, "Ejecución cancelada por el usuario", None);
                return RunOutcome::Cancelled;
            }
            Err(e) => {
                let message = e.to_string();
                reporter.emit(K::Error, S::Failed, message.clone(), None);
                reporter.emit(K::Completed, S::Failed, "Finalizado con error", None);
                return RunOutcome::Failed { message };
            }
            Ok(Err(e)) => (e.to_string(), e.is_retryable()),
            Ok(Ok(value)) => {
                let content = value.get("content").and_then(|c| c.as_str()).unwrap_or_default();
                match parse_result(req.mode, content) {
                    Ok(result) => {
                        reporter.emit(
                            K::ToolResult,
                            S::Ok,
                            "Respuesta estructurada recibida",
                            Some(json!({ "chars": content.len(), "ms": started.elapsed().as_millis() as u64 })),
                        );
                        break result;
                    }
                    Err(e) => (format!("El modelo devolvió una respuesta que no cumple el esquema: {e}"), true),
                }
            }
        };
        let (message, retryable) = failure;
        reporter.emit(K::ToolResult, S::Failed, message.clone(), None);
        if retryable && attempt <= max_retries {
            reporter.emit(K::Retry, S::Running, format!("Reintento {attempt} de {max_retries}"), None);
            continue;
        }
        reporter.emit(K::Error, S::Failed, message.clone(), None);
        reporter.emit(K::Completed, S::Failed, "Finalizado con error", None);
        return RunOutcome::Failed { message };
    };

    reporter.emit(K::Phase, S::Ok, "Consulta completada", None);
    if let ThinkResult::Plan(plan) = &result {
        reporter.emit(K::Plan, S::Ok, plan.resumen.clone(), Some(json!({ "pasos": plan.pasos })));
    }
    reporter.emit(K::Result, S::Ok, "Resultado", Some(serde_json::to_value(&result).expect("serializable")));
    if let Some(next) = result.next_step() {
        reporter.emit(K::NextStep, S::Pending, next.to_string(), None);
    }
    reporter.emit(K::Completed, S::Ok, "Completado", None);
    RunOutcome::Completed { result }
}
