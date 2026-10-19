//! Listening test: renders the same phrases with every candidate voice, with
//! the previous text preparation (A) and the prosody layer (B). Nothing is
//! played; files go to a new timestamped folder under target/voice-samples.
//!
//!   cargo run -p jarvis-core --example voice_listening_test
//!   open target/voice-samples/<fecha>/index.html

use std::path::{Path, PathBuf};
use std::process::Command;

use jarvis_core::voice::prosody;
use jarvis_core::voice::tts::{self, Voice};

const PHRASES: &[(&str, &str)] = &[
    ("breve", "¡Hola! Claro que sí, ahora mismo lo miro."),
    ("pregunta", "¿Quieres que revise también la conexión de red o prefieres empezar por la memoria?"),
    (
        "explicacion",
        "La presión de memoria es normal. macOS usa la RAM libre como caché de archivos, así que un porcentaje alto de memoria «usada» no indica un problema por sí solo.",
    ),
    (
        "enumeracion",
        "Para liberar espacio te propongo tres pasos:\n1. Vacía la papelera.\n2. Revisa la carpeta Descargas.\n3. Elimina las copias de seguridad antiguas de iOS.",
    ),
    ("cifras", "El 18/10/2026 a las 14:30 el disco tenía 391,5 GB libres, un 85% del total."),
    ("pausa", "He revisado todo… y, sinceramente, el equipo está en buen estado."),
    (
        "advertencia",
        "⚠️ **Importante:** no apagues el Mac mientras se instala la actualización, o podrías dañar el sistema.",
    ),
    ("comando", "Si quieres más detalle, escribe `jarvis diagnostico --full` y te mostraré el informe completo."),
    ("red", "El router 192.168.1.1 responde en 3 ms y el DNS resuelve apple.com en 20 ms."),
    ("aclaracion", "Chrome es el proceso que más memoria usa (alrededor de 1 GB), pero no es preocupante."),
];

/// Text preparation used before phase 3A.2, kept here only for the A/B comparison.
fn legacy(input: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in input.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let line = line.trim_start_matches(|c: char| c == '#' || c == '>' || c.is_whitespace());
        let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line);
        out.push_str(line);
        out.push('\n');
    }
    let cleaned: String = out.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '|')).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// (base name, locale). Matched by base name: macOS lists some voices as
/// "Mónica" and others as "Mónica (Español (España))".
const CANDIDATES: &[(&str, &str)] = &[("Paulina", "es_MX"), ("Mónica", "es_ES"), ("Eddy", "es_MX"), ("Flo", "es_MX")];

fn to_m4a(aiff: &Path) -> PathBuf {
    let m4a = aiff.with_extension("m4a");
    let ok = Command::new("/usr/bin/afconvert")
        .args(["-f", "m4af", "-d", "aac"])
        .arg(aiff)
        .arg(&m4a)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        let _ = std::fs::remove_file(aiff);
        m4a
    } else {
        aiff.to_path_buf()
    }
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[tokio::main]
async fn main() {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/voice-samples").join(&stamp);
    std::fs::create_dir_all(&root).expect("crear carpeta de muestras");
    let voices = tts::list_voices().await.expect("listar voces");
    let chosen: Vec<&Voice> = CANDIDATES
        .iter()
        .filter_map(|(base, locale)| voices.iter().find(|v| v.base_name() == *base && v.locale == *locale))
        .collect();

    let passage_raw = PHRASES.iter().map(|(_, t)| *t).collect::<Vec<_>>().join("\n\n");
    let mut items: Vec<(String, String)> = PHRASES.iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
    items.push(("pasaje-continuo".into(), passage_raw));

    let mut rows = String::new();
    for (key, text) in &items {
        let a_text = legacy(text);
        let b = prosody::prepare(text);
        let b_text = b.clone();
        rows.push_str(&format!(
            "<h2>{key}</h2><pre class=src>{}</pre><p class=prep><b>A</b> {}<br><b>B</b> {}</p><table><tr><th>Voz</th><th>A · preparación anterior</th><th>B · capa prosódica</th></tr>",
            html(text),
            html(&a_text),
            html(&b_text)
        ));
        for v in &chosen {
            let mut cells = Vec::new();
            for (variant, script) in [("A", a_text.as_str()), ("B", b_text.as_str())] {
                let file = root.join(format!("{key}__{}__{variant}.aiff", slug(&v.name)));
                match tts::render_to_file(v, script, file.clone()).await {
                    Ok(_) => {
                        let out = to_m4a(&file);
                        let name = out.file_name().unwrap().to_string_lossy().into_owned();
                        cells.push(format!("<audio controls preload=none src=\"{name}\"></audio>"));
                    }
                    Err(e) => cells.push(format!("error: {}", html(&e.to_string()))),
                }
            }
            rows.push_str(&format!("<tr><td>{} · {}</td><td>{}</td><td>{}</td></tr>", html(&v.name), v.locale, cells[0], cells[1]));
        }
        rows.push_str("</table>");
        println!("{key}: {} voces", chosen.len());
    }

    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Jarvis · prueba de escucha {stamp}</title><style>body{{font:15px -apple-system,sans-serif;background:#0b1118;color:#d6e6f2;max-width:1100px;margin:24px auto;padding:0 16px}}h2{{color:#38e1ff;margin-top:32px}}table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #1d2b3a;padding:6px;text-align:left}}pre.src{{white-space:pre-wrap;background:#0f1822;padding:8px}}p.prep{{font:12px ui-monospace,monospace;color:#7d93a6}}audio{{width:100%}}</style><h1>Prueba de escucha · {stamp}</h1><p>Escucha con auriculares, primero frases sueltas y después el <b>pasaje continuo</b> (más de un minuto): ahí se nota la fatiga de una voz. Compara A y B con la misma voz (prosodia), y voces entre sí (timbre). No se reproduce nada automáticamente.</p>{rows}<h2>Referencia (no integrable en Jarvis)</h2><p>Si existe <code>referencia-siri-carmen.m4a</code>, es la voz neuronal de Siri es-MX ya instalada, generada solo para comparar: macOS no la expone a apps de terceros.</p><audio controls preload=none src=\"referencia-siri-carmen.m4a\"></audio>"
    );
    std::fs::write(root.join("index.html"), page).unwrap();
    std::fs::write(root.join("frases.txt"), PHRASES.iter().map(|(k, t)| format!("[{k}]\n{t}\n")).collect::<String>()).unwrap();
    println!("\nMuestras en {}\nÁbrelas con: open {}/index.html", root.display(), root.display());
}

fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>")
}
