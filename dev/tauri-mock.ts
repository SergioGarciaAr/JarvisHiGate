// Development-only stand-in for the Tauri IPC bridge, so the real React UI
// can be exercised in a regular browser by scripts/ui-check.mjs.
// It is not part of the production build (only index.html is bundled).
import fixture from "../src/lib/fixtures/diagnostic-report.json";

type Cb = (msg: unknown) => void;

const params = new URLSearchParams(location.search);
const label = params.get("window") ?? "main";
const callbacks = new Map<number, Cb>();
const listeners = new Map<string, Map<number, number>>();
let nextId = 1;
const streams = new Map<string, { timer: number; cancelled: boolean }>();

function emit(event: string, payload: unknown) {
  for (const [eventId, cbId] of listeners.get(event) ?? []) callbacks.get(cbId)?.({ event, id: eventId, payload });
}

const mock = {
  emit,
  /** Text the next chat reply will stream. */
  nextReply: "Respuesta de prueba.",
  chunkSize: 12,
  intervalMs: 15,
  calls: [] as { cmd: string; args: unknown }[],
  voices: [
    { name: "Mónica", locale: "es_ES", sample: "Hola, me llamo Mónica." },
    { name: "Eddy (Español (España))", locale: "es_ES", sample: "¡Hola! Me llamo Eddy." },
    { name: "Samantha", locale: "en_US", sample: "Hello, my name is Samantha." },
  ],
  speaking: null as null | { utterance: number; requestId: string; timer: number },
  speakMs: 400,
};

function streamChat(requestId: string) {
  const text = mock.nextReply;
  let pos = 0;
  const state = { timer: 0, cancelled: false };
  streams.set(requestId, state);
  const tick = () => {
    if (state.cancelled) return;
    if (pos >= text.length) {
      emit("jarvis://chat", { kind: "done", request_id: requestId });
      streams.delete(requestId);
      return;
    }
    emit("jarvis://chat", { kind: "delta", request_id: requestId, text: text.slice(pos, pos + mock.chunkSize) });
    pos += mock.chunkSize;
    state.timer = window.setTimeout(tick, mock.intervalMs);
  };
  state.timer = window.setTimeout(tick, mock.intervalMs);
}

function runDiagnostic(requestId: string) {
  let seq = 0;
  const ev = (kind: string, status: string, summary: string, data?: unknown) =>
    emit("jarvis://progress", { id: `${requestId}-${seq}`, run_id: requestId, seq: seq++, kind, status, timestamp: new Date().toISOString(), summary, data });
  const steps: [string, string, string, string?, unknown?][] = [
    ["goal", "ok", "Diagnóstico completo (solo lectura)"],
    ["phase", "running", "Memoria"],
    ["tool_call", "running", "vm_stat"],
    ["tool_result", "ok", "Estadísticas de memoria virtual"],
    ["result", "ok", "1 advertencias", undefined, fixture],
    ["completed", "ok", "Diagnóstico completado"],
  ];
  steps.forEach(([k, s, sum, , data], i) => window.setTimeout(() => ev(k, s, sum, data), 20 * (i + 1)));
}

async function invoke(cmd: string, args: Record<string, unknown> = {}) {
  mock.calls.push({ cmd, args });
  switch (cmd) {
    case "plugin:event|listen": {
      const id = nextId++;
      const map = listeners.get(args.event as string) ?? new Map();
      map.set(id, args.handler as number);
      listeners.set(args.event as string, map);
      return id;
    }
    case "plugin:event|unlisten":
      listeners.get(args.event as string)?.delete(args.eventId as number);
      return null;
    case "ollama_health":
      return { status: "available", version: "mock", models: [{ name: "mock-model", size: 1, capabilities: [] }] };
    case "get_settings":
      return { shortcut: "Cmd+J", model: "mock-model", voice: null, autospeak: false, tts_engine: null, neural_voice: null };
    case "shortcut_status":
      return { shortcut: "Cmd+J", registered: true, error: null };
    case "submit_input": {
      const id = args.requestId as string;
      const text = (args.text as string).trim();
      if (text.startsWith("jarvis diagnostico")) {
        runDiagnostic(id);
        return { kind: "diagnostic_started", request_id: id, scope: "full" };
      }
      streamChat(id);
      return { kind: "chat_started", request_id: id, model: "mock-model" };
    }
    case "cancel_request": {
      const s = streams.get(args.requestId as string);
      if (!s) return false;
      s.cancelled = true;
      clearTimeout(s.timer);
      emit("jarvis://chat", { kind: "cancelled", request_id: args.requestId });
      streams.delete(args.requestId as string);
      return true;
    }
    case "voice_status":
      return {
        tts: { available: true, voices: mock.voices, selected: "Mónica", reason: null },
        stt: {
          available: false,
          reason: "Reconocimiento de voz no instalado (fase 3B, pendiente de autorización).",
          engines: [
            { engine: "whisper_cpp", status: "not_installed", detail: "whisper-cli no encontrado" },
            { engine: "apple_speech_transcriber", status: "not_integrated", detail: "requiere un ayudante Swift" },
          ],
        },
        autospeak: false,
        engine: "neural",
        engines: [
          { id: "neural", label: "Neuronal local · Qwen3-TTS", available: true, reason: null },
          { id: "say", label: "Sistema · say (recuperación)", available: true, reason: null },
        ],
        neural_voices: ["jarvis-es-1", "jarvis-es-2", "jarvis-es-3"],
        neural_voice: "jarvis-es-1",
      };
    case "voice_speak": {
      if (mock.speaking) {
        clearTimeout(mock.speaking.timer);
        emit("jarvis://voice", { kind: "stopped", utterance: mock.speaking.utterance, request_id: mock.speaking.requestId });
      }
      const utterance = nextId++;
      const requestId = args.requestId as string;
      emit("jarvis://voice", { kind: "started", utterance, request_id: requestId });
      const timer = window.setTimeout(() => {
        mock.speaking = null;
        emit("jarvis://voice", { kind: "finished", utterance, request_id: requestId });
      }, mock.speakMs);
      mock.speaking = { utterance, requestId, timer };
      return utterance;
    }
    case "voice_stop": {
      if (!mock.speaking) return false;
      clearTimeout(mock.speaking.timer);
      emit("jarvis://voice", { kind: "stopped", utterance: mock.speaking.utterance, request_id: mock.speaking.requestId });
      mock.speaking = null;
      return true;
    }
    default:
      return null;
  }
}

Object.assign(window, {
  __TAURI_INTERNALS__: {
    metadata: { currentWindow: { label }, currentWebview: { label } },
    invoke,
    transformCallback(cb: Cb) {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc: (p: string) => p,
  },
  __TAURI_EVENT_PLUGIN_INTERNALS__: {
    unregisterListener(event: string, id: number) {
      listeners.get(event)?.delete(id);
    },
  },
  __mock: mock,
});
