//! Tauri shell: thin IPC layer over `jarvis-core`.

mod settings;
mod shortcut;
mod voice_engine;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jarvis_core::agent::{Limits, LlmAskTool, PolicyEngine, ThinkMode, ThinkRequest, ToolRegistry, run_think};
use jarvis_core::command::{self, Availability, Command, DiagnosticoScope, ParseErrorInfo, PiensaMode};
use jarvis_core::diagnostics::{self, probe::MacProbe};
use jarvis_core::events::{EventSink, ProgressEvent, Reporter};
use jarvis_core::llm::{ChatRequest, ChatSession, Finish, ModelInfo, OllamaClient, OllamaErrorInfo, OllamaHealth};
use jarvis_core::router::{Route, route_input};
use jarvis_core::telemetry::TelemetryService;
use jarvis_core::voice::stt::{self, SttStatus};
use jarvis_core::voice::prosody;
use jarvis_core::voice::tts::{self, Speaker, SpeechEnd, Voice};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio_util::sync::CancellationToken;

use settings::Settings;
use shortcut::{QUICK_LABEL, ShortcutStatus};

struct AppState {
    ollama: OllamaClient,
    registry: Arc<ToolRegistry>,
    chat: Arc<Mutex<ChatSession>>,
    active: Arc<Mutex<HashMap<String, CancellationToken>>>,
    models: Mutex<Vec<ModelInfo>>,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    shortcut: Mutex<ShortcutStatus>,
    /// Shared by the periodic sampler and the diagnostics.
    telemetry: Arc<Mutex<TelemetryService>>,
    speaker: Arc<Speaker>,
    /// Installed `say` voices, loaded on first use.
    voices: Mutex<Option<Vec<Voice>>>,
    /// Most recent submitted request; only its answer may be spoken.
    latest_request: Mutex<Option<String>>,
    models_dir: PathBuf,
    neural: Arc<voice_engine::NeuralHost>,
}

