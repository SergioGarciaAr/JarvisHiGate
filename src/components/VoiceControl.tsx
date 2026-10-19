import type { VoiceControls } from "../lib/useVoice";
import { VOICE_LABEL } from "../lib/voice";

function MicIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2">
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    </svg>
  );
}

/** Voice controls. Recognition stays disabled until a real engine exists. */
export function VoiceControl({ voice, compact }: { voice: VoiceControls; compact?: boolean }) {
  const { session, status, statusError } = voice;
  const speaking = session.state === "speaking";
  const engineInfo = status?.engines.find((e) => e.id === voice.engine);
  const ttsReady = voice.engine === "neural" ? !!engineInfo?.available : !!status?.tts.available;
  const sttReason = statusError ?? status?.stt.reason ?? "Comprobando el reconocimiento de voz…";
  const spanish = status?.tts.voices.filter((v) => v.locale.startsWith("es_")) ?? [];
  const others = status?.tts.voices.filter((v) => !v.locale.startsWith("es_")) ?? [];

  return (
    <div className={`voice ${compact ? "voice--compact" : ""}`} role="group" aria-label="Voz">
      <span className={`voice__state voice__state--${session.state}`} role="status" aria-live="polite" title={session.error ?? undefined}>
        <span className="voice__dot" aria-hidden="true" />
        {VOICE_LABEL[session.state]}
      </span>
      {!compact && (
        <>
          <button className="btn btn--ghost btn--small voice__mic" disabled={!status?.stt.available} title={sttReason} aria-label={`Pulsar para hablar: ${sttReason}`}>
            <MicIcon />
          </button>
          <label className="voice__autospeak" title={ttsReady ? "Leer en voz alta las respuestas del chat al terminar" : (status?.tts.reason ?? "")}>
            <input type="checkbox" checked={voice.autospeak} disabled={!ttsReady} onChange={(e) => void voice.setAutospeak(e.target.checked)} />
            Leer respuestas
          </label>
          {status && (
            <select
              className="voice__select voice__engine"
              aria-label="Motor de voz"
              value={voice.engine}
              title={engineInfo?.reason ?? engineInfo?.label}
              onChange={(e) => void voice.setEngine(e.target.value as "neural" | "say")}
            >
              {status.engines.map((e) => (
                <option key={e.id} value={e.id} disabled={!e.available && e.id !== voice.engine}>
                  {e.label}
                  {e.available ? "" : " (no disponible)"}
                </option>
              ))}
            </select>
          )}
          {ttsReady && voice.engine === "neural" && (status?.neural_voices.length ?? 0) > 0 && (
            <select className="voice__select" aria-label="Voz neuronal" value={voice.neuralVoice ?? ""} onChange={(e) => void voice.setNeuralVoice(e.target.value)}>
              {status!.neural_voices.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </select>
          )}
          {ttsReady && voice.engine === "say" && (
            <select className="voice__select" aria-label="Voz de lectura" value={voice.voice ?? ""} onChange={(e) => void voice.setVoice(e.target.value)}>
              {spanish.length > 0 && (
                <optgroup label="Español">
                  {spanish.map((v) => (
                    <option key={v.name} value={v.name}>
                      {v.name} · {v.locale}
                    </option>
                  ))}
                </optgroup>
              )}
              <optgroup label="Otras">
                {others.map((v) => (
                  <option key={v.name} value={v.name}>
                    {v.name} · {v.locale}
                  </option>
                ))}
              </optgroup>
            </select>
          )}
        </>
      )}
      {speaking ? (
        <button className="btn btn--danger btn--small voice__stop" onClick={() => void voice.stop()}>
          ■ Detener voz
        </button>
      ) : (
        !compact && (
          <button className="btn btn--ghost btn--small voice__speak" disabled={!ttsReady || !voice.speakLast} onClick={() => voice.speakLast?.()}>
            ▶ Leer última
          </button>
        )
      )}
      {!compact && engineInfo && !engineInfo.available && (
        <p className="voice__error" role="alert">
          {engineInfo.reason}
        </p>
      )}
      {session.error && !compact && (
        <p className="voice__error" role="alert">
          {session.error}
        </p>
      )}
    </div>
  );
}
