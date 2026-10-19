//! Tests against a real local Ollama. Run with:
//! `JARVIS_LIVE_MODEL=qwen3.5:4b cargo test -p jarvis-core --test live_ollama -- --ignored --nocapture`

use std::sync::Arc;

use jarvis_core::agent::{Limits, LlmAskTool, PolicyEngine, RunOutcome, ThinkMode, ThinkRequest, ToolRegistry, run_think};
use jarvis_core::events::{MemorySink, Reporter};
use jarvis_core::llm::{ChatMessage, ChatRequest, Finish, OllamaClient, OllamaHealth, Role};
use tokio_util::sync::CancellationToken;

async fn setup() -> (OllamaClient, String, Option<bool>) {
    let client = OllamaClient::new(jarvis_core::llm::ollama::DEFAULT_BASE_URL);
    let OllamaHealth::Available { models, .. } = client.health().await else { panic!("Ollama no disponible") };
    let name = std::env::var("JARVIS_LIVE_MODEL").unwrap_or_else(|_| models[0].name.clone());
    let think = models.iter().find(|m| m.name == name).filter(|m| m.supports_thinking()).map(|_| false);
    (client, name, think)
}

#[tokio::test]
#[ignore]
async fn live_stream_and_cancel() {
    let (client, model, think) = setup().await;
    let msgs = [ChatMessage::new(Role::User, "Cuenta del 1 al 30 separado por comas.")];
    let req = ChatRequest { model: &model, messages: &msgs, think, format: None };
    let out = client.chat_stream(&req, &CancellationToken::new(), |_| {}).await.unwrap();
    assert_eq!(out.finish, Finish::Done);
    println!("stream: {}", out.content);

    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let mut n = 0;
    let out = client
        .chat_stream(&req, &cancel, |_| {
            n += 1;
            if n == 3 {
                c2.cancel();
            }
        })
        .await
        .unwrap();
    assert_eq!(out.finish, Finish::Cancelled);
    println!("cancelado tras 3 deltas: {:?}", out.content);
}

#[tokio::test]
#[ignore]
async fn live_think_plan() {
    let (client, model, think) = setup().await;
    let mut reg = ToolRegistry::new(PolicyEngine::default());
    reg.register(LlmAskTool::new(client));
    let sink = Arc::new(MemorySink::default());
    let reporter = Reporter::new("live", sink.clone());
    let req = ThinkRequest { mode: ThinkMode::Plan, prompt: "organizar mi semana de estudio".into(), model, think };
    let out = run_think(&reg, req, &reporter, Limits::default(), CancellationToken::new()).await;
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    println!("eventos: {:?}", sink.kinds());
    assert!(matches!(out, RunOutcome::Completed { .. }));
}
