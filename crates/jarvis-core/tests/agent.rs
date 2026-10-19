mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use common::{Canned, chat_once_body, serve};
use futures_util::future::BoxFuture;
use jarvis_core::agent::{
    Budget, LimitError, Limits, LlmAskTool, PolicyEngine, PolicyError, RiskLevel, RunOutcome, ThinkMode, ThinkRequest,
    Tool, ToolError, ToolRegistry, ToolSpec, run_think,
};
use jarvis_core::events::{EventKind, MemorySink, Reporter};
use jarvis_core::llm::OllamaClient;
use tokio_util::sync::CancellationToken;

struct CountingTool {
    risk: RiskLevel,
    calls: Arc<AtomicU32>,
}

impl Tool for CountingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "test.tool", description: "test", risk: self.risk }
    }
    fn invoke(&self, _args: serde_json::Value) -> BoxFuture<'_, Result<serde_json::Value, ToolError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }
}

#[tokio::test]
async fn unregistered_tool_is_rejected() {
    let reg = ToolRegistry::new(PolicyEngine::default());
    let err = reg.invoke("shell.exec", serde_json::json!({"cmd": "ls"})).await.unwrap_err();
    assert_eq!(err, ToolError::NotRegistered("shell.exec".into()));
}

#[tokio::test]
async fn non_read_only_tools_are_denied_and_not_invoked() {
    for risk in [RiskLevel::SystemAffecting, RiskLevel::Destructive] {
        let calls = Arc::new(AtomicU32::new(0));
        let mut reg = ToolRegistry::new(PolicyEngine::default());
        reg.register(Arc::new(CountingTool { risk, calls: calls.clone() }));
        let err = reg.invoke("test.tool", serde_json::Value::Null).await.unwrap_err();
        assert_eq!(err, ToolError::Denied(PolicyError::Denied { tool: "test.tool", risk }));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn read_only_tool_is_allowed() {
    let calls = Arc::new(AtomicU32::new(0));
    let mut reg = ToolRegistry::new(PolicyEngine::default());
    reg.register(Arc::new(CountingTool { risk: RiskLevel::ReadOnly, calls: calls.clone() }));
    reg.invoke("test.tool", serde_json::Value::Null).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn budget_enforces_steps_timeouts_and_cancel() {
    let limits = Limits { max_steps: 1, step_timeout: Duration::from_millis(50), total_timeout: Duration::from_secs(5), max_retries: 0 };
    let mut b = Budget::new(limits, CancellationToken::new());
    assert_eq!(b.step(async { 1 }).await, Ok(1));
    assert_eq!(b.step(async { 2 }).await, Err(LimitError::StepLimit(1)));

    let mut b = Budget::new(Limits { max_steps: 5, ..limits }, CancellationToken::new());
    assert_eq!(b.step(tokio::time::sleep(Duration::from_secs(2))).await, Err(LimitError::StepTimeout));

    let limits = Limits { max_steps: 5, step_timeout: Duration::from_secs(5), total_timeout: Duration::from_millis(50), max_retries: 0 };
    let mut b = Budget::new(limits, CancellationToken::new());
    assert_eq!(b.step(tokio::time::sleep(Duration::from_secs(2))).await, Err(LimitError::TotalTimeout));

    let cancel = CancellationToken::new();
    let mut b = Budget::new(Limits::default(), cancel.clone());
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        c2.cancel();
    });
    assert_eq!(b.step(tokio::time::sleep(Duration::from_secs(2))).await, Err(LimitError::Cancelled));
    assert_eq!(b.step(async {}).await, Err(LimitError::Cancelled));
}

const PLAN_JSON: &str = r#"{"objetivo":"o","resumen":"r","pasos":[{"titulo":"t","detalle":"d"}],"riesgos":[],"siguiente_paso":"empieza"}"#;

fn setup(url: &str) -> (ToolRegistry, Arc<MemorySink>, Reporter) {
    let mut reg = ToolRegistry::new(PolicyEngine::default());
    reg.register(LlmAskTool::new(OllamaClient::with_timeouts(url, Duration::from_millis(500), Duration::from_secs(2))));
    let sink = Arc::new(MemorySink::default());
    let reporter = Reporter::new("run-1", sink.clone());
    (reg, sink, reporter)
}

fn req(mode: ThinkMode) -> ThinkRequest {
    ThinkRequest { mode, prompt: "organizar mi semana".into(), model: "m".into(), think: None }
}

#[tokio::test]
async fn think_plan_completes_with_real_events() {
    let url = serve(vec![chat_once_body(PLAN_JSON)]).await;
    let (reg, sink, reporter) = setup(&url);
    let out = run_think(&reg, req(ThinkMode::Plan), &reporter, Limits::default(), CancellationToken::new()).await;
    assert!(matches!(out, RunOutcome::Completed { .. }), "{out:?}");
    use EventKind::*;
    assert_eq!(sink.kinds(), vec![Goal, Phase, ToolCall, ToolResult, Phase, Plan, Result, NextStep, Completed]);
    let seqs: Vec<u32> = sink.0.lock().unwrap().iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (0..9).collect::<Vec<_>>());
}

