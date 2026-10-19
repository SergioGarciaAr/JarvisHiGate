//! Speech synthesis with macOS `say`.
//!
//! - Fixed executable (`/usr/bin/say`) and fixed flags; the text goes through
//!   stdin (`-f -`), so it can never be read as an option.
//! - The voice must be one of the installed voices: `say` silently falls back
//!   to the default voice for unknown names (verified on macOS 27).
//! - Each utterance is a child process that is killed on stop.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::diagnostics::sysexec::{FixedCommand, ProbeError, run_fixed};

pub const SAY: &str = "/usr/bin/say";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Voice {
    pub name: String,
    pub locale: String,
    pub sample: String,
}

/// Older Eloquence voices, noticeably more robotic than the Vocalizer ones.
const ELOQUENCE: [&str; 8] = ["Eddy", "Flo", "Grandma", "Grandpa", "Reed", "Rocko", "Sandy", "Shelley"];

impl Voice {
    pub fn is_spanish(&self) -> bool {
        self.locale.starts_with("es_")
    }

    /// Name without the language suffix: "Mónica (Español (España))" → "Mónica".
    /// macOS has listed the same voice both ways, so names are matched by base.
    pub fn base_name(&self) -> &str {
        self.name.split(" (").next().unwrap_or(&self.name)
    }

    pub fn is_eloquence(&self) -> bool {
        ELOQUENCE.contains(&self.base_name())
    }
}

/// Parses `say -v ?`, e.g. `Eddy (Español (España)) es_ES    # ¡Hola! Me llamo Eddy.`
/// (names may contain spaces and parentheses). Duplicate names are dropped:
/// `say -v` selects by name, so they cannot be told apart anyway.
pub fn parse_voices(out: &str) -> Vec<Voice> {
    let mut seen = std::collections::HashSet::new();
    out.lines()
        .filter_map(|line| {
            let (head, sample) = line.split_once('#').unwrap_or((line, ""));
            let head = head.trim_end();
            let (name, locale) = head.rsplit_once(char::is_whitespace)?;
            let valid_locale = locale.len() >= 4 && locale.contains('_') && locale.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            let name = name.trim();
            (valid_locale && !name.is_empty()).then(|| Voice { name: name.to_string(), locale: locale.to_string(), sample: sample.trim().to_string() })
        })
        .filter(|v| seen.insert(v.name.clone()))
        .collect()
}

pub async fn list_voices() -> Result<Vec<Voice>, ProbeError> {
    let out = run_fixed(FixedCommand::SayListVoices, Duration::from_secs(5)).await?.into_stdout()?;
    Ok(parse_voices(&out))
}

/// Preferred Spanish voice: Latin American Paulina, then Mónica, then any
/// non-Eloquence Spanish voice, and only then an Eloquence one.
pub fn default_spanish(voices: &[Voice]) -> Option<&Voice> {
    let by_base = |base: &str, locale: &str| voices.iter().find(|v| v.base_name() == base && v.locale == locale);
    by_base("Paulina", "es_MX")
        .or_else(|| by_base("Mónica", "es_ES"))
        .or_else(|| voices.iter().find(|v| v.is_spanish() && !v.is_eloquence()))
        .or_else(|| voices.iter().find(|v| v.is_spanish()))
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TtsError {
    #[error("la voz «{voice}» no está instalada")]
    UnknownVoice { voice: String },
    #[error("no hay ninguna voz instalada")]
    NoVoices,
    #[error("no hay texto que leer")]
    Empty,
    #[error("respuesta antigua: solo se lee la respuesta de la petición más reciente")]
    Stale,
    #[error("no se pudo iniciar la síntesis de voz: {0}")]
    Spawn(ProbeError),
    #[error("la síntesis de voz falló (código {code:?}): {stderr}")]
    Failed { code: Option<i32>, stderr: String },
}

/// Validates the requested voice against the installed list.
pub fn resolve_voice<'a>(requested: Option<&str>, voices: &'a [Voice]) -> Result<&'a Voice, TtsError> {
    match requested {
        Some(name) => voices.iter().find(|v| v.name == name).ok_or_else(|| TtsError::UnknownVoice { voice: name.into() }),
        None => default_spanish(voices).or_else(|| voices.first()).ok_or(TtsError::NoVoices),
    }
}

