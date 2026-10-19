"""Designs reference voices for Jarvis with Qwen3-TTS VoiceDesign (once).
The Base model later clones the chosen reference, so every answer keeps the
same timbre. Writes <out>/jarvis-es-<n>.wav and .txt (transcript)."""
import sys
from pathlib import Path

import mlx.core as mx
import numpy as np
import soundfile as sf
from mlx_audio.tts.utils import load_model

out = Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
REF_TEXT = ("Hola, soy Jarvis. Estoy aquí para ayudarte con tu equipo y responder tus preguntas "
            "con calma y claridad. Cuando quieras, empezamos.")
INSTRUCT = ("Voz masculina adulta, de unos treinta y cinco años, con acento latinoamericano neutro. "
            "Timbre cálido y grave moderado, tono sereno, seguro y amable; ritmo conversacional natural, "
            "articulación clara, sin dramatismo ni entonación de locutor.")
model = load_model("mlx-community/Qwen3-TTS-12Hz-1.7B-VoiceDesign-8bit")
for n, seed in enumerate([7, 21, 42], start=1):
    mx.random.seed(seed)
    audio = np.concatenate([np.asarray(r.audio, dtype=np.float32) for r in model.generate_voice_design(text=REF_TEXT, instruct=INSTRUCT, language="Spanish")])
    sf.write(out / f"jarvis-es-{n}.wav", audio, model.sample_rate)
    (out / f"jarvis-es-{n}.txt").write_text(REF_TEXT)
    print(f"voz {n} (semilla {seed}): {len(audio)/model.sample_rate:.1f} s")
(out / "INSTRUCCION.txt").write_text(INSTRUCT)