#[tokio::test]
async fn think_retries_on_invalid_json() {
    let url = serve(vec![chat_once_body("no es json"), chat_once_body(PLAN_JSON)]).await;
    let (reg, sink, reporter) = setup(&url);
    let out = run_think(&reg, req(ThinkMode::Plan), &reporter, Limits::default(), CancellationToken::new()).await;
    assert!(matches!(out, RunOutcome::Completed { .. }), "{out:?}");
    assert!(sink.kinds().contains(&EventKind::Retry));
}

#[tokio::test]
async fn think_fails_after_retries_exhausted() {
    let url = serve(vec![chat_once_body("x"), chat_once_body("y")]).await;
    let (reg, sink, reporter) = setup(&url);
    let out = run_think(&reg, req(ThinkMode::Comparar), &reporter, Limits::default(), CancellationToken::new()).await;
    assert!(matches!(out, RunOutcome::Failed { .. }));
    let kinds = sink.kinds();
    assert_eq!(kinds.iter().filter(|k| **k == EventKind::Retry).count(), 1);
    assert_eq!(kinds.last(), Some(&EventKind::Completed));
}

#[tokio::test]
async fn think_missing_model_fails_without_retry() {
    let url = serve(vec![Canned::Json { status: 404, body: r#"{"error":"model 'm' not found"}"#.into() }]).await;
    let (reg, sink, reporter) = setup(&url);
    let out = run_think(&reg, req(ThinkMode::Analisis), &reporter, Limits::default(), CancellationToken::new()).await;
    match out {
        RunOutcome::Failed { message } => assert!(message.contains("ollama pull m"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(!sink.kinds().contains(&EventKind::Retry));
}

#[tokio::test]
async fn think_can_be_cancelled() {
    let url = serve(vec![Canned::Hang]).await;
    let (reg, sink, reporter) = setup(&url);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c2.cancel();
    });
    let out = run_think(&reg, req(ThinkMode::Plan), &reporter, Limits::default(), cancel).await;
    assert_eq!(out, RunOutcome::Cancelled);
    assert_eq!(sink.kinds().last(), Some(&EventKind::Cancelled));
}

#[test]
fn telemetry_snapshot_is_real_or_marked_unavailable() {
    let mut t = jarvis_core::telemetry::TelemetryService::new();
    let first = t.snapshot();
    assert!(first.cpu_percent.is_none());
    assert!(first.unavailable.iter().any(|u| u.metric == "cpu"));
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    let second = t.snapshot();
    assert!(second.cpu_percent.is_some());
    assert!(second.mem_total.unwrap_or(0) > 0);
    assert!(second.cpu_cores > 0);
}