/// Only the answer of the most recent request may be spoken.
pub fn check_latest(latest: Option<&str>, request_id: &str) -> Result<(), TtsError> {
    if latest == Some(request_id) { Ok(()) } else { Err(TtsError::Stale) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechEnd {
    Finished,
    Stopped,
}

pub(crate) enum Output {
    Speakers,
    /// Render to an audio file instead of playing; used by tests and the
    /// listening-test tool so they make no sound.
    File(PathBuf),
}

pub(crate) async fn run_say(voice: &Voice, text: &str, output: Output, cancel: CancellationToken) -> Result<SpeechEnd, TtsError> {
    if text.trim().is_empty() {
        return Err(TtsError::Empty);
    }
    let mut args: Vec<String> = vec!["-v".into(), voice.name.clone(), "-f".into(), "-".into()];
    if let Output::File(path) = &output {
        args.push("-o".into());
        args.push(path.to_string_lossy().into_owned());
    }
    let mut child = tokio::process::Command::new(SAY)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            TtsError::Spawn(match e.kind() {
                std::io::ErrorKind::NotFound => ProbeError::NotFound { path: SAY.into() },
                std::io::ErrorKind::PermissionDenied => ProbeError::PermissionDenied { path: SAY.into() },
                _ => ProbeError::Io { message: e.to_string() },
            })
        })?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let body = text.to_string();
    let run = async {
        // Closing stdin tells `say` the text is complete.
        let _ = stdin.write_all(body.as_bytes()).await;
        drop(stdin);
        child.wait().await
    };
    let status = tokio::select! {
        _ = cancel.cancelled() => None,
        s = run => Some(s),
    };
    match status {
        None => {
            let _ = child.kill().await;
            Ok(SpeechEnd::Stopped)
        }
        Some(Ok(s)) if s.success() => Ok(SpeechEnd::Finished),
        Some(Ok(s)) => {
            let mut err = String::new();
            let _ = stderr.read_to_string(&mut err).await;
            Err(TtsError::Failed { code: s.code(), stderr: err.trim().to_string() })
        }
        Some(Err(e)) => Err(TtsError::Spawn(ProbeError::Io { message: e.to_string() })),
    }
}

/// Renders to an audio file instead of the speakers. Development tooling
/// only (listening tests); the app never calls it.
#[doc(hidden)]
pub async fn render_to_file(voice: &Voice, text: &str, path: PathBuf) -> Result<SpeechEnd, TtsError> {
    run_say(voice, text, Output::File(path), CancellationToken::new()).await
}

/// Plays `text` through the speakers until it ends or `cancel` fires.
pub async fn speak(voice: &Voice, text: &str, cancel: CancellationToken) -> Result<SpeechEnd, TtsError> {
    run_say(voice, text, Output::Speakers, cancel).await
}

/// Tracks the single active utterance. Starting a new one stops the previous.
#[derive(Default)]
pub struct Speaker {
    current: Mutex<Option<(u64, CancellationToken)>>,
    next: AtomicU64,
}

