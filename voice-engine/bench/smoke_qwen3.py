# Smoke test: Qwen3-TTS 1.7B VoiceDesign (MLX, 8-bit) in Spanish, streaming vs full.
import sys, time, resource
import numpy as np, soundfile as sf
from mlx_audio.tts.utils import load_model

repo, out = sys.argv[1], sys.argv[2]
t0 = time.time(); model = load_model(repo); load = time.time() - t0
text = "Hola, soy Jarvis. Revisé tu equipo y todo está en orden; la memoria y el disco funcionan con normalidad."
instruct = "Voz masculina adulta de español latinoamericano neutro, cálida, serena y segura; ritmo conversacional natural, sin dramatismo."
for i in range(2):
    t1 = time.time(); first = None; chunks = []
    for r in model.generate_voice_design(text=text, language="Spanish", instruct=instruct, stream=True, streaming_interval=0.32):
        if first is None: first = time.time() - t1
        chunks.append(np.array(r.audio))
    gen = time.time() - t1
    audio = np.concatenate(chunks); sr = model.sample_rate
    print(f"pasada {i+1}: primer audio {first*1000:.0f} ms · total {gen:.2f} s · audio {len(audio)/sr:.2f} s · RTF {gen/(len(audio)/sr):.2f} · {len(chunks)} trozos")
sf.write(out, audio, sr)
print(f"carga {load:.1f} s · sr {sr} · RSS máx {resource.getrusage(resource.RUSAGE_SELF).ru_maxrss/1e9:.2f} GB")
