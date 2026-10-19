import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { events, ipc } from "./ipc";
import type { Jarvis } from "./useJarvis";
import type { TtsEngineId, VoiceStatus } from "./types";
import { answerToSpeak, initialVoice, latestRequestId, voiceReducer } from "./voice";

export function useVoice(jarvis: Jarvis) {
  const [session, dispatch] = useReducer(voiceReducer, initialVoice);
  const [status, setStatus] = useState<VoiceStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [autospeak, setAutospeakState] = useState(false);
  const [voice, setVoiceState] = useState<string | null>(null);
  const [engine, setEngineState] = useState<TtsEngineId>("say");
  const [neuralVoice, setNeuralVoiceState] = useState<string | null>(null);
  const spoken = useRef(new Set<string>());
  const awaiting = useRef<string | null>(null);
  const stateRef = useRef(session.state);
  stateRef.current = session.state;

  const refreshStatus = useCallback(
    () =>
      ipc
        .voiceStatus()
        .then((s) => {
          setStatus(s);
          setAutospeakState(s.autospeak);
          setVoiceState(s.tts.selected);
          setEngineState(s.engine);
          setNeuralVoiceState(s.neural_voice || null);
        })
        .catch((e) => setStatusError(String(e))),
    [],
  );

  useEffect(() => {
    void refreshStatus();
    const p = events.voice((e) => {
      switch (e.kind) {
        case "started":
          dispatch({ type: "speak_started", utterance: e.utterance, requestId: e.request_id });
          break;
        case "finished":
          dispatch({ type: "speak_finished", utterance: e.utterance });
          break;
        case "stopped":
          dispatch({ type: "speak_stopped", utterance: e.utterance });
          break;
        case "error":
          dispatch({ type: "fail", message: e.message });
          break;
      }
    });
    return () => void p.then((f) => f());
  }, [refreshStatus]);

  const speak = useCallback(async (requestId: string, text: string) => {
    spoken.current.add(requestId);
    try {
      await ipc.voiceSpeak(requestId, text);
    } catch (e) {
      dispatch({ type: "fail", message: String(e) });
    }
  }, []);

  // Follow the latest request when answers are read aloud automatically.
  const latest = latestRequestId(jarvis.entries);
  const latestEntry = jarvis.entries.find((e) => e.id === latest);
  const answer = answerToSpeak(jarvis.entries);
  useEffect(() => {
    if (!autospeak || !latest || !latestEntry) return;
    const inFlight = latestEntry.type === "pending" || (latestEntry.type === "assistant" && latestEntry.status === "streaming");
    if (inFlight && awaiting.current !== latest) {
      awaiting.current = latest;
      dispatch({ type: "awaiting_answer", requestId: latest });
      return;
    }
    if (answer && answer.requestId === latest && !spoken.current.has(latest)) {
      void speak(answer.requestId, answer.text);
      return;
    }
    // Cancelled, failed, or not a chat answer (diagnostic, help…): nothing to read.
    if (!inFlight && !answer) dispatch({ type: "answer_ended", requestId: latest });
  }, [autospeak, latest, latestEntry, answer, speak]);

  const stop = useCallback(async () => {
    const wasThinking = stateRef.current === "thinking";
    await ipc.voiceStop();
    if (wasThinking) dispatch({ type: "cancel" });
  }, []);

  // Esc stops speech from anywhere in the window (the composer also cancels the request).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && (stateRef.current === "speaking" || stateRef.current === "thinking")) void stop();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [stop]);

  const setAutospeak = useCallback(async (enabled: boolean) => {
    setAutospeakState(enabled);
    // Answers that already exist are not read when enabling.
    for (const e of jarvis.entries) spoken.current.add(e.id);
    if (!enabled) dispatch({ type: "reset" });
    await ipc.voiceSetAutospeak(enabled);
  }, [jarvis.entries]);

  const setVoice = useCallback(async (name: string) => {
    setVoiceState(name);
    try {
      await ipc.voiceSetVoice(name);
    } catch (e) {
      dispatch({ type: "fail", message: String(e) });
    }
  }, []);

  const setEngine = useCallback(
    async (id: TtsEngineId) => {
      setEngineState(id);
      try {
        await ipc.voiceSetEngine(id);
      } catch (e) {
        dispatch({ type: "fail", message: String(e) });
      }
      await refreshStatus();
    },
    [refreshStatus],
  );

  const setNeuralVoice = useCallback(
    async (name: string) => {
      setNeuralVoiceState(name);
      try {
        await ipc.voiceSetNeuralVoice(name);
      } catch (e) {
        dispatch({ type: "fail", message: String(e) });
      }
    },
    [],
  );

  return {
    session,
    status,
    statusError,
    autospeak,
    voice,
    engine,
    neuralVoice,
    setAutospeak,
    setVoice,
    setEngine,
    setNeuralVoice,
    stop,
    /** Reads the latest finished answer on demand. */
    speakLast: answer ? () => void speak(answer.requestId, answer.text) : null,
  };
}

export type VoiceControls = ReturnType<typeof useVoice>;
