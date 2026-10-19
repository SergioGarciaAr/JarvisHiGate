// Voice session state. Every transition is driven by a real event: a user
// action, a chat outcome, or a `jarvis://voice` event from the backend.
// Transitions not listed in `TRANSITIONS` are ignored.

import type { Activity, Entry } from "./conversation";

export type VoiceState = "idle" | "listening" | "transcribing" | "thinking" | "speaking" | "cancelled" | "error";

export const VOICE_LABEL: Record<VoiceState, string> = {
  idle: "Inactivo",
  listening: "Escuchando",
  transcribing: "Transcribiendo",
  thinking: "Pensando",
  speaking: "Hablando",
  cancelled: "Cancelado",
  error: "Error",
};

export type VoiceAction =
  /** Push-to-talk pressed. Only dispatched when recognition is available (not in phase 3A). */
  | { type: "listen_start" }
  | { type: "listen_stop" }
  | { type: "transcript"; text: string }
  /** A request whose answer will be read aloud is in flight. */
  | { type: "awaiting_answer"; requestId: string }
  | { type: "answer_ended"; requestId: string }
  | { type: "speak_started"; utterance: number; requestId: string }
  | { type: "speak_finished"; utterance: number }
  | { type: "speak_stopped"; utterance: number }
  | { type: "fail"; message: string }
  | { type: "cancel" }
  | { type: "reset" };

export interface VoiceSession {
  state: VoiceState;
  utterance: number | null;
  requestId: string | null;
  error: string | null;
}

export const initialVoice: VoiceSession = { state: "idle", utterance: null, requestId: null, error: null };

type T = VoiceAction["type"];
const RESTING: VoiceState[] = ["idle", "cancelled", "error"];

/** Allowed action types per state. */
export const TRANSITIONS: Record<VoiceState, T[]> = {
  idle: ["listen_start", "awaiting_answer", "speak_started", "fail", "reset"],
  cancelled: ["listen_start", "awaiting_answer", "speak_started", "fail", "reset"],
  error: ["listen_start", "awaiting_answer", "speak_started", "reset"],
  listening: ["listen_stop", "cancel", "fail"],
  transcribing: ["transcript", "cancel", "fail"],
  thinking: ["answer_ended", "speak_started", "awaiting_answer", "cancel", "fail"],
  speaking: ["speak_finished", "speak_stopped", "speak_started", "awaiting_answer", "cancel", "fail"],
};

export function voiceReducer(s: VoiceSession, a: VoiceAction): VoiceSession {
  if (!TRANSITIONS[s.state].includes(a.type)) return s;
  switch (a.type) {
    case "listen_start":
      return { ...initialVoice, state: "listening" };
    case "listen_stop":
      return { ...s, state: "transcribing" };
    case "transcript":
      return { ...s, state: "thinking" };
    case "awaiting_answer":
      return { ...initialVoice, state: "thinking", requestId: a.requestId };
    case "answer_ended":
      return a.requestId === s.requestId ? initialVoice : s;
    case "speak_started":
      return { state: "speaking", utterance: a.utterance, requestId: a.requestId, error: null };
    case "speak_finished":
      return a.utterance === s.utterance ? initialVoice : s;
    case "speak_stopped":
      return a.utterance === s.utterance ? { ...initialVoice, state: "cancelled" } : s;
    case "fail":
      return { ...initialVoice, state: "error", error: a.message };
    case "cancel":
      return { ...initialVoice, state: "cancelled" };
    case "reset":
      return RESTING.includes(s.state) ? initialVoice : s;
  }
}

/** Id of the most recent request in this window's conversation. */
export function latestRequestId(entries: Entry[]): string | null {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.type === "assistant" || e.type === "run" || e.type === "pending") return e.id;
    if (e.type === "user") return e.id.replace(/^u-/, "");
  }
  return null;
}

/** The finished answer to read aloud, only if it belongs to the latest request. */
export function answerToSpeak(entries: Entry[]): { requestId: string; text: string } | null {
  const latest = latestRequestId(entries);
  const e = entries.find((x) => x.id === latest);
  if (!e || e.type !== "assistant" || e.status !== "done" || !e.text.trim()) return null;
  return { requestId: e.id, text: e.text };
}

/** Main activity shown by the HUD core, including speech when it is real. */
export function withVoice(activity: Activity, voice: VoiceSession, voiceName: string | null): Activity {
  if (voice.state === "speaking") return { core: "responding", label: `Hablando${voiceName ? ` · ${voiceName}` : ""}` };
  if (voice.state === "error" && activity.core === "idle") return { core: "error", label: "Error de voz" };
  return activity;
}
