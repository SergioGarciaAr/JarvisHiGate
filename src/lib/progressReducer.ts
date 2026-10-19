import type { DiagnosticReport, ProgressEvent, ThinkMode, ThinkResult } from "./types";

export type RunMode = ThinkMode | "diagnostico";

export type RunStatus = "running" | "completed" | "failed" | "cancelled";

export interface ToolUse {
  name: string;
  attempt: number;
  status: "running" | "ok" | "failed";
  detail?: string;
}

export interface RunState {
  runId: string;
  goal: string;
  mode: RunMode;
  /** null for runs that do not use the model (diagnostics). */
  model: string | null;
  status: RunStatus;
  phase: string | null;
  planSummary: string | null;
  tools: ToolUse[];
  result: ThinkResult | null;
  report: DiagnosticReport | null;
  errors: string[];
  retries: number;
  nextStep: string | null;
  events: ProgressEvent[];
}

export function initialRun(runId: string, goal: string, mode: RunMode, model: string | null): RunState {
  return {
    runId,
    goal,
    mode,
    model,
    status: "running",
    phase: null,
    planSummary: null,
    tools: [],
    result: null,
    report: null,
    errors: [],
    retries: 0,
    nextStep: null,
    events: [],
  };
}

/** Applies one real progress event to the run state. Events are ordered by `seq`. */
export function applyProgress(state: RunState, ev: ProgressEvent): RunState {
  if (ev.run_id !== state.runId || state.events.some((e) => e.id === ev.id)) return state;
  const events = [...state.events, ev].sort((a, b) => a.seq - b.seq);
  const next: RunState = { ...state, events };
  switch (ev.kind) {
    case "goal":
      next.goal = ev.summary;
      break;
    case "phase":
      next.phase = ev.summary;
      break;
    case "plan":
      next.planSummary = ev.summary;
      break;
    case "tool_call": {
      const attempt = (ev.data as { attempt?: number } | undefined)?.attempt ?? 1;
      next.tools = [...state.tools, { name: ev.summary, attempt, status: "running" }];
      break;
    }
    case "tool_result": {
      const tools = [...state.tools];
      const last = tools.length - 1;
      if (last >= 0) {
        tools[last] = { ...tools[last], status: ev.status === "ok" ? "ok" : "failed", detail: ev.summary };
      }
      next.tools = tools;
      break;
    }
    case "result":
      if (state.mode === "diagnostico") next.report = (ev.data as DiagnosticReport | undefined) ?? null;
      else next.result = (ev.data as ThinkResult | undefined) ?? null;
      break;
    case "error":
      next.errors = [...state.errors, ev.summary];
      break;
    case "retry":
      next.retries = state.retries + 1;
      break;
    case "next_step":
      next.nextStep = ev.summary;
      break;
    case "completed":
      next.status = ev.status === "ok" ? "completed" : "failed";
      break;
    case "cancelled":
      next.status = "cancelled";
      break;
  }
  return next;
}
