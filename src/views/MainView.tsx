import { ChatLog } from "../components/ChatLog";
import { Composer } from "../components/Composer";
import { Core } from "../components/Core";
import { MetricsPanel } from "../components/MetricsPanel";
import { OllamaStatus } from "../components/OllamaStatus";
import { ShortcutSettings } from "../components/ShortcutSettings";
import { useJarvis } from "../lib/useJarvis";
import { useVoice } from "../lib/useVoice";
import { withVoice } from "../lib/voice";
import { VoiceControl } from "../components/VoiceControl";

const EXAMPLES = ["jarvis ayuda", "jarvis diagnostico --full", "jarvis piensa --plan organizar mi semana", "-- analiza cómo reducir el tiempo de arranque de mi Mac"];

export function MainView() {
  const jarvis = useJarvis();
  const voice = useVoice(jarvis);
  const activity = withVoice(jarvis.activity, voice.session, voice.voice);

  return (
    <div className="hud">
      <aside className="hud__side">
        <div className="brand">
          <span className="brand__mark" aria-hidden="true" />
          <h1>JARVIS</h1>
        </div>
        <OllamaStatus jarvis={jarvis} />
        <MetricsPanel />
        <ShortcutSettings />
      </aside>

      <main className="hud__main">
        <header className="hud__top">
          <div className="core-wrap">
            <Core state={activity.core} size={132} />
          </div>
          <div>
            <p className="activity" role="status" aria-live="polite">
              <span className={`status-dot status-dot--${activity.core}`} aria-hidden="true" />
              {activity.label}
            </p>
            <p className="muted small">Modo solo lectura · el modelo no ejecuta acciones en el sistema</p>
          </div>
          <VoiceControl voice={voice} />
          <button className="btn btn--ghost btn--small" onClick={() => void jarvis.clear()} disabled={!!jarvis.pending}>
            Nueva sesión
          </button>
        </header>

        <ChatLog
          jarvis={jarvis}
          emptyHint={
            <div className="empty">
              <p>Escribe un mensaje para conversar con el modelo local, o prueba:</p>
              <ul>
                {EXAMPLES.map((ex) => (
                  <li key={ex}>
                    <button className="link" onClick={() => void jarvis.submit(ex)} disabled={!!jarvis.pending}>
                      <code>{ex}</code>
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          }
        />
        <Composer jarvis={jarvis} autoFocus />
      </main>
    </div>
  );
}
