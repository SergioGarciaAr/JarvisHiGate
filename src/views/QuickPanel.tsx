import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ChatLog } from "../components/ChatLog";
import { Composer } from "../components/Composer";
import { Core } from "../components/Core";
import { ipc } from "../lib/ipc";
import { useJarvis } from "../lib/useJarvis";
import { useVoice } from "../lib/useVoice";
import { withVoice } from "../lib/voice";
import { VoiceControl } from "../components/VoiceControl";

export function QuickPanel() {
  const jarvis = useJarvis();
  const voice = useVoice(jarvis);
  const activity = withVoice(jarvis.activity, voice.session, voice.voice);

  // Refocus the input every time the panel is shown.
  useEffect(() => {
    const p = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) document.getElementById("composer-input")?.focus();
    });
    return () => void p.then((f) => f());
  }, []);

  return (
    <div className="quick">
      <header className="quick__head" data-tauri-drag-region>
        <Core state={activity.core} size={28} />
        <span className="activity small" role="status" aria-live="polite">
          {activity.label}
        </span>
        <span className="push-right" />
        <VoiceControl voice={voice} compact />
        <button className="btn btn--ghost btn--small" onClick={() => void ipc.showMain()}>
          Abrir Jarvis
        </button>
      </header>
      <ChatLog jarvis={jarvis} compact emptyHint={<p className="muted empty">Mismos comandos que el chat principal. Esc para cerrar.</p>} />
      <Composer
        jarvis={jarvis}
        autoFocus
        onEscape={() => {
          // While speech is active, Esc only stops it (handled by useVoice).
          if (voice.session.state !== "speaking" && voice.session.state !== "thinking") void ipc.hideQuickPanel();
        }}
      />
    </div>
  );
}
