//! NeuralTts process management against a fake sidecar that speaks the same
//! JSON protocol (no model, no audio). Requires /usr/bin/python3.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jarvis_core::voice::neural::{NeuralConfig, NeuralError, NeuralEvent, NeuralTts};
use tokio::sync::broadcast::Receiver;

const FAKE: &str = r#"
import json, sys, threading, time
args = sys.argv
mode = args[args.index("--voice") + 1]
def emit(**e):
    sys.stdout.write(json.dumps(e) + "\n"); sys.stdout.flush()
if "hang" in mode:
    time.sleep(30)
if "fatal" in mode:
    emit(event="fatal", message="modelo corrupto"); sys.exit(1)
emit(event="ready", load_ms=5)
current = {"id": None}
def play(uid):
    time.sleep(0.05)
    if current["id"] != uid: return
    emit(event="first_audio", id=uid, ms=50)
    time.sleep(0.15)
    if current["id"] == uid:
        emit(event="finished", id=uid, underruns=0, underrun_ms=0, audio_ms=150); current["id"] = None
for line in sys.stdin:
    cmd = json.loads(line)
    if cmd["cmd"] == "speak":
        if current["id"] is not None: emit(event="stopped", id=current["id"])
        if "MUERE" in cmd["text"]: sys.exit(3)
        if "FALLA" in cmd["text"]:
            emit(event="error", id=cmd["id"], message="fallo del motor"); continue
        current["id"] = cmd["id"]
        threading.Thread(target=play, args=(cmd["id"],), daemon=True).start()
    elif cmd["cmd"] == "stop":
        if current["id"] is not None: emit(event="stopped", id=current["id"]); current["id"] = None
    elif cmd["cmd"] == "shutdown":
        break
"#;

fn setup(mode: &str) -> (Arc<NeuralTts>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("jarvis-neural-{}-{mode}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(dir.join("hf/hub/models--fake--model")).unwrap();
    std::fs::write(dir.join("server.py"), FAKE).unwrap();
    let voice = dir.join(format!("{mode}.wav"));
    std::fs::write(&voice, b"").unwrap();
    std::fs::write(voice.with_extension("txt"), "hola").unwrap();
    let config = NeuralConfig {
        python: "/usr/bin/python3".into(),
        script: dir.join("server.py"),
        voice,
        model_repo: "fake/model".into(),
        hf_home: dir.join("hf"),
        gain: 0.0,
        ready_timeout: Duration::from_secs(5),
    };
    (NeuralTts::new(config), dir)
}

/// Unique per test even when tests start in the same microsecond (the macOS
/// clock has microsecond resolution, so a timestamp alone collided and two
/// tests shared, then deleted, the same directory).
fn rand_suffix() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("{t}-{n}")
}

