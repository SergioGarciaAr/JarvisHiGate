mod common;

use std::time::Duration;

use common::{Canned, chat_line, dead_url, ok, serve};
use jarvis_core::llm::{ChatMessage, ChatRequest, Finish, OllamaClient, OllamaError, OllamaHealth, Role};
use tokio_util::sync::CancellationToken;

fn msgs() -> Vec<ChatMessage> {
    vec![ChatMessage::new(Role::User, "hola")]
}

fn client(url: &str) -> OllamaClient {
    OllamaClient::with_timeouts(url, Duration::from_millis(500), Duration::from_millis(500))
}

#[tokio::test]
async fn streams_deltas_until_done() {
    let url = serve(vec![Canned::Stream {
        lines: vec![chat_line("Ho", false), chat_line("la", false), chat_line("", true)],
        delay: Duration::from_millis(5),
    }])
    .await;
    let m = msgs();
    let mut deltas = Vec::new();
    let out = client(&url)
        .chat_stream(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &CancellationToken::new(), |d| {
            deltas.push(d.to_string())
        })
        .await
        .unwrap();
    assert_eq!(out.content, "Hola");
    assert_eq!(out.finish, Finish::Done);
    assert_eq!(deltas, vec!["Ho", "la"]);
}

#[tokio::test]
async fn unreachable_server() {
    let url = dead_url().await;
    let m = msgs();
    let err = client(&url)
        .chat_stream(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, OllamaError::Unreachable { .. }), "{err:?}");
    assert!(err.recovery().contains("ollama serve"));
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn model_not_found() {
    let url = serve(vec![Canned::Json { status: 404, body: r#"{"error":"model 'x:1b' not found"}"#.into() }]).await;
    let m = msgs();
    let err = client(&url)
        .chat_stream(&ChatRequest { model: "x:1b", messages: &m, think: None, format: None }, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err, OllamaError::ModelNotFound { model: "x:1b".into() });
    assert!(err.recovery().contains("ollama pull x:1b"));
}

#[tokio::test]
async fn server_error_is_retryable() {
    let url = serve(vec![Canned::Json { status: 500, body: r#"{"error":"boom"}"#.into() }]).await;
    let m = msgs();
    let err = client(&url)
        .chat_once(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err, OllamaError::Http { status: 500, message: "boom".into() });
    assert!(err.is_retryable());
}

#[tokio::test]
async fn error_line_mid_stream() {
    let url = serve(vec![Canned::Stream {
        lines: vec![chat_line("a", false), r#"{"error":"out of memory"}"#.into()],
        delay: Duration::from_millis(1),
    }])
    .await;
    let m = msgs();
    let err = client(&url)
        .chat_stream(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err, OllamaError::StreamError("out of memory".into()));
}

#[tokio::test]
async fn stream_cut_before_done() {
    let url = serve(vec![Canned::Stream { lines: vec![chat_line("a", false)], delay: Duration::from_millis(1) }]).await;
    let m = msgs();
    let err = client(&url)
        .chat_stream(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err, OllamaError::StreamInterrupted);
}

#[tokio::test]
async fn hanging_server_times_out() {
    let url = serve(vec![Canned::Hang]).await;
    let m = msgs();
    let err = OllamaClient::with_timeouts(&url, Duration::from_millis(500), Duration::from_millis(150))
        .chat_stream(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err, OllamaError::Timeout);
}

#[tokio::test]
async fn cancel_mid_stream_keeps_partial() {
    let lines = (0..50).map(|i| chat_line(&format!("t{i} "), false)).collect();
    let url = serve(vec![Canned::Stream { lines, delay: Duration::from_millis(20) }]).await;
    let m = msgs();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let mut count = 0;
    let out = client(&url)
        .chat_stream(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &cancel, |_| {
            count += 1;
            if count == 2 {
                c2.cancel();
            }
        })
        .await
        .unwrap();
    assert_eq!(out.finish, Finish::Cancelled);
    assert_eq!(out.content, "t0 t1 ");
}

#[tokio::test]
async fn cancel_before_response() {
    let url = serve(vec![Canned::Hang]).await;
    let m = msgs();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c2.cancel();
    });
    let out = client(&url)
        .chat_stream(&ChatRequest { model: "m", messages: &m, think: None, format: None }, &cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(out.finish, Finish::Cancelled);
    assert!(out.content.is_empty());
}

#[tokio::test]
async fn health_states() {
    let url = serve(vec![
        ok(r#"{"version":"0.40.6"}"#),
        ok(r#"{"models":[{"name":"qwen3.5:4b","size":1,"capabilities":["completion","thinking"]}]}"#),
    ])
    .await;
    match client(&url).health().await {
        OllamaHealth::Available { version, models } => {
            assert_eq!(version, "0.40.6");
            assert_eq!(models[0].name, "qwen3.5:4b");
            assert!(models[0].supports_thinking());
        }
        other => panic!("{other:?}"),
    }

    let url = serve(vec![ok(r#"{"version":"0.40.6"}"#), ok(r#"{"models":[]}"#)]).await;
    assert!(matches!(client(&url).health().await, OllamaHealth::NoModels { .. }));

    let url = dead_url().await;
    match client(&url).health().await {
        OllamaHealth::Unavailable { error } => assert_eq!(error.kind, "unreachable"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn session_trims_history() {
    use jarvis_core::llm::ChatSession;
    let mut s = ChatSession::new(2);
    for i in 0..5 {
        s.record_turn(&format!("u{i}"), &format!("a{i}"));
    }
    assert_eq!(s.len(), 4);
    let req = s.request_messages("nuevo");
    assert_eq!(req.first().unwrap().role, Role::System);
    assert_eq!(req[1].content, "u3");
    assert_eq!(req.last().unwrap().content, "nuevo");
}
