"""Objective intelligibility check: transcribes each continuous sample with
Whisper (evaluation only, not part of Jarvis) and aligns it to the prepared
text to count omitted, inserted and substituted words.

  python bench/asr_eval.py <out_dir>
"""
import difflib
import json
import re
import sys
import unicodedata
from pathlib import Path

import mlx_whisper
import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

MODEL = "mlx-community/whisper-large-v3-turbo"


def words(text: str) -> list[str]:
    t = unicodedata.normalize("NFC", text.lower())
    t = re.sub(r"[^\w\sáéíóúñü]", " ", t)
    return t.split()


def align(ref: list[str], hyp: list[str]) -> dict:
    sm = difflib.SequenceMatcher(a=ref, b=hyp, autojunk=False)
    omitted = inserted = substituted = 0
    examples = []
    for op, i1, i2, j1, j2 in sm.get_opcodes():
        if op == "delete":
            omitted += i2 - i1
            examples.append(f"falta: «{' '.join(ref[i1:i2])}»")
        elif op == "insert":
            inserted += j2 - j1
            examples.append(f"sobra: «{' '.join(hyp[j1:j2])}»")
        elif op == "replace":
            n = max(i2 - i1, j2 - j1)
            substituted += min(i2 - i1, j2 - j1)
            omitted += max(0, (i2 - i1) - (j2 - j1))
            inserted += max(0, (j2 - j1) - (i2 - i1))
            if n <= 6:
                examples.append(f"«{' '.join(ref[i1:i2])}» → «{' '.join(hyp[j1:j2])}»")
    errors = omitted + inserted + substituted
    return {
        "ref_words": len(ref),
        "wer": round(errors / max(1, len(ref)), 3),
        "omitted": omitted,
        "inserted": inserted,
        "substituted": substituted,
        "examples": examples[:12],
    }


def main():
    out = Path(sys.argv[1])
    texts = json.loads((out / "prepared.json").read_text())
    results = {}
    for wav in sorted((out / "audio").glob("*.wav")):
        if wav.stem.endswith("__tiempo-real"):
            continue
        key, engine = wav.stem.split("__")
        audio, sr = sf.read(wav, dtype="float32")
        audio16 = resample_poly(audio, 16000, sr).astype(np.float32)  # Whisper expects 16 kHz
        hyp = mlx_whisper.transcribe(audio16, path_or_hf_repo=MODEL, language="es")["text"]
        r = align(words(texts[key]), words(hyp))
        r["transcript"] = hyp.strip()
        results.setdefault(engine, {})[key] = r
        print(f"{engine:22} {key:15} WER {r['wer']:.3f} · omitidas {r['omitted']} · añadidas {r['inserted']} · sustituidas {r['substituted']}", flush=True)
    (out / "asr-eval.json").write_text(json.dumps(results, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