impl Speaker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops the current utterance (if any) and registers a new one.
    pub fn begin(&self) -> (u64, CancellationToken) {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let token = CancellationToken::new();
        let previous = self.current.lock().expect("speaker lock").replace((id, token.clone()));
        if let Some((_, old)) = previous {
            old.cancel();
        }
        (id, token)
    }

    /// Clears the utterance if it is still the current one.
    pub fn finish(&self, id: u64) {
        let mut cur = self.current.lock().expect("speaker lock");
        if cur.as_ref().is_some_and(|(c, _)| *c == id) {
            *cur = None;
        }
    }

    /// Stops the current utterance; returns its id.
    pub fn stop(&self) -> Option<u64> {
        let taken = self.current.lock().expect("speaker lock").take();
        taken.map(|(id, token)| {
            token.cancel();
            id
        })
    }

    pub fn current(&self) -> Option<u64> {
        self.current.lock().expect("speaker lock").as_ref().map(|(id, _)| *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAY_OUTPUT: &str = "Eddy (Español (España)) es_ES    # ¡Hola! Me llamo Eddy.
Mónica              es_ES    # ¡Hola! Me llamo Mónica.
Paulina             es_MX    # ¡Hola! Me llamo Paulina.
Samantha            en_US    # Hello! My name is Samantha.
";

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jarvis-tts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn parses_real_voice_list() {
        let v = parse_voices(SAY_OUTPUT);
        assert_eq!(v.len(), 4);
        assert_eq!(v[0].name, "Eddy (Español (España))");
        assert_eq!(v[0].locale, "es_ES");
        assert_eq!(v[1].name, "Mónica");
        assert!(v[2].is_spanish() && !v[3].is_spanish());
        assert!(parse_voices("garbage\n\n").is_empty());
    }

    /// Output format seen later in the same session (macOS renamed the voices).
    const SAY_OUTPUT_RENAMED: &str = "Eddy (Español (España)) es_ES    # ¡Hola! Me llamo Eddy.
Mónica (Español (España)) es_ES    # ¡Hola! Me llamo Mónica.
Mónica (Español (España)) es_ES    # ¡Hola! Me llamo Mónica.
Eddy (Español (México)) es_MX    # ¡Hola! Me llamo Eddy.
Paulina (Español (México)) es_MX    # ¡Hola! Me llamo Paulina.
";

    #[test]
    fn default_never_falls_back_to_eloquence_when_renamed() {
        let v = parse_voices(SAY_OUTPUT_RENAMED);
        assert_eq!(v.len(), 4, "the duplicated Mónica entry is listed once");
        let d = default_spanish(&v).unwrap();
        assert_eq!(d.base_name(), "Paulina");
        assert!(!d.is_eloquence());
        let es_only: Vec<Voice> = v.iter().filter(|x| x.locale == "es_ES").cloned().collect();
        assert_eq!(default_spanish(&es_only).unwrap().base_name(), "Mónica");
        assert!(v[0].is_eloquence());
    }

    #[test]
    fn spanish_default_and_validation() {
        let v = parse_voices(SAY_OUTPUT);
        assert_eq!(default_spanish(&v).unwrap().name, "Paulina");
        assert_eq!(resolve_voice(None, &v).unwrap().name, "Paulina");
        assert_eq!(resolve_voice(Some("Mónica"), &v).unwrap().locale, "es_ES");
        assert_eq!(resolve_voice(Some("Paulina"), &v).unwrap().locale, "es_MX");
        assert_eq!(resolve_voice(Some("-o"), &v).unwrap_err(), TtsError::UnknownVoice { voice: "-o".into() });
        assert_eq!(resolve_voice(None, &[]).unwrap_err(), TtsError::NoVoices);
    }

    #[test]
    fn stale_answers_are_rejected() {
        assert!(check_latest(Some("b"), "b").is_ok());
        assert_eq!(check_latest(Some("b"), "a").unwrap_err(), TtsError::Stale);
        assert_eq!(check_latest(None, "a").unwrap_err(), TtsError::Stale);
    }

    #[test]
    fn speaker_tracks_a_single_utterance() {
        let s = Speaker::new();
        let (a, ta) = s.begin();
        let (b, _tb) = s.begin();
        assert!(ta.is_cancelled(), "a new utterance stops the previous one");
        assert_eq!(s.current(), Some(b));
        s.finish(a); // stale finish does not clear the current one
        assert_eq!(s.current(), Some(b));
        assert_eq!(s.stop(), Some(b));
        assert_eq!(s.current(), None);
        assert_eq!(s.stop(), None);
    }

    // The following run the real `say` binary but render to a file: no sound.

    #[tokio::test]
    async fn real_say_renders_spanish_voice_to_file() {
        let voices = list_voices().await.unwrap();
        let voice = default_spanish(&voices).expect("hay voces en español en este Mac");
        assert!(!voice.is_eloquence(), "default on this Mac must not be an Eloquence voice: {}", voice.name);
        let out = tmp("ok.aiff");
        let r = run_say(voice, "Hola, soy Jarvis.", Output::File(out.clone()), CancellationToken::new()).await;
        assert_eq!(r, Ok(SpeechEnd::Finished));
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
        std::fs::remove_file(out).unwrap();
    }

    #[tokio::test]
    async fn real_say_text_starting_with_dash_is_spoken_not_parsed() {
        let voices = list_voices().await.unwrap();
        let out = tmp("dash.aiff");
        let r = run_say(&voices[0], "-o /tmp/evil.aiff", Output::File(out.clone()), CancellationToken::new()).await;
        assert_eq!(r, Ok(SpeechEnd::Finished));
        assert!(!std::path::Path::new("/tmp/evil.aiff").exists());
        std::fs::remove_file(out).unwrap();
    }

    #[tokio::test]
    async fn real_say_failure_is_reported() {
        let voices = list_voices().await.unwrap();
        let r = run_say(&voices[0], "hola", Output::File("/nonexistent/dir/x.aiff".into()), CancellationToken::new()).await;
        match r {
            Err(TtsError::Failed { code: Some(1), stderr }) => assert!(stderr.contains("Opening output file failed"), "{stderr}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn real_say_stops_immediately() {
        let voices = list_voices().await.unwrap();
        let out = tmp("long.aiff");
        let token = CancellationToken::new();
        let t2 = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            t2.cancel();
        });
        let started = std::time::Instant::now();
        let long = "Esta frase se repite para que la síntesis tarde bastante. ".repeat(400);
        let r = run_say(&voices[0], &long, Output::File(out.clone()), token).await;
        assert_eq!(r, Ok(SpeechEnd::Stopped));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let _ = std::fs::remove_file(out);
    }

    fn duration_secs(path: &std::path::Path) -> f64 {
        // AIFF from `say` is 16-bit mono PCM: duration = data bytes / (2 * rate).
        let out = std::process::Command::new("/usr/bin/afinfo").env("LC_ALL", "C").arg(path).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find_map(|l| l.strip_prefix("estimated duration: ").and_then(|v| v.split_whitespace().next()?.parse().ok()))
            .expect("afinfo duration")
    }

    /// A long prepared answer reaches `say` whole through stdin: the rendered
    /// audio covers the conclusion (no 4000-character cut, no inserted gaps).
    #[tokio::test]
    async fn real_say_renders_long_prepared_answer_whole() {
        let voices = list_voices().await.unwrap();
        let voice = default_spanish(&voices).unwrap();
        let short = tmp("short.aiff");
        let long = tmp("long-answer.aiff");
        let unit = "La memoria y el disco funcionan con normalidad. ";
        let text = crate::voice::prosody::prepare(&format!("{}

Conclusión: todo está en orden.", unit.repeat(90)));
        assert!(text.chars().count() > 4000);
        run_say(voice, unit, Output::File(short.clone()), CancellationToken::new()).await.unwrap();
        run_say(voice, &text, Output::File(long.clone()), CancellationToken::new()).await.unwrap();
        let (one, all) = (duration_secs(&short), duration_secs(&long));
        assert!(all > one * 80.0, "one sentence {one:.2}s, whole answer {all:.2}s");
        std::fs::remove_file(short).unwrap();
        std::fs::remove_file(long).unwrap();
    }

    #[tokio::test]
    async fn empty_text_is_rejected_without_spawning() {
        let v = Voice { name: "Mónica".into(), locale: "es_ES".into(), sample: String::new() };
        assert_eq!(run_say(&v, "  \n", Output::Speakers, CancellationToken::new()).await, Err(TtsError::Empty));
    }
}
