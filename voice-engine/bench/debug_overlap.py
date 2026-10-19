# Reproduce: second speak while the first is generating/playing (real engine, simulated output).
import json, sys, threading, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from jarvis_tts import engines, server as srv
from jarvis_tts.player import Player
T0 = time.perf_counter()
srv.emit = lambda **e: print(f"{time.perf_counter()-T0:7.2f}s EVENT {e}", flush=True)
B = Path.home() / "Library/Application Support/com.jarvis.desktop/voice-engine/models/voices/jarvis-es-1"
s = srv.Server(lambda: engines.Qwen3Clone("mlx-community/Qwen3-TTS-12Hz-1.7B-Base-8bit", B.with_suffix(".wav"), B.with_suffix(".txt").read_text()), lambda sr: Player(sr, device_output=False))
s.ready.wait(); assert s.startup_error is None, s.startup_error
texts = json.loads(Path(sys.argv[1]).read_text())
print(f"{time.perf_counter()-T0:7.2f}s speak 21 pasaje", flush=True); s.speak(21, texts["pasaje"])
time.sleep(4); print(f"{time.perf_counter()-T0:7.2f}s stop", flush=True); s.stop()
print(f"{time.perf_counter()-T0:7.2f}s speak 30", flush=True); s.speak(30, texts["explicacion"])
time.sleep(3); t = time.perf_counter(); print(f"{t-T0:7.2f}s speak 31 (llamada)", flush=True); s.speak(31, texts["breve"]); print(f"{time.perf_counter()-T0:7.2f}s speak 31 devuelto ({(time.perf_counter()-t)*1000:.0f} ms)", flush=True)
time.sleep(12)
print("hilos vivos:", [th.name for th in threading.enumerate()])
