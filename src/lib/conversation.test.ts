import { describe, expect, it } from "vitest";
import { activeRequest, conversationReducer, deriveActivity, type Entry } from "./conversation";
import { applyProgress, initialRun } from "./progressReducer";
import type { EventKind, EventStatus, OllamaHealth, ProgressEvent } from "./types";

const available: OllamaHealth = { status: "available", version: "0.40.6", models: [{ name: "m", size: 1, capabilities: [] }] };

let seq = 0;
function ev(kind: EventKind, status: EventStatus, summary = "", data?: unknown, run_id = "r1"): ProgressEvent {
  return { id: `e${seq}`, run_id, seq: seq++, kind, status, timestamp: new Date().toISOString(), summary, data };
}

describe("progress reducer", () => {
  it("builds run state from real events only", () => {
    seq = 0;
    let run = initialRun("r1", "g", "plan", "m");
    expect(run.status).toBe("running");
    expect(run.tools).toEqual([]);
    run = applyProgress(run, ev("goal", "ok", "organizar"));
    run = applyProgress(run, ev("phase", "running", "Consulta"));
    run = applyProgress(run, ev("tool_call", "running", "llm.ask", { attempt: 1 }));
    run = applyProgress(run, ev("tool_result", "failed", "json inválido"));
    run = applyProgress(run, ev("retry", "running", "Reintento 1 de 1"));
    run = applyProgress(run, ev("tool_call", "running", "llm.ask", { attempt: 2 }));
    run = applyProgress(run, ev("tool_result", "ok", "ok"));
    run = applyProgress(run, ev("next_step", "pending", "empieza"));
    run = applyProgress(run, ev("completed", "ok", "Completado"));
    expect(run.goal).toBe("organizar");
    expect(run.phase).toBe("Consulta");
    expect(run.retries).toBe(1);
    expect(run.tools.map((t) => [t.attempt, t.status])).toEqual([
      [1, "failed"],
      [2, "ok"],
    ]);
    expect(run.nextStep).toBe("empieza");
    expect(run.status).toBe("completed");
  });

  it("ignores events of other runs and duplicates", () => {
    seq = 0;
    const run = initialRun("r1", "g", "plan", "m");
    const other = applyProgress(run, ev("completed", "ok", "", undefined, "r2"));
    expect(other).toBe(run);
    const e = ev("phase", "running", "x");
    const once = applyProgress(run, e);
    expect(applyProgress(once, e)).toBe(once);
  });

  it("marks cancellation and failure", () => {
    seq = 0;
    expect(applyProgress(initialRun("r1", "g", "plan", "m"), ev("cancelled", "cancelled")).status).toBe("cancelled");
    expect(applyProgress(initialRun("r1", "g", "plan", "m"), ev("completed", "failed")).status).toBe("failed");
  });
});

describe("conversation reducer", () => {
  it("streams a chat answer and handles cancellation", () => {
    let s: Entry[] = [];
    s = conversationReducer(s, { type: "user", id: "1", text: "hola" });
    s = conversationReducer(s, { type: "outcome", id: "1", sourceText: "hola", outcome: { kind: "chat_started", request_id: "1", model: "m" } });
    expect(activeRequest(s)).toBe("1");
    expect(deriveActivity(s, available).core).toBe("thinking");
    s = conversationReducer(s, { type: "chat_event", event: { kind: "delta", request_id: "1", text: "Ho" } });
    s = conversationReducer(s, { type: "chat_event", event: { kind: "delta", request_id: "1", text: "la" } });
    expect(deriveActivity(s, available).core).toBe("responding");
    s = conversationReducer(s, { type: "chat_event", event: { kind: "cancelled", request_id: "1" } });
    const a = s[1];
    expect(a.type === "assistant" && a.text).toBe("Hola");
    expect(a.type === "assistant" && a.status).toBe("cancelled");
    expect(activeRequest(s)).toBeNull();
    expect(deriveActivity(s, available).core).toBe("idle");
  });

  it("surfaces Ollama errors with recovery", () => {
    let s: Entry[] = [];
    s = conversationReducer(s, { type: "outcome", id: "1", sourceText: "x", outcome: { kind: "chat_started", request_id: "1", model: "m" } });
    s = conversationReducer(s, {
      type: "chat_event",
      event: { kind: "error", request_id: "1", error: { kind: "unreachable", message: "no", recovery: "ollama serve", retryable: false } },
    });
    expect(deriveActivity(s, available).core).toBe("error");
    const a = s[0];
    expect(a.type === "assistant" && a.error?.recovery).toBe("ollama serve");
  });

  it("reports offline when Ollama is unavailable", () => {
    const down: OllamaHealth = { status: "unavailable", error: { kind: "unreachable", message: "x", recovery: "y", retryable: false } };
    expect(deriveActivity([], down)).toEqual({ core: "offline", label: "Ollama no disponible" });
    expect(deriveActivity([], null).core).toBe("idle");
  });

  it("records unavailable commands without pretending to run them", () => {
    const s = conversationReducer([], {
      type: "outcome",
      id: "1",
      sourceText: "jarvis claw --task x",
      outcome: { kind: "not_available", command: { command: "claw", task: "x" }, phase: 4, reason: "r", alternative: "a" },
    });
    expect(s[0].type).toBe("not_available");
    expect(activeRequest(s)).toBeNull();
  });
});

