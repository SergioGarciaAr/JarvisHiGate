//! Applies Jarvis' speech text preparation to a JSON map of texts, so the
//! neural TTS bench uses exactly the same normalization as the app.
//!
//!   cargo run -p jarvis-core --example prepare_texts -- in.json out.json

use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input: BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(&args[1]).expect("leer entrada")).expect("JSON válido");
    let prepared: BTreeMap<String, String> =
        input.into_iter().map(|(k, v)| (k, jarvis_core::voice::prosody::prepare(&v))).collect();
    std::fs::write(&args[2], serde_json::to_string_pretty(&prepared).unwrap()).expect("escribir salida");
}
