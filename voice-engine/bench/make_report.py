"""Builds the listening page (index.html) for a bench folder. Nothing is
played automatically. Adds `say` (Paulina) renders as the current baseline.

  python bench/make_report.py <out_dir>
"""
import html
import json
import shutil
import subprocess
import sys
from pathlib import Path

out = Path(sys.argv[1])
audio = out / "audio"
texts = json.loads((out / "prepared.json").read_text())
raw = json.loads((Path(__file__).parent / "phrases.json").read_text())
asr = json.loads((out / "asr-eval.json").read_text()) if (out / "asr-eval.json").exists() else {}
metrics = {p.stem.removeprefix("metrics-"): json.loads(p.read_text()) for p in out.glob("metrics-*.json")}
voices_dir = Path.home() / "Library/Application Support/com.jarvis.desktop/voice-engine/models/voices"

# Baseline: macOS `say` with the current default voice, same prepared text.
for key, text in texts.items():
    dst = audio / f"{key}__say-paulina.wav"
    if not dst.exists():
        tmp = audio / f"{key}__say.aiff"
        subprocess.run(["/usr/bin/say", "-v", "Paulina (Español (México))", "-f", "-", "-o", str(tmp)], input=text.encode(), check=True)
        subprocess.run(["/usr/bin/afconvert", "-f", "WAVE", "-d", "LEI16", str(tmp), str(dst)], check=True)
        tmp.unlink()

(out / "voces").mkdir(exist_ok=True)
for wav in sorted(voices_dir.glob("jarvis-es-*.wav")):
    shutil.copy(wav, out / "voces" / wav.name)

ENGINES = [
    ("qwen3-clone", "Qwen3-TTS 1.7B Base 8-bit · voz Jarvis clonada (diseñada con VoiceDesign)"),
    ("chatterbox-latam-ref", "Chatterbox Multilingual V3 + es-mx-latam · misma voz de referencia"),
    ("chatterbox-latam", "Chatterbox Multilingual V3 + es-mx-latam · voz por defecto"),
    ("say-paulina", "macOS say · Paulina (motor actual, referencia)"),
]


def esc(s):
    return html.escape(str(s))


rows = []
for key in texts:
    rows.append(f"<h2>{esc(key)}</h2><details><summary>Texto original y preparado</summary><pre>{esc(raw.get(key, ''))}</pre><pre class=prep>{esc(texts[key])}</pre></details><table><tr><th>Motor</th><th>Audio</th><th>Primer audio</th><th>RTF</th><th>Huecos</th><th>Whisper</th></tr>")
    for eng, label in ENGINES:
        f = audio / f"{key}__{eng}.wav"
        if not f.exists():
            continue
        m = metrics.get(eng, {}).get("items", {}).get(key, {})
        a = asr.get(eng, {}).get(key, {})
        rt = audio / f"{key}__{eng}__tiempo-real.wav"
        rt_html = f"<br><small>tiempo real (con huecos medidos):</small><br><audio controls preload=none src='audio/{rt.name}'></audio>" if rt.exists() else ""
        gaps = f"{m.get('underruns', 0)} · {m.get('underrun_total_s', 0)} s" if m else "—"
        wer = f"WER {a['wer']:.2f} · −{a['omitted']} +{a['inserted']} ~{a['substituted']}" if a else "—"
        rows.append(
            f"<tr><td>{esc(label)}</td><td><audio controls preload=none src='audio/{f.name}'></audio>{rt_html}</td>"
            f"<td>{m.get('first_audio_ms', '—')} ms</td><td>{m.get('rtf', '—')}</td><td>{gaps}</td><td title='{esc(a.get('transcript', ''))}'>{wer}</td></tr>"
        )
    rows.append("</table>")

summary = ["<table><tr><th>Motor</th><th>Carga</th><th>1.ª llamada en frío</th><th>Primer audio (mediana)</th><th>RTF medio</th><th>Huecos en el pasaje</th><th>RSS máx.</th></tr>"]
for eng, label in ENGINES:
    m = metrics.get(eng)
    if not m:
        continue
    items = m["items"].values()
    firsts = sorted(i["first_audio_ms"] for i in items)
    rtf = sum(i["rtf"] for i in items) / len(items)
    p = m["items"].get("pasaje", {})
    summary.append(
        f"<tr><td>{esc(label)}</td><td>{m['load_s']} s</td><td>{m['cold_first_call_s']} s</td><td>{firsts[len(firsts)//2]} ms</td>"
        f"<td>{rtf:.2f}</td><td>{p.get('underruns')} · {p.get('underrun_total_s')} s</td><td>{m['rss_peak_gb']} GB</td></tr>"
    )
summary.append("</table>")
voices = "".join(f"<p>{esc(w.stem)}<br><audio controls preload=none src='voces/{w.name}'></audio></p>" for w in sorted((out / "voces").glob("*.wav")))

page = f"""<!doctype html><meta charset=utf-8><title>Jarvis · TTS neuronal · prueba de escucha</title>
<style>body{{font:15px -apple-system,sans-serif;background:#0b1118;color:#d6e6f2;max-width:1200px;margin:24px auto;padding:0 16px}}
h1,h2{{color:#38e1ff}}table{{border-collapse:collapse;width:100%;margin:8px 0}}td,th{{border-bottom:1px solid #1d2b3a;padding:6px;text-align:left;vertical-align:top}}
pre{{white-space:pre-wrap;background:#0f1822;padding:8px}}pre.prep{{color:#7d93a6}}audio{{width:320px}}small{{color:#7d93a6}}.warn{{color:#ffc457}}</style>
<h1>Prueba de escucha · TTS neuronal local</h1>
<p>Nada se reproduce automáticamente. Usa auriculares. Escucha primero el <b>pasaje</b> (más de 2 minutos) completo con cada motor: es lo que más se parece a usar Jarvis.
La versión «tiempo real» reproduce los huecos que se oirían en este Mac mientras se genera; la normal es el audio continuo generado.</p>
<p class=warn>Revisa especialmente: el final del pasaje de Chatterbox con voz por defecto (Whisper detecta «bien, bien, bien…» tras la última frase) y «391,5 gigabytes» en «cifras» de Qwen3.</p>
<p>El WER se calcula con Whisper large-v3-turbo; buena parte de los «errores» son diferencias de formato (cifras en dígitos, «GB», «192.168»), no fallos de pronunciación.</p>
<h2>Resumen medido en este Mac (M1 Pro, 16 GB)</h2>{''.join(summary)}
<h2>Voces de referencia diseñadas (elige la que prefieras para Jarvis)</h2>{voices}
{''.join(rows)}"""
(out / "index.html").write_text(page)
print(out / "index.html")