describe("diagnostic runs", () => {
  it("tracks real probes and stores the report", () => {
    seq = 0;
    let s: Entry[] = conversationReducer([], {
      type: "outcome",
      id: "d1",
      sourceText: "jarvis diagnostico --ram",
      outcome: { kind: "diagnostic_started", request_id: "d1", scope: "ram" },
    });
    expect(activeRequest(s)).toBe("d1");
    s = conversationReducer(s, { type: "progress", event: ev("tool_call", "running", "vm_stat", undefined, "d1") });
    expect(deriveActivity(s, available).label).toBe("Diagnóstico · vm_stat");
    // diagnostics work even if Ollama is down
    const down: OllamaHealth = { status: "unavailable", error: { kind: "unreachable", message: "x", recovery: "y", retryable: false } };
    expect(deriveActivity(s, down).core).toBe("thinking");
    s = conversationReducer(s, { type: "progress", event: ev("tool_result", "failed", "vm_stat: permiso denegado", undefined, "d1") });
    const report = { scope: "ram", generated_at: "", duration_ms: 10, system: null, memory: null, network: null, findings: [], gaps: [], notes: [] };
    s = conversationReducer(s, { type: "progress", event: ev("result", "ok", "0 problemas", report, "d1") });
    s = conversationReducer(s, { type: "progress", event: ev("completed", "ok", "", undefined, "d1") });
    const run = s[0].type === "run" ? s[0].run : null;
    expect(run?.model).toBeNull();
    expect(run?.report).toEqual(report);
    expect(run?.result).toBeNull();
    expect(run?.tools[0].status).toBe("failed");
    expect(activeRequest(s)).toBeNull();
  });
});

describe("events that arrive before the IPC outcome", () => {
  // The backend starts work before `submit_input` returns, so events may win the race.
  it("buffers early progress events and replays them in order", () => {
    seq = 0;
    let s: Entry[] = conversationReducer([], { type: "user", id: "d1", text: "jarvis diagnostico" });
    s = conversationReducer(s, { type: "progress", event: ev("goal", "ok", "Diagnóstico general (solo lectura)", undefined, "d1") });
    s = conversationReducer(s, { type: "progress", event: ev("tool_call", "running", "vm_stat", undefined, "d1") });
    expect(activeRequest(s)).toBeNull();
    s = conversationReducer(s, { type: "outcome", id: "d1", sourceText: "x", outcome: { kind: "diagnostic_started", request_id: "d1", scope: "general" } });
    expect(s.some((e) => e.type === "pending")).toBe(false);
    const run = s[1].type === "run" ? s[1].run : null;
    expect(run?.events.map((e) => e.kind)).toEqual(["goal", "tool_call"]);
    expect(run?.goal).toBe("Diagnóstico general (solo lectura)");
    expect(activeRequest(s)).toBe("d1");
  });

  it("buffers early chat deltas, even a full answer", () => {
    let s: Entry[] = conversationReducer([], { type: "user", id: "c1", text: "hola" });
    s = conversationReducer(s, { type: "chat_event", event: { kind: "delta", request_id: "c1", text: "Ho" } });
    s = conversationReducer(s, { type: "chat_event", event: { kind: "delta", request_id: "c1", text: "la" } });
    s = conversationReducer(s, { type: "chat_event", event: { kind: "done", request_id: "c1" } });
    s = conversationReducer(s, { type: "outcome", id: "c1", sourceText: "hola", outcome: { kind: "chat_started", request_id: "c1", model: "m" } });
    const a = s[1];
    expect(a.type === "assistant" && [a.text, a.status]).toEqual(["Hola", "done"]);
  });

  it("ignores events from requests issued by another window", () => {
    let s: Entry[] = conversationReducer([], { type: "user", id: "mine", text: "x" });
    s = conversationReducer(s, { type: "progress", event: ev("goal", "ok", "otro", undefined, "theirs") });
    expect(s[1].type === "pending" && s[1].early).toEqual([]);
  });

  it("drops the placeholder for outcomes without a run", () => {
    let s: Entry[] = conversationReducer([], { type: "user", id: "h", text: "jarvis ayuda" });
    s = conversationReducer(s, { type: "outcome", id: "h", sourceText: "jarvis ayuda", outcome: { kind: "help", text: "ayuda" } });
    expect(s.map((e) => e.type)).toEqual(["user", "help"]);
    s = conversationReducer(s, { type: "user", id: "e", text: " " });
    s = conversationReducer(s, { type: "outcome", id: "e", sourceText: "", outcome: { kind: "empty" } });
    expect(s.map((e) => e.type)).toEqual(["user", "help", "user"]);
  });
});
