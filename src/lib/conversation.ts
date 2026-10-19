import { applyProgress, initialRun, type RunState } from "./progressReducer";
import type { ChatEvent, Command, OllamaErrorInfo, OllamaHealth, ParseErrorInfo, ProgressEvent, SubmitOutcome } from "./types";

export type Entry =
  | { id: string; type: "user"; text: string }
  | {
      id: string;
      type: "assistant";
      text: string;
      model: string;
      status: "streaming" | "done" | "cancelled" | "error";
      error?: OllamaErrorInfo;
      sourceText: string;
    }
  | { id: string; type: "help"; text: string }
  | { id: string; type: "parse_error"; error: ParseErrorInfo }
  | { id: string; type: "not_available"; command: Command; phase: number; reason: string; alternative: string }
  | { id: string; type: "no_model"; message: string; recovery: string; sourceText: string }
  | { id: string; type: "run"; run: RunState; sourceText: string }
  /**
   * Placeholder between submitting and receiving the IPC outcome. The backend
   * starts work before replying, so chat/progress events can arrive first;
   * they are buffered here and replayed in order once the outcome arrives.
   */
  | { id: string; type: "pending"; early: (Action & { type: "chat_event" | "progress" })[] };

export type Action =
  | { type: "user"; id: string; text: string }
  | { type: "outcome"; id: string; outcome: SubmitOutcome; sourceText: string }
  | { type: "chat_event"; event: ChatEvent }
  | { type: "progress"; event: ProgressEvent }
  | { type: "clear" };

export function conversationReducer(entries: Entry[], action: Action): Entry[] {
  switch (action.type) {
    case "clear":
      return [];
    case "user":
      return [...entries, { id: `u-${action.id}`, type: "user", text: action.text }, { id: action.id, type: "pending", early: [] }];
    case "outcome": {
      const pending = entries.find((e) => e.type === "pending" && e.id === action.id);
      const rest = entries.filter((e) => e !== pending);
      const created = withOutcome(rest, action);
      return pending?.type === "pending" ? pending.early.reduce(conversationReducer, created) : created;
    }
    case "chat_event": {
      const ev = action.event;
      if (entries.some((e) => e.type === "pending" && e.id === ev.request_id)) return buffer(entries, ev.request_id, action);
      return entries.map((e) => {
        if (e.type !== "assistant" || e.id !== ev.request_id) return e;
        switch (ev.kind) {
          case "delta":
            return { ...e, text: e.text + ev.text };
          case "done":
            return { ...e, status: "done" };
          case "cancelled":
            return { ...e, status: "cancelled" };
          case "error":
            return { ...e, status: "error", error: ev.error };
        }
        return e;
      });
    }
    case "progress": {
      const ev = action.event;
      if (entries.some((e) => e.type === "pending" && e.id === ev.run_id)) return buffer(entries, ev.run_id, action);
      return entries.map((e) => (e.type === "run" && e.id === ev.run_id ? { ...e, run: applyProgress(e.run, ev) } : e));
    }
  }
}

function buffer(entries: Entry[], id: string, action: Action & { type: "chat_event" | "progress" }): Entry[] {
  return entries.map((e) => (e.type === "pending" && e.id === id ? { ...e, early: [...e.early, action] } : e));
}

function withOutcome(entries: Entry[], action: Action & { type: "outcome" }): Entry[] {
  const o = action.outcome;
  const id = action.id;
  switch (o.kind) {
    case "empty":
      return entries;
    case "help":
      return [...entries, { id, type: "help", text: o.text }];
    case "parse_error":
      return [...entries, { id, type: "parse_error", error: o.error }];
    case "not_available":
      return [...entries, { id, type: "not_available", command: o.command, phase: o.phase, reason: o.reason, alternative: o.alternative }];
    case "no_model":
      return [...entries, { id, type: "no_model", message: o.message, recovery: o.recovery, sourceText: action.sourceText }];
    case "chat_started":
      return [...entries, { id: o.request_id, type: "assistant", text: "", model: o.model, status: "streaming", sourceText: action.sourceText }];
    case "diagnostic_started":
      return [...entries, { id: o.request_id, type: "run", run: initialRun(o.request_id, `Diagnóstico ${o.scope}`, "diagnostico", null), sourceText: action.sourceText }];
    case "run_started":
      return [...entries, { id: o.request_id, type: "run", run: initialRun(o.request_id, o.goal, o.mode, o.model), sourceText: action.sourceText }];
  }
  return entries;
}

/** Id of the request still in flight, if any. */
export function activeRequest(entries: Entry[]): string | null {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.type === "assistant" && e.status === "streaming") return e.id;
    if (e.type === "run" && e.run.status === "running") return e.id;
  }
  return null;
}

export type CoreState = "offline" | "idle" | "thinking" | "responding" | "error";

export interface Activity {
  core: CoreState;
  label: string;
}

/** Derives what Jarvis is doing from real state only. */
export function deriveActivity(entries: Entry[], health: OllamaHealth | null): Activity {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.type === "assistant" && e.status === "streaming") {
      return e.text ? { core: "responding", label: `Respondiendo · ${e.model}` } : { core: "thinking", label: `Esperando al modelo · ${e.model}` };
    }
    if (e.type === "run" && e.run.status === "running") {
      if (e.run.mode === "diagnostico") {
        const tool = e.run.tools.find((t) => t.status === "running");
        return { core: "thinking", label: `Diagnóstico · ${tool?.name ?? e.run.phase ?? "iniciando"}` };
      }
      return { core: "thinking", label: e.run.phase ?? "Iniciando análisis" };
    }
  }
  if (health === null) return { core: "idle", label: "Comprobando Ollama…" };
  if (health.status === "unavailable") return { core: "offline", label: "Ollama no disponible" };
  if (health.status === "no_models") return { core: "offline", label: "Sin modelos instalados" };
  const last = entries[entries.length - 1];
  if (last && ((last.type === "assistant" && last.status === "error") || (last.type === "run" && last.run.status === "failed") || last.type === "no_model")) {
    return { core: "error", label: "Error en la última operación" };
  }
  return { core: "idle", label: "En espera" };
}