impl AppState {
    /// Engine in use: the saved choice, else neural when it is installed.
    fn engine(&self) -> &'static str {
        match self.settings.lock().unwrap().tts_engine.as_deref() {
            Some("say") => "say",
            Some("neural") => "neural",
            _ if self.neural.info().available => "neural",
            _ => "say",
        }
    }

    /// Stops any speech from either engine.
    fn stop_speech(&self) -> bool {
        let said = self.speaker.stop().is_some();
        let neural = self.neural.stop();
        said || neural
    }
}

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: ProgressEvent) {
        let _ = self.0.emit("jarvis://progress", event);
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChatEvent {
    Delta { request_id: String, text: String },
    Done { request_id: String },
    Cancelled { request_id: String },
    Error { request_id: String, error: OllamaErrorInfo },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SubmitOutcome {
    Empty,
    Help { text: String },
    ParseError { error: ParseErrorInfo },
    ChatStarted { request_id: String, model: String },
    RunStarted { request_id: String, goal: String, mode: ThinkMode, model: String },
    DiagnosticStarted { request_id: String, scope: DiagnosticoScope },
    NotAvailable { command: Command, phase: u8, reason: String, alternative: String },
    NoModel { message: String, recovery: String },
}

impl AppState {
    /// Explicit model, else the saved one, else the first installed.
    fn resolve_model(&self, requested: Option<String>) -> Option<(String, Option<bool>)> {
        let models = self.models.lock().unwrap();
        let name = requested
            .or_else(|| self.settings.lock().unwrap().model.clone())
            .or_else(|| models.first().map(|m| m.name.clone()))?;
        // Disable "thinking" output on models that support it: the UI shows
        // answers and summaries, not the model's internal reasoning.
        let think = models.iter().find(|m| m.name == name).filter(|m| m.supports_thinking()).map(|_| false);
        Some((name, think))
    }

    fn register_token(&self, id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.active.lock().unwrap().insert(id.to_string(), token.clone());
        token
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum VoiceEvent {
    Started { utterance: u64, request_id: String },
    Finished { utterance: u64, request_id: String },
    Stopped { utterance: u64, request_id: String },
    Error { utterance: u64, request_id: String, message: String },
}

#[derive(Serialize)]
struct TtsStatus {
    available: bool,
    voices: Vec<Voice>,
    selected: Option<String>,
    reason: Option<String>,
}

#[derive(Serialize)]
struct VoiceStatus {
    tts: TtsStatus,
    stt: SttStatus,
    autospeak: bool,
    engine: &'static str,
    engines: Vec<voice_engine::EngineInfo>,
    neural_voices: Vec<String>,
    neural_voice: String,
}

async fn installed_voices(state: &AppState) -> Result<Vec<Voice>, String> {
    if let Some(v) = state.voices.lock().unwrap().clone() {
        return Ok(v);
    }
    let voices = tts::list_voices().await.map_err(|e| format!("No se pudieron consultar las voces de macOS: {e}"))?;
    *state.voices.lock().unwrap() = Some(voices.clone());
    Ok(voices)
}

#[tauri::command]
async fn voice_status(state: State<'_, AppState>) -> Result<VoiceStatus, ()> {
    let (saved_voice, autospeak) = {
        let s = state.settings.lock().unwrap();
        (s.voice.clone(), s.autospeak)
    };
    let tts = match installed_voices(&state).await {
        Ok(voices) => {
            let selected = tts::resolve_voice(saved_voice.as_deref(), &voices)
                .or_else(|_| tts::resolve_voice(None, &voices))
                .ok()
                .map(|v| v.name.clone());
            let reason = selected.is_none().then(|| "No hay voces instaladas en macOS.".to_string());
            TtsStatus { available: selected.is_some(), voices, selected, reason }
        }
        Err(e) => TtsStatus { available: false, voices: vec![], selected: None, reason: Some(e) },
    };
    let stt = stt::detect(&state.models_dir, &stt::WHISPER_CLI_CANDIDATES);
    let say_info = voice_engine::EngineInfo { id: "say", label: "Sistema · say (recuperación)", available: tts.available, reason: tts.reason.clone() };
    Ok(VoiceStatus {
        tts,
        stt,
        autospeak,
        engine: state.engine(),
        engines: vec![state.neural.info(), say_info],
        neural_voices: state.neural.voices(),
        neural_voice: state.neural.current_voice(),
    })
}

#[tauri::command]
fn voice_set_engine(state: State<'_, AppState>, engine: String) -> Result<(), String> {
    if engine != "neural" && engine != "say" {
        return Err(format!("motor desconocido: {engine}"));
    }
    state.stop_speech();
    let mut s = state.settings.lock().unwrap();
    // Load the model only if answers are read aloud; otherwise on first use.
    if engine == "neural" && s.autospeak {
        state.neural.preload();
    }
    s.tts_engine = Some(engine);
    s.save(&state.settings_path).map_err(|e| format!("No se pudieron guardar los ajustes: {e}"))
}

#[tauri::command]
fn voice_set_neural_voice(app: AppHandle, state: State<'_, AppState>, name: String) -> Result<(), String> {
    let preload = state.settings.lock().unwrap().autospeak;
    state.neural.set_voice(&app, &name, preload)?;
    let mut s = state.settings.lock().unwrap();
    s.neural_voice = Some(name);
    s.save(&state.settings_path).map_err(|e| format!("No se pudieron guardar los ajustes: {e}"))
}

#[tauri::command]
async fn voice_set_voice(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let voices = installed_voices(&state).await?;
    tts::resolve_voice(Some(&name), &voices).map_err(|e| e.to_string())?;
    let mut s = state.settings.lock().unwrap();
    s.voice = Some(name);
    s.save(&state.settings_path).map_err(|e| format!("No se pudieron guardar los ajustes: {e}"))
}

#[tauri::command]
fn voice_set_autospeak(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    if enabled && state.engine() == "neural" {
        state.neural.preload();
    }
    let mut s = state.settings.lock().unwrap();
    s.autospeak = enabled;
    s.save(&state.settings_path).map_err(|e| format!("No se pudieron guardar los ajustes: {e}"))
}

/// Speaks the answer of `request_id`. Rejected unless it is the most recent
/// request, so a cancelled or superseded answer is never read aloud.
#[tauri::command]
async fn voice_speak(app: AppHandle, state: State<'_, AppState>, request_id: String, text: String) -> Result<u64, String> {
    tts::check_latest(state.latest_request.lock().unwrap().as_deref(), &request_id).map_err(|e| e.to_string())?;
    let voices = installed_voices(&state).await?;
    let saved = state.settings.lock().unwrap().voice.clone();
    let voice = tts::resolve_voice(saved.as_deref(), &voices)
        .or_else(|_| tts::resolve_voice(None, &voices))
        .map_err(|e| e.to_string())?
        .clone();
    let script = prosody::prepare(&text);
    if script.is_empty() {
        return Err(tts::TtsError::Empty.to_string());
    }
    if state.engine() == "neural" {
        state.speaker.stop();
        return Ok(state.neural.speak(&app, request_id, script));
    }
    state.neural.stop();
    let (utterance, token) = state.speaker.begin();
    let speaker = state.speaker.clone();
    let _ = app.emit("jarvis://voice", VoiceEvent::Started { utterance, request_id: request_id.clone() });
    tauri::async_runtime::spawn(async move {
        let event = match tts::speak(&voice, &script, token).await {
            Ok(SpeechEnd::Finished) => VoiceEvent::Finished { utterance, request_id },
            Ok(SpeechEnd::Stopped) => VoiceEvent::Stopped { utterance, request_id },
            Err(e) => VoiceEvent::Error { utterance, request_id, message: e.to_string() },
        };
        speaker.finish(utterance);
        let _ = app.emit("jarvis://voice", event);
    });
    Ok(utterance)
}

#[tauri::command]
fn voice_stop(state: State<'_, AppState>) -> bool {
    state.stop_speech()
}

#[tauri::command]
async fn ollama_health(state: State<'_, AppState>) -> Result<OllamaHealth, ()> {
    let health = state.ollama.health().await;
    if let OllamaHealth::Available { models, .. } = &health {
        *state.models.lock().unwrap() = models.clone();
    } else {
        state.models.lock().unwrap().clear();
    }
    Ok(health)
}

#[tauri::command]
fn submit_input(
    app: AppHandle,
    state: State<'_, AppState>,
    request_id: String,
    text: String,
    model: Option<String>,
) -> SubmitOutcome {
    // A new request supersedes any answer still being read aloud.
    if !matches!(route_input(&text), Route::Empty) {
        *state.latest_request.lock().unwrap() = Some(request_id.clone());
        state.stop_speech();
    }
    let route = route_input(&text);
    let (goal, mode) = match route {
        Route::Empty => return SubmitOutcome::Empty,
        Route::CommandError { error } => return SubmitOutcome::ParseError { error },
        Route::Command { command: Command::Ayuda { topic } } => {
            return SubmitOutcome::Help { text: command::help_text(topic) };
        }
        Route::Command { command } => match command::availability(&command) {
            Availability::NotYet { phase, reason, alternative } => {
                return SubmitOutcome::NotAvailable { command, phase, reason, alternative };
            }
            Availability::Ready => match command {
                Command::Piensa { mode, prompt } => (
                    prompt,
                    match mode {
                        PiensaMode::Plan => ThinkMode::Plan,
                        PiensaMode::Comparar => ThinkMode::Comparar,
                    },
                ),
                Command::Diagnostico { scope } => return start_diagnostic(app, &state, request_id, scope),
                other => unreachable!("availability marks {other:?} as not ready"),
            },
        },
        Route::Agent { goal } => (goal, ThinkMode::Analisis),
        Route::Chat { text } => return start_chat(app, &state, request_id, text, model),
    };

    let Some((model, think)) = state.resolve_model(model) else {
        return no_model();
    };
    let token = state.register_token(&request_id);
    let registry = state.registry.clone();
    let active = state.active.clone();
    let reporter = Reporter::new(request_id.clone(), Arc::new(TauriSink(app)));
    let req = ThinkRequest { mode, prompt: goal.clone(), model: model.clone(), think };
    let id = request_id.clone();
    tauri::async_runtime::spawn(async move {
        run_think(&registry, req, &reporter, Limits::default(), token).await;
        active.lock().unwrap().remove(&id);
    });
    SubmitOutcome::RunStarted { request_id, goal, mode, model }
}

/// Diagnostics do not use the model, so they work even when Ollama is down.
fn start_diagnostic(app: AppHandle, state: &AppState, request_id: String, scope: DiagnosticoScope) -> SubmitOutcome {
    let token = state.register_token(&request_id);
    let telemetry = state.telemetry.clone();
    let active = state.active.clone();
    let reporter = Reporter::new(request_id.clone(), Arc::new(TauriSink(app)));
    let id = request_id.clone();
    tauri::async_runtime::spawn(async move {
        let probe = MacProbe::default();
        diagnostics::run_diagnostic(scope, &probe, telemetry, &reporter, diagnostics::default_limits(), token).await;
        active.lock().unwrap().remove(&id);
    });
    SubmitOutcome::DiagnosticStarted { request_id, scope }
}

fn no_model() -> SubmitOutcome {
    SubmitOutcome::NoModel {
        message: "No hay ningún modelo de Ollama disponible.".into(),
        recovery: "Comprueba que Ollama está en ejecución y que tienes un modelo instalado (`ollama pull <modelo>`), luego pulsa Reintentar.".into(),
    }
}

fn start_chat(app: AppHandle, state: &AppState, request_id: String, text: String, model: Option<String>) -> SubmitOutcome {
    let Some((model, think)) = state.resolve_model(model) else {
        return no_model();
    };
    let token = state.register_token(&request_id);
    let client = state.ollama.clone();
    let chat = state.chat.clone();
    let active = state.active.clone();
    let messages = chat.lock().unwrap().request_messages(&text);
    let id = request_id.clone();
    let model_name = model.clone();
    tauri::async_runtime::spawn(async move {
        let req = ChatRequest { model: &model_name, messages: &messages, think, format: None };
        let result = client
            .chat_stream(&req, &token, |delta| {
                let _ = app.emit("jarvis://chat", ChatEvent::Delta { request_id: id.clone(), text: delta.to_string() });
            })
            .await;
        let event = match result {
            Ok(out) => {
                if !out.content.is_empty() {
                    chat.lock().unwrap().record_turn(&text, &out.content);
                }
                match out.finish {
                    Finish::Done => ChatEvent::Done { request_id: id.clone() },
                    Finish::Cancelled => ChatEvent::Cancelled { request_id: id.clone() },
                }
            }
            Err(e) => ChatEvent::Error { request_id: id.clone(), error: e.info() },
        };
        let _ = app.emit("jarvis://chat", event);
        active.lock().unwrap().remove(&id);
    });
    SubmitOutcome::ChatStarted { request_id, model }
}

#[tauri::command]
fn cancel_request(state: State<'_, AppState>, request_id: String) -> bool {
    match state.active.lock().unwrap().get(&request_id) {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

#[tauri::command]
fn reset_chat(state: State<'_, AppState>) {
    state.chat.lock().unwrap().clear();
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_model(state: State<'_, AppState>, model: String) -> Result<(), String> {
    let mut s = state.settings.lock().unwrap();
    s.model = Some(model);
    s.save(&state.settings_path).map_err(|e| format!("No se pudieron guardar los ajustes: {e}"))
}

#[tauri::command]
fn shortcut_status(state: State<'_, AppState>) -> ShortcutStatus {
    state.shortcut.lock().unwrap().clone()
}

#[tauri::command]
fn set_shortcut(app: AppHandle, state: State<'_, AppState>, shortcut: String) -> ShortcutStatus {
    let current = state.shortcut.lock().unwrap().clone();
    let previous = current.registered.then_some(current.shortcut.as_str());
    let status = shortcut::register(&app, &shortcut, previous);
    if status.registered {
        let mut s = state.settings.lock().unwrap();
        s.shortcut = shortcut;
        let _ = s.save(&state.settings_path);
        *state.shortcut.lock().unwrap() = status.clone();
    } else if let Some(prev) = previous {
        // Keep the previous shortcut working instead of leaving none.
        *state.shortcut.lock().unwrap() = shortcut::register(&app, prev, None);
    }
    status
}

#[tauri::command]
fn hide_quick_panel(app: AppHandle) {
    if let Some(w) = app.get_webview_window(QUICK_LABEL) {
        let _ = w.hide();
    }
}

#[tauri::command]
fn show_main(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn spawn_telemetry(app: AppHandle, telemetry: Arc<Mutex<TelemetryService>>) {
    std::thread::spawn(move || {
        loop {
            let snap = match telemetry.lock() {
                Ok(mut t) => t.snapshot(),
                Err(_) => break,
            };
            if app.emit("jarvis://telemetry", snap).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = Settings::load(&settings_path);
            let base_url = std::env::var("JARVIS_OLLAMA_URL")
                .unwrap_or_else(|_| jarvis_core::llm::ollama::DEFAULT_BASE_URL.to_string());
            let ollama = OllamaClient::new(base_url);
            let mut registry = ToolRegistry::new(PolicyEngine::default());
            registry.register(LlmAskTool::new(ollama.clone()));

            let shortcut_status = shortcut::register(app.handle(), &settings.shortcut, None);
            let telemetry = Arc::new(Mutex::new(TelemetryService::new()));
            let neural = voice_engine::NeuralHost::new(
                app.handle(),
                app.path().app_data_dir()?,
                app.path().home_dir()?,
                settings.neural_voice.as_deref().unwrap_or(voice_engine::DEFAULT_VOICE),
            );
            app.manage(AppState {
                ollama,
                registry: Arc::new(registry),
                chat: Arc::new(Mutex::new(ChatSession::default())),
                active: Arc::new(Mutex::new(HashMap::new())),
                models: Mutex::new(Vec::new()),
                settings: Mutex::new(settings),
                settings_path,
                shortcut: Mutex::new(shortcut_status),
                telemetry: telemetry.clone(),
                speaker: Arc::new(Speaker::new()),
                voices: Mutex::new(None),
                latest_request: Mutex::new(None),
                models_dir: app.path().app_data_dir()?.join("models"),
                neural: neural.clone(),
            });
            // ~3.5 GB: loaded at startup only if answers are read aloud.
            let state = app.state::<AppState>();
            if state.engine() == "neural" && state.settings.lock().unwrap().autospeak {
                neural.preload();
            }
            spawn_telemetry(app.handle().clone(), telemetry);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == QUICK_LABEL && matches!(event, WindowEvent::Focused(false)) {
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            ollama_health,
            submit_input,
            cancel_request,
            reset_chat,
            get_settings,
            set_model,
            shortcut_status,
            set_shortcut,
            hide_quick_panel,
            show_main,
            voice_status,
            voice_set_voice,
            voice_set_autospeak,
            voice_speak,
            voice_stop,
            voice_set_engine,
            voice_set_neural_voice,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // The sidecar also exits on stdin EOF; this makes shutdown orderly.
            if let tauri::RunEvent::Exit = event
                && let Some(state) = app.try_state::<AppState>()
            {
                tauri::async_runtime::block_on(state.neural.shutdown());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape consumed by `SubmitOutcome` in src/lib/types.ts.
    #[test]
    fn diagnostic_started_json_matches_frontend() {
        let v = serde_json::to_value(SubmitOutcome::DiagnosticStarted { request_id: "r".into(), scope: DiagnosticoScope::Ram }).unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "diagnostic_started", "request_id": "r", "scope": "ram" }));
        let v = serde_json::to_value(SubmitOutcome::DiagnosticStarted { request_id: "r".into(), scope: DiagnosticoScope::General }).unwrap();
        assert_eq!(v["scope"], "general");
    }

    #[test]
    fn progress_events_serialize_as_frontend_expects() {
        use jarvis_core::events::{EventKind, EventStatus, MemorySink};
        let sink = Arc::new(MemorySink::default());
        Reporter::new("r", sink.clone()).emit(EventKind::ToolCall, EventStatus::Running, "vm_stat", None);
        let v = serde_json::to_value(&sink.0.lock().unwrap()[0]).unwrap();
        for key in ["id", "run_id", "seq", "kind", "status", "timestamp", "summary"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(v["kind"], "tool_call");
        assert_eq!(v["status"], "running");
    }
}
