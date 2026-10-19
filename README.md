# JarvisHiGate

Jarvis funcional con Ollama (En fase de desarrollo).

Asistente de escritorio local para macOS: Tauri 2 + React/TypeScript, con Ollama como modelo local.

## Estado por fases

| Fase | Contenido | Estado |
|------|-----------|--------|
| 1 | HUD, métricas reales, Ollama (salud, modelos, streaming, cancelación), Command Layer, progreso, agente de solo lectura | **Implementada** |
| 2 | `jarvis diagnostico [--ram|--red|--full]`: diagnóstico de solo lectura con hallazgos basados en evidencias | **Implementada** |
| 3A | Voz básica: lectura en voz alta con `say`, selección de voz, interrupción, estados de voz | **Implementada** |
| 3B | Reconocimiento de voz (micrófono + motor local) | Pendiente de autorización |
| 4 | `jarvis claw`: OpenClaw | Pendiente: OpenClaw no está instalado |

## Requisitos

- Rust (stable), Node.js ≥ 20, Xcode Command Line Tools
- [Ollama](https://ollama.com) en ejecución con al menos un modelo (`ollama pull <modelo>`)
- Opcional: `JARVIS_OLLAMA_URL` (por defecto `http://127.0.0.1:11434`)

## Uso

```sh
npm install
npm run tauri dev                              # desarrollo
npm run tauri build -- --debug --bundles app   # genera target/debug/bundle/macos/Jarvis.app
```

### Entradas reconocidas

| Entrada | Efecto |
|---------|--------|
| texto libre | Conversación con el modelo (streaming, botón Detener / Esc) |
| `jarvis ayuda [comando]` | Ayuda integrada |
| `jarvis piensa --plan <tema>` | Plan estructurado (objetivo, pasos, riesgos, próximo paso) |
| `jarvis piensa --comparar <tema>` | Comparación de opciones y recomendación |
| `-- <objetivo>` | Análisis con panel de progreso |
| `jarvis diagnostico` | Resumen general: carga, almacenamiento, presión de memoria y swap (sin pruebas de red) |
| `jarvis diagnostico --ram` | Presión de memoria, nivel libre, swap, compresión y procesos por memoria residente |
| `jarvis diagnostico --red` | Interfaces, ruta por defecto, servidores DNS, ping al router, resolución DNS y conexiones TCP con tiempo límite |
| `jarvis diagnostico --full` | Todo lo anterior combinado |
| `jarvis conversa …`, `jarvis claw --task "…"` | Se validan; indican la fase en que estarán disponibles |

### Voz neuronal (fase 3A.3)

- **Motor por defecto, si está instalado:** Qwen3-TTS 1.7B en local (MLX), con la voz de Jarvis clonada de una referencia diseñada.
- **Alternativa de recuperación:** `say`, seleccionable desde el control de voz.
- **Proceso:** el auxiliar (`voice-engine/`) se arranca una sola vez, mantiene el modelo cargado y reproduce por un único flujo continuo con prebúfer.
- **Garantías:**
  - Esc o «Detener voz» paran la generación y la reproducción.
  - Una respuesta nueva corta la anterior y nunca suena audio de una locución cancelada.
- **Requisitos:** el entorno, los modelos y las licencias se documentan en `voice-engine/README.md`.
- **Limitación:** el script del proceso auxiliar se resuelve desde este árbol de código (builds locales en este Mac), no se empaqueta todavía dentro de la `.app`.

### Voz (fase 3A)

- **Leer respuestas:** al terminar una respuesta del chat, Jarvis la lee con `say` de macOS.
  - La voz por defecto es española; se elige otra con el selector.
  - El botón «▶ Leer última» lee la respuesta más reciente a petición.
- **Interrumpir:** Esc o «■ Detener voz». Una petición nueva también corta la lectura en curso.
- **Solo la respuesta más reciente:** una respuesta antigua o cancelada nunca se reproduce. Se comprueba en la interfaz y otra vez en el backend.
- **Ejecución de `say`:**
  - Ruta fija (`/usr/bin/say`); el texto va por stdin, así que nunca se interpreta como una opción.
  - La voz se valida contra la lista instalada, porque `say` usa en silencio la voz por defecto si el nombre no existe.
- **Micrófono:** no se usa. El botón aparece deshabilitado y explica por qué. No hay grabación, ni escucha continua, ni envío de audio.
- **Preparación del texto** (`voice/prosody.rs`), que desde la fase 3A.3 se limita a ayudas de pronunciación: sin pausas insertadas ni truncado:
  - Pausas según la estructura (párrafos, títulos, listas, advertencias, incisos).
  - Fechas, horas, porcentajes, unidades, siglas, IP, versiones, dominios y comandos se leen de forma inteligible.
  - No verbaliza Markdown ni bloques de código.
  - Bloquea los comandos `[[…]]` que vengan en el texto del modelo.
  - Límite de `say`: `[[rate]]` solo se aplica a toda la locución, así que el ritmo no puede variar dentro de una frase.
- **Prueba de escucha:** `cargo run -p jarvis-core --example voice_listening_test` genera muestras A/B por voz en `target/voice-samples/<fecha>/index.html`. No reproduce nada.
- **Reconocimiento (fase 3B):** está definida la interfaz `SpeechToText` y una detección de motores de solo lectura (`crates/jarvis-core/src/voice/stt.rs`).

### Diagnóstico

- Cada hallazgo indica su tipo (problema confirmado, advertencia u observación), la gravedad, la evidencia con su fuente, el nivel de confianza y una recomendación.
- Los datos que no se pueden obtener aparecen en «Datos no disponibles» con el motivo; nunca se rellenan con valores inventados.
- Funciona aunque Ollama no esté en ejecución.
- Solo lectura: no cierra procesos, no cambia ajustes y no escribe archivos. Las «acciones propuestas» son texto y no se ejecutan nunca.
- Comandos del sistema: un `enum` cerrado (`crates/jarvis-core/src/diagnostics/sysexec.rs`). Cada comando tiene una ruta absoluta y argumentos fijos, y se ejecuta sin shell, con `LC_ALL=C` y un límite de tiempo. El único argumento variable es la IP de la puerta de enlace, tipada como `Ipv4Addr`.
  - Comandos usados: `vm_stat`, `sysctl -n` (presión, nivel libre, swap), `route -n get default`, `scutil --dns`, `networksetup -listnetworkserviceorder` y `ping -c 2 -t 3 <gateway>`.
  - `memory_pressure` se excluye a propósito: aplica o simula presión en lugar de solo leerla.
- Pruebas de red: 1.1.1.1:443 y 8.8.8.8:443 por IP, `apple.com` y `cloudflare.com` por DNS, y una conexión TCP al resultado de DNS. Cada prueba tiene un límite de 3 s.
- Que un destino falle no demuestra por sí solo que toda la conexión esté caída, y el informe lo indica.
- Limitación conocida: en macOS reciente, el ping al router puede fallar con «No route to host» si la app no tiene el permiso de **Red local** (Ajustes del Sistema › Privacidad y seguridad › Red local). El diagnóstico lo identifica como tal y no como una avería.

Atajo global **⌘J**: abre/cierra el panel rápido (mismo enrutador que el chat). Si no se puede registrar, la interfaz lo indica y permite elegir otro en «Panel rápido». Los ajustes se guardan en `~/Library/Application Support/com.jarvis.desktop/settings.json`.

## Arquitectura

```
crates/jarvis-core/   lógica pura y testeable
  command/            tokenizador, parser, ayuda, tipos (Command)
  router.rs           entrada → Command | CommandError | Agent | Chat
  events.rs           ProgressEvent tipado (id, run_id, seq, kind, status, timestamp)
  agent/              ToolRegistry + PolicyEngine (solo lectura) + Limits/Budget + run_think
  llm/                cliente Ollama (/api/version, /api/tags, /api/chat) y sesión en memoria
  telemetry.rs        sysinfo (compartido por el muestreo periódico y el diagnóstico); métricas no disponibles se marcan con motivo
  diagnostics/        sysexec (comandos fijos), probe (acceso real, sustituible en tests), parse, análisis memory/network/system
src-tauri/            IPC fino, eventos jarvis://{chat,progress,telemetry,shortcut}, atajo global
src/                  HUD React: Core (canvas), métricas, chat, RunView, panel rápido
```

Seguridad:
- El texto que genera el modelo nunca se interpreta como comando.
- El agente solo llama herramientas registradas (hoy solo `llm.ask`), y la política deniega todo lo que no sea de solo lectura.
- No hay herramienta de shell.
- No se registran prompts ni respuestas en logs.
- La conversación vive solo en memoria.

## Pruebas

```sh
cargo test --workspace                        # parser, router, errores de Ollama (servidor simulado), cancelación, política, límites
npm test                                      # reductores de progreso y conversación
npm run typecheck && cargo clippy --workspace --all-targets -- -D warnings
# contrato JSON Rust ↔ frontend: si cambia la forma del informe, regenerar el fixture que renderiza vitest
JARVIS_UPDATE_FIXTURES=1 cargo test -p jarvis-core --test diagnostics partial_results
# diagnóstico de red real (necesita Internet):
cargo test -p jarvis-core --test diagnostics live_network -- --ignored --nocapture
# interfaz real en Chrome headless (scroll, panel rápido, voz) sobre un mock del IPC:
npm run dev &        # Vite en :1420
npm run ui:check     # capturas en target/ui-check/
# contra Ollama real (opcional):
JARVIS_LIVE_MODEL=qwen3.5:4b cargo test -p jarvis-core --test live_ollama -- --ignored --nocapture
```
