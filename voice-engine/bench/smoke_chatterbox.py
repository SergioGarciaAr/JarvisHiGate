# Smoke test: load Chatterbox Multilingual (es-mx-latam finetune) on MPS and time one sentence.
import os, sys, time, resource
from pathlib import Path
import torch, torchaudio as ta
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from jarvis_tts import chatterbox_engine

ckpt = Path(os.environ["CHATTERBOX_CKPT"])
device = sys.argv[1] if len(sys.argv) > 1 else "mps"
t0 = time.time()
model = chatterbox_engine.load(ckpt, device)
load = time.time() - t0
text = "Hola, soy Jarvis. Revisé tu equipo y todo está en orden; la memoria y el disco funcionan con normalidad."
for i in range(2):
    t1 = time.time()
    wav = model.generate(text, language_id="es")
    gen = time.time() - t1
    dur = wav.shape[-1] / model.sr
    print(f"[{device}] pasada {i+1}: generación {gen:.2f} s · audio {dur:.2f} s · RTF {gen/dur:.2f}")
ta.save(sys.argv[2] if len(sys.argv) > 2 else "smoke.wav", wav, model.sr)
print(f"carga {load:.1f} s · sr {model.sr} · RSS máx {resource.getrusage(resource.RUSAGE_SELF).ru_maxrss/1e9:.2f} GB")