async fn next_for(rx: &mut Receiver<NeuralEvent>, wanted: impl Fn(&NeuralEvent) -> bool) -> NeuralEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ev = rx.recv().await.expect("canal abierto");
            if wanted(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("evento esperado")
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn availability_reports_what_is_missing() {
    let (tts, dir) = setup("ok");
    assert!(tts.config().availability().is_ok());
    let mut c = tts.config().clone();
    c.model_repo = "otro/modelo".into();
    assert!(c.availability().unwrap_err().contains("falta el modelo otro/modelo"));
    let mut c = tts.config().clone();
    c.python = "/nonexistent/python".into();
    assert!(c.availability().unwrap_err().contains("entorno Python"));
    let mut c = tts.config().clone();
    c.voice = dir.join("nope.wav");
    assert!(c.availability().unwrap_err().contains("voz de referencia"));
    cleanup(&dir);
}

#[tokio::test]
async fn speak_reports_first_audio_then_finished() {
    let (tts, dir) = setup("ok");
    let mut rx = tts.subscribe();
    tts.speak(1, "hola").await.unwrap();
    assert_eq!(next_for(&mut rx, |e| matches!(e, NeuralEvent::FirstAudio { .. })).await, NeuralEvent::FirstAudio { id: 1, ms: 50 });
    assert!(matches!(next_for(&mut rx, |e| matches!(e, NeuralEvent::Finished { .. })).await, NeuralEvent::Finished { id: 1, .. }));
    tts.shutdown().await;
    cleanup(&dir);
}

#[tokio::test]
async fn stop_interrupts_and_new_speak_replaces_previous() {
    let (tts, dir) = setup("ok");
    let mut rx = tts.subscribe();
    tts.speak(1, "uno").await.unwrap();
    tts.stop().await;
    assert_eq!(next_for(&mut rx, |e| matches!(e, NeuralEvent::Stopped { .. })).await, NeuralEvent::Stopped { id: 1 });
    tts.speak(2, "dos").await.unwrap();
    tts.speak(3, "tres").await.unwrap();
    assert_eq!(next_for(&mut rx, |e| matches!(e, NeuralEvent::Stopped { .. })).await, NeuralEvent::Stopped { id: 2 });
    let fin = next_for(&mut rx, |e| matches!(e, NeuralEvent::Finished { .. })).await;
    assert!(matches!(fin, NeuralEvent::Finished { id: 3, .. }), "{fin:?}");
    tts.shutdown().await;
    cleanup(&dir);
}

#[tokio::test]
async fn engine_error_is_reported_and_process_survives() {
    let (tts, dir) = setup("ok");
    let mut rx = tts.subscribe();
    tts.speak(4, "FALLA").await.unwrap();
    let err = next_for(&mut rx, |e| matches!(e, NeuralEvent::Error { .. })).await;
    assert_eq!(err, NeuralEvent::Error { id: Some(4), message: "fallo del motor".into() });
    assert!(tts.is_running().await);
    tts.shutdown().await;
    cleanup(&dir);
}

#[tokio::test]
async fn crash_is_reported_and_next_speak_restarts() {
    let (tts, dir) = setup("ok");
    let mut rx = tts.subscribe();
    tts.speak(5, "MUERE").await.unwrap();
    assert!(matches!(next_for(&mut rx, |e| matches!(e, NeuralEvent::Exited { .. })).await, NeuralEvent::Exited { .. }));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!tts.is_running().await);
    tts.speak(6, "otra vez").await.unwrap();
    assert!(matches!(next_for(&mut rx, |e| matches!(e, NeuralEvent::Finished { .. })).await, NeuralEvent::Finished { id: 6, .. }));
    tts.shutdown().await;
    cleanup(&dir);
}

#[tokio::test]
async fn fatal_start_and_timeout_are_errors_not_hangs() {
    let (tts, dir) = setup("fatal");
    match tts.speak(1, "hola").await {
        Err(NeuralError::Exited { message }) => assert!(message.contains("modelo corrupto"), "{message}"),
        other => panic!("{other:?}"),
    }
    cleanup(&dir);

    let (tts, dir) = setup("hang");
    let mut c = tts.config().clone();
    c.ready_timeout = Duration::from_millis(500);
    let tts = NeuralTts::new(c);
    let started = std::time::Instant::now();
    assert_eq!(tts.speak(1, "hola").await, Err(NeuralError::StartTimeout { secs: 0 }));
    assert!(started.elapsed() < Duration::from_secs(3));
    cleanup(&dir);
}

#[tokio::test]
async fn unavailable_engine_never_spawns() {
    let (tts, dir) = setup("ok");
    let mut c = tts.config().clone();
    c.python = "/nonexistent/python".into();
    let tts = NeuralTts::new(c);
    assert!(matches!(tts.speak(1, "hola").await, Err(NeuralError::Unavailable { .. })));
    assert!(!tts.is_running().await);
    cleanup(&dir);
}

/// Idle unload followed by a new answer: the engine reloads on demand.
#[tokio::test]
async fn shutdown_frees_the_engine_and_next_speak_reloads_it() {
    let (tts, dir) = setup("ok");
    let mut rx = tts.subscribe();
    tts.ensure_started().await.unwrap();
    assert!(tts.is_running().await);
    tts.shutdown().await;
    assert!(!tts.is_running().await);
    tts.speak(7, "de nuevo").await.unwrap();
    assert!(tts.is_running().await);
    assert!(matches!(next_for(&mut rx, |e| matches!(e, NeuralEvent::Finished { .. })).await, NeuralEvent::Finished { id: 7, .. }));
    tts.shutdown().await;
    cleanup(&dir);
}
