//! Real-engine validation of the neural TTS sidecar on this Mac. Uses the
//! real audio device with gain 0 (timing is real, nothing is audible).
//!
//!   cargo run -p jarvis-core --example neural_live_check -- <prepared.json>

use std::path::PathBuf;
use std::time::{Duration, Instant};

use jarvis_core::voice::neural::{NeuralConfig, NeuralEvent, NeuralTts};
use tokio::sync::broadcast::Receiver;

async fn wait(rx: &mut Receiver<NeuralEvent>, secs: u64, f: impl Fn(&NeuralEvent) -> bool) -> Option<NeuralEvent> {
    tokio::time::timeout(Duration::from_secs(secs), async {
        loop {
            match rx.recv().await {
                Ok(ev) if f(&ev) => return Some(ev),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

#[tokio::main]
async fn main() {
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let base = home.join("Library/Application Support/com.jarvis.desktop/voice-engine");
    let texts: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::env::args().nth(1).expect("prepared.json")).unwrap()).unwrap();
    let text = |k: &str| texts[k].as_str().unwrap().to_string();
    let config = NeuralConfig {
        python: base.join("mlx-venv/bin/python"),
        script: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../voice-engine/jarvis_tts/server.py")),
        voice: base.join("models/voices/jarvis-es-1.wav"),
        model_repo: "mlx-community/Qwen3-TTS-12Hz-1.7B-Base-8bit".into(),
        hf_home: home.join("Library/Caches/Jarvis/hf"),
        gain: 0.0,
        ready_timeout: Duration::from_secs(180),
    };
    let tts = NeuralTts::new(config);
    let mut rx = tts.subscribe();

    let t = Instant::now();
    tts.ensure_started().await.expect("arranque");
    println!("arranque + carga + calentamiento: {} ms", ms(t.elapsed()));

    // 1. Short answers: request → audio reaching the device.
    for (i, k) in ["breve", "pregunta", "advertencia"].iter().enumerate() {
        let id = 10 + i as u64;
        let t = Instant::now();
        tts.speak(id, &text(k)).await.unwrap();
        let first = wait(&mut rx, 30, |e| matches!(e, NeuralEvent::FirstAudio { id: x, .. } if *x == id)).await;
        let t_first = t.elapsed();
        let fin = wait(&mut rx, 60, |e| matches!(e, NeuralEvent::Finished { id: x, .. } if *x == id)).await;
        println!("{k:12} primer audio {} ms (sidecar {first:?}) · terminado {fin:?}", ms(t_first));
    }

    // 2. Cancel during generation (right after the request).
    let t = Instant::now();
    tts.speak(20, &text("pasaje")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let ts = Instant::now();
    tts.stop().await;
    let st = wait(&mut rx, 5, |e| matches!(e, NeuralEvent::Stopped { id: 20 })).await;
    println!("cancelar durante la generación: Stopped en {} ms ({st:?}), {} ms tras pedir", ms(ts.elapsed()), ms(t.elapsed()));
    let leak = wait(&mut rx, 2, |e| matches!(e, NeuralEvent::FirstAudio { id: 20, .. } | NeuralEvent::Finished { id: 20, .. })).await;
    println!("  audio tardío de la locución cancelada: {leak:?}");

    // 3. Cancel during playback.
    tts.speak(21, &text("pasaje")).await.unwrap();
    wait(&mut rx, 30, |e| matches!(e, NeuralEvent::FirstAudio { id: 21, .. })).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let ts = Instant::now();
    tts.stop().await;
    let st = wait(&mut rx, 5, |e| matches!(e, NeuralEvent::Stopped { id: 21 })).await;
    println!("cancelar durante la reproducción: Stopped en {} ms ({st:?})", ms(ts.elapsed()));

    // 4. A second answer while the first one plays.
    tts.speak(30, &text("explicacion")).await.unwrap();
    wait(&mut rx, 30, |e| matches!(e, NeuralEvent::FirstAudio { id: 30, .. })).await;
    let t = Instant::now();
    tts.speak(31, &text("breve")).await.unwrap();
    let st = wait(&mut rx, 5, |e| matches!(e, NeuralEvent::Stopped { id: 30 } | NeuralEvent::Error { .. })).await;
    let first = wait(&mut rx, 30, |e| matches!(e, NeuralEvent::FirstAudio { id: 31, .. } | NeuralEvent::Error { .. })).await;
    println!("segunda respuesta: anterior detenida {st:?}, nueva suena en {} ms", ms(t.elapsed()));
    let fin = wait(&mut rx, 60, |e| matches!(e, NeuralEvent::Finished { .. } | NeuralEvent::Error { .. })).await;
    println!("  primer final recibido: {fin:?} (debe ser id 31) · first {first:?}");

    // 5. Long continuous passage, played to the end (silently).
    let t = Instant::now();
    tts.speak(40, &text("pasaje")).await.unwrap();
    let first = wait(&mut rx, 30, |e| matches!(e, NeuralEvent::FirstAudio { id: 40, .. })).await;
    println!("pasaje: primer audio {} ms ({first:?})", ms(t.elapsed()));
    let fin = wait(&mut rx, 400, |e| matches!(e, NeuralEvent::Finished { id: 40, .. } | NeuralEvent::Error { .. })).await;
    println!("pasaje: {fin:?} en {} ms totales", ms(t.elapsed()));

    tts.shutdown().await;
}
