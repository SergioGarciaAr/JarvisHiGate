"""Bench one engine on the prepared phrase set. Nothing is played.

  python bench/run_bench.py <engine> <prepared.json> <out_dir>

Engines: qwen3-clone, chatterbox-latam, chatterbox-latam-ref
Writes <out>/audio/<key>__<engine>.wav (continuous, as generated),
<key>__<engine>__tiempo-real.wav (as heard in real time, with measured
underruns as silence) and <out>/metrics-<engine>.json.
"""
import json
import resource
import sys
import threading
import time
from pathlib import Path

import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from jarvis_tts import engines  # noqa: E402
from jarvis_tts.chunker import chunks  # noqa: E402
from jarvis_tts.pipeline import realtime_render, stream_text, timeline  # noqa: E402

BASE = Path.home() / "Library/Application Support/com.jarvis.desktop/voice-engine"
VOICE = BASE / "models/voices/jarvis-es-1"


def make(engine_id: str):
    if engine_id == "qwen3-clone":
        return engines.Qwen3Clone("mlx-community/Qwen3-TTS-12Hz-1.7B-Base-8bit", VOICE.with_suffix(".wav"), VOICE.with_suffix(".txt").read_text())
    if engine_id == "chatterbox-latam":
        return engines.ChatterboxLatam(BASE / "models/chatterbox-es-latam-mlx")
    if engine_id == "chatterbox-latam-ref":
        e = engines.ChatterboxLatam(BASE / "models/chatterbox-es-latam-mlx", ref_audio=VOICE.with_suffix(".wav"))
        e.id = "chatterbox-latam-ref"
        return e
    raise SystemExit(f"motor desconocido: {engine_id}")


def rss_gb() -> float:
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1e9  # bytes on macOS


def main():
    engine_id, prepared_path, out = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
    (out / "audio").mkdir(parents=True, exist_ok=True)
    texts = json.loads(prepared_path.read_text())
    t0 = time.perf_counter()
    eng = make(engine_id)
    load_s = time.perf_counter() - t0
    sr = eng.sample_rate

    # Cold first call, then the model stays loaded (production scenario).
    t1 = time.perf_counter()
    warm = list(stream_text(eng, "Hola, esto es una prueba de calentamiento.", threading.Event()))
    cold_s = time.perf_counter() - t1
    metrics = {"engine": engine_id, "sample_rate": sr, "load_s": round(load_s, 2), "cold_first_call_s": round(cold_s, 2), "items": {}}
    print(f"[{engine_id}] carga {load_s:.1f} s · primera llamada en frío {cold_s:.1f} s ({len(warm)} trozos)", flush=True)

    for key, text in texts.items():
        cancel = threading.Event()
        start = time.perf_counter()
        pieces = list(stream_text(eng, text, cancel, t0=start))
        t = timeline(pieces, sr)
        continuous = __import__("numpy").concatenate([p.audio for p in pieces])
        sf.write(out / "audio" / f"{key}__{engine_id}.wav", continuous, sr)
        if t["gaps"]:
            sf.write(out / "audio" / f"{key}__{engine_id}__tiempo-real.wav", realtime_render(pieces, sr), sr)
        item = {
            "chars": len(text),
            "fragments": len(chunks(text)),
            "first_audio_ms": round(t["first_audio_s"] * 1000),
            "audio_s": round(t["audio_s"], 2),
            "gen_s": round(t["gen_s"], 2),
            "rtf": round(t["gen_s"] / t["audio_s"], 3) if t["audio_s"] else None,
            "underruns": len(t["gaps"]),
            "underrun_total_s": round(t["gap_total_s"], 2),
            "underruns_over_50ms": t["gaps_over_50ms"],
            "rss_peak_gb": round(rss_gb(), 2),
        }
        metrics["items"][key] = item
        print(f"[{engine_id}] {key:15} {item}", flush=True)

    metrics["rss_peak_gb"] = round(rss_gb(), 2)
    (out / f"metrics-{engine_id}.json").write_text(json.dumps(metrics, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
