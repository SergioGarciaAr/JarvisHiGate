import { describe, expect, it } from "vitest";
import { conversationReducer, type Entry } from "./conversation";
import { answerToSpeak, initialVoice, latestRequestId, TRANSITIONS, voiceReducer, type VoiceAction, type VoiceSession, type VoiceState } from "./voice";

const run = (actions: VoiceAction[], from: VoiceSession = initialVoice) => actions.reduce(voiceReducer, from);

describe("voice state machine", () => {
  it("autospeak flow: thinking → speaking → idle", () => {
    let s = run([{ type: "awaiting_answer", requestId: "r1" }]);
    expect(s.state).toBe("thinking");
    s = voiceReducer(s, { type: "speak_started", utterance: 1, requestId: "r1" });
    expect(s.state).toBe("speaking");
    s = voiceReducer(s, { type: "speak_finished", utterance: 1 });
    expect(s).toEqual(initialVoice);
  });

  it("Esc / stop while speaking ends in cancelled", () => {
    const s = run([{ type: "speak_started", utterance: 3, requestId: "r" }, { type: "speak_stopped", utterance: 3 }]);
    expect(s.state).toBe("cancelled");
    expect(run([{ type: "awaiting_answer", requestId: "r" }, { type: "cancel" }]).state).toBe("cancelled");
  });

  it("events of an older utterance do not affect the current one", () => {
    let s = run([{ type: "speak_started", utterance: 1, requestId: "a" }, { type: "speak_started", utterance: 2, requestId: "b" }]);
    s = voiceReducer(s, { type: "speak_stopped", utterance: 1 });
    expect(s).toMatchObject({ state: "speaking", utterance: 2 });
    s = voiceReducer(s, { type: "speak_finished", utterance: 1 });
    expect(s.state).toBe("speaking");
  });

  it("a new request while speaking goes to thinking and ignores the late stop of the old utterance", () => {
    let s = run([{ type: "speak_started", utterance: 1, requestId: "a" }, { type: "awaiting_answer", requestId: "b" }]);
    expect(s).toMatchObject({ state: "thinking", requestId: "b" });
    s = voiceReducer(s, { type: "speak_stopped", utterance: 1 });
    expect(s.state).toBe("thinking");
  });

  it("errors are explicit and recoverable", () => {
    let s = voiceReducer(initialVoice, { type: "fail", message: "say falló" });
    expect(s).toMatchObject({ state: "error", error: "say falló" });
    s = voiceReducer(s, { type: "reset" });
    expect(s).toEqual(initialVoice);
  });

  it("listening and transcribing are only reachable through real capture events", () => {
    let s = run([{ type: "listen_start" }]);
    expect(s.state).toBe("listening");
    s = run([{ type: "listen_stop" }, { type: "transcript", text: "hola" }], s);
    expect(s.state).toBe("thinking");
    // No transcript without having listened first.
    expect(voiceReducer(initialVoice, { type: "transcript", text: "x" })).toBe(initialVoice);
    expect(voiceReducer(initialVoice, { type: "listen_stop" })).toBe(initialVoice);
  });

  it("invalid transitions are ignored (same object)", () => {
    const states = Object.keys(TRANSITIONS) as VoiceState[];
    const all: VoiceAction[] = [
      { type: "listen_start" },
      { type: "listen_stop" },
      { type: "transcript", text: "t" },
      { type: "awaiting_answer", requestId: "r" },
      { type: "answer_ended", requestId: "r" },
      { type: "speak_started", utterance: 9, requestId: "r" },
      { type: "speak_finished", utterance: 9 },
      { type: "speak_stopped", utterance: 9 },
      { type: "fail", message: "m" },
      { type: "cancel" },
      { type: "reset" },
    ];
    for (const st of states) {
      const s: VoiceSession = { ...initialVoice, state: st };
      for (const a of all) {
        if (!TRANSITIONS[st].includes(a.type)) expect(voiceReducer(s, a)).toBe(s);
      }
    }
  });
});

describe("which answer may be read aloud", () => {
  function chat(id: string, text: string, final: "done" | "cancelled" | null): Entry[] {
    let s: Entry[] = conversationReducer([], { type: "user", id, text: "q" });
    s = conversationReducer(s, { type: "outcome", id, sourceText: "q", outcome: { kind: "chat_started", request_id: id, model: "m" } });
    s = conversationReducer(s, { type: "chat_event", event: { kind: "delta", request_id: id, text } });
    if (final) s = conversationReducer(s, { type: "chat_event", event: { kind: final, request_id: id } });
    return s;
  }

  it("only a finished answer of the latest request", () => {
    expect(answerToSpeak(chat("a", "hola", "done"))).toEqual({ requestId: "a", text: "hola" });
    expect(answerToSpeak(chat("a", "hola", null))).toBeNull();
    expect(answerToSpeak(chat("a", "hola", "cancelled"))).toBeNull();
  });

  it("an older answer is never spoken once a newer request exists", () => {
    let s = chat("a", "vieja", "done");
    s = conversationReducer(s, { type: "user", id: "b", text: "otra" });
    expect(latestRequestId(s)).toBe("b");
    expect(answerToSpeak(s)).toBeNull();
    s = conversationReducer(s, { type: "outcome", id: "b", sourceText: "x", outcome: { kind: "diagnostic_started", request_id: "b", scope: "ram" } });
    expect(answerToSpeak(s)).toBeNull();
  });
});

describe("HUD activity while speaking", () => {
  it("the core reflects real speech and voice errors", async () => {
    const { withVoice } = await import("./voice");
    const idle = { core: "idle" as const, label: "En espera" };
    expect(withVoice(idle, { ...initialVoice, state: "speaking", utterance: 1 }, "Mónica")).toEqual({ core: "responding", label: "Hablando · Mónica" });
    expect(withVoice(idle, { ...initialVoice, state: "error", error: "x" }, null).core).toBe("error");
    expect(withVoice(idle, initialVoice, "Mónica")).toBe(idle);
  });
});
