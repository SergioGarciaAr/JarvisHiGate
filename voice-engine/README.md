# Motor de voz neuronal de Jarvis (local)

Proceso auxiliar de Python que sintetiza voz con un modelo neuronal ejecutado localmente con MLX en Apple Silicon.

- Jarvis (Rust/Tauri) lo arranca una sola vez y se comunica con él por JSON en stdin/stdout (`jarvis_tts/server.py`).
- El modelo se queda cargado entre respuestas.
- El audio sale por un único flujo continuo, con cola y prebúfer (`jarvis_tts/player.py`).
- Funciona sin red: Jarvis lo lanza con `HF_HUB_OFFLINE=1`, así que el texto no sale del Mac y no se descarga nada en tiempo de ejecución.

## Componentes y licencias

| Componente | Uso | Licencia |
|---|---|---|
| Qwen3-TTS 12Hz 1.7B Base (8-bit, `mlx-community/Qwen3-TTS-12Hz-1.7B-Base-8bit`) | motor de producción (clonación de la voz de referencia) | Apache-2.0 (código y pesos) |
| Qwen3-TTS 12Hz 1.7B VoiceDesign (8-bit) | diseño de las voces de referencia `jarvis-es-*` | Apache-2.0 |
| mlx-audio 0.5.9 | ejecución de los modelos con MLX | MIT |
| MLX 0.32 | runtime de Apple Silicon | MIT |
| sounddevice / soundfile | salida de audio y WAV | MIT / BSD-3 |
| Chatterbox Multilingual V3 + `es-mx-latam` | solo banco de pruebas (descartado: RTF ≈ 1 y huecos) | MIT. Incrusta la marca de agua PerTh en el audio |
| Whisper large-v3-turbo (`mlx-whisper`) | solo evaluación (medir palabras omitidas o repetidas); no forma parte de Jarvis | MIT |

## Ubicaciones (fuera del proyecto)

- **Entornos:** `~/Library/Application Support/com.jarvis.desktop/voice-engine/{mlx-venv,chatterbox-venv}`
- **Voces de referencia:** `…/voice-engine/models/voices/jarvis-es-{1,2,3}.wav` y `.txt`
- **Modelos:** `~/Library/Caches/Jarvis/hf` (caché de Hugging Face)

## Pruebas

```sh
PY="$HOME/Library/Application Support/com.jarvis.desktop/voice-engine/mlx-venv/bin/python"
"$PY" -m pytest -q voice-engine/tests                       # troceado, recorte, protocolo, cancelación, un solo hilo MLX
cargo test -p jarvis-core --test neural                     # gestión del proceso desde Rust (proceso auxiliar falso)
cargo run -p jarvis-core --example neural_live_check -- <prepared.json>   # motor real, dispositivo real, ganancia 0 (no suena)
```

Banco de pruebas y página de escucha: `voice-engine/bench/` (`run_bench.py`, `asr_eval.py`, `make_report.py`).
