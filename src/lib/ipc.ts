import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ChatEvent,
  OllamaHealth,
  ProgressEvent,
  Settings,
  ShortcutStatus,
  SubmitOutcome,
  TelemetrySnapshot,
  VoiceEvent,
  VoiceStatus,
} from "./types";

export const ipc = {
  ollamaHealth: () => invoke<OllamaHealth>("ollama_health"),
  submit: (requestId: string, text: string, model: string | null) =>
    invoke<SubmitOutcome>("submit_input", { requestId, text, model }),
  cancel: (requestId: string) => invoke<boolean>("cancel_request", { requestId }),
  resetChat: () => invoke<void>("reset_chat"),
  getSettings: () => invoke<Settings>("get_settings"),
  setModel: (model: string) => invoke<void>("set_model", { model }),
  shortcutStatus: () => invoke<ShortcutStatus>("shortcut_status"),
  setShortcut: (shortcut: string) => invoke<ShortcutStatus>("set_shortcut", { shortcut }),
  hideQuickPanel: () => invoke<void>("hide_quick_panel"),
  showMain: () => invoke<void>("show_main"),
  voiceStatus: () => invoke<VoiceStatus>("voice_status"),
  voiceSetVoice: (name: string) => invoke<void>("voice_set_voice", { name }),
  voiceSetAutospeak: (enabled: boolean) => invoke<void>("voice_set_autospeak", { enabled }),
  voiceSpeak: (requestId: string, text: string) => invoke<number>("voice_speak", { requestId, text }),
  voiceStop: () => invoke<boolean>("voice_stop"),
  voiceSetEngine: (engine: string) => invoke<void>("voice_set_engine", { engine }),
  voiceSetNeuralVoice: (name: string) => invoke<void>("voice_set_neural_voice", { name }),
};

export const events = {
  chat: (cb: (e: ChatEvent) => void): Promise<UnlistenFn> => listen<ChatEvent>("jarvis://chat", (e) => cb(e.payload)),
  progress: (cb: (e: ProgressEvent) => void): Promise<UnlistenFn> =>
    listen<ProgressEvent>("jarvis://progress", (e) => cb(e.payload)),
  telemetry: (cb: (e: TelemetrySnapshot) => void): Promise<UnlistenFn> =>
    listen<TelemetrySnapshot>("jarvis://telemetry", (e) => cb(e.payload)),
  shortcut: (cb: (e: ShortcutStatus) => void): Promise<UnlistenFn> =>
    listen<ShortcutStatus>("jarvis://shortcut", (e) => cb(e.payload)),
  voice: (cb: (e: VoiceEvent) => void): Promise<UnlistenFn> => listen<VoiceEvent>("jarvis://voice", (e) => cb(e.payload)),
};

export function newRequestId(): string {
  return crypto.randomUUID();
}
