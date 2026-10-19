import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { events, ipc, newRequestId } from "./ipc";
import { activeRequest, conversationReducer, deriveActivity } from "./conversation";
import type { OllamaHealth, SubmitOutcome } from "./types";

const HEALTH_INTERVAL_MS = 10_000;

export function useJarvis() {
  const [entries, dispatch] = useReducer(conversationReducer, []);
  const [health, setHealth] = useState<OllamaHealth | null>(null);
  const [model, setModelState] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const modelRef = useRef<string | null>(null);
  modelRef.current = model;

  const refreshHealth = useCallback(async () => {
    setChecking(true);
    try {
      const h = await ipc.ollamaHealth();
      setHealth(h);
      if (h.status === "available") {
        const names = h.models.map((m) => m.name);
        const saved = (await ipc.getSettings()).model;
        const current = modelRef.current;
        if (!current || !names.includes(current)) {
          setModelState(saved && names.includes(saved) ? saved : names[0]);
        }
      }
      return h;
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    void refreshHealth();
    const timer = setInterval(() => void refreshHealth(), HEALTH_INTERVAL_MS);
    const unlisten = [
      events.chat((event) => dispatch({ type: "chat_event", event })),
      events.progress((event) => dispatch({ type: "progress", event })),
    ];
    return () => {
      clearInterval(timer);
      unlisten.forEach((p) => void p.then((f) => f()));
    };
  }, [refreshHealth]);

  const submit = useCallback(async (text: string) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    const id = newRequestId();
    dispatch({ type: "user", id, text: trimmed });
    let outcome: SubmitOutcome;
    try {
      outcome = await ipc.submit(id, trimmed, modelRef.current);
    } catch (err) {
      outcome = {
        kind: "parse_error",
        error: { kind: "ipc", message: "No se pudo comunicar con el backend de Jarvis.", hint: String(err) },
      };
    }
    dispatch({ type: "outcome", id, outcome, sourceText: trimmed });
  }, []);

  const retry = useCallback(
    async (text: string) => {
      await refreshHealth();
      await submit(text);
    },
    [refreshHealth, submit],
  );

  const pending = activeRequest(entries);
  const cancel = useCallback(async () => {
    if (pending) await ipc.cancel(pending);
  }, [pending]);

  const setModel = useCallback(async (name: string) => {
    setModelState(name);
    await ipc.setModel(name);
  }, []);

  const clear = useCallback(async () => {
    await ipc.resetChat();
    dispatch({ type: "clear" });
  }, []);

  return {
    entries,
    health,
    checking,
    model,
    setModel,
    submit,
    retry,
    cancel,
    clear,
    refreshHealth,
    pending,
    activity: deriveActivity(entries, health),
  };
}

export type Jarvis = ReturnType<typeof useJarvis>;
