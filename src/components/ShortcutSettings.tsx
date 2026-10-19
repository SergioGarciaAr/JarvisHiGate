import { useEffect, useState } from "react";
import { events, ipc } from "../lib/ipc";
import type { ShortcutStatus } from "../lib/types";

/** Shows the real registration state of the global shortcut and lets the user change it. */
export function ShortcutSettings() {
  const [status, setStatus] = useState<ShortcutStatus | null>(null);
  const [draft, setDraft] = useState("");
  const [lastError, setLastError] = useState<string | null>(null);

  useEffect(() => {
    void ipc.shortcutStatus().then((s) => {
      setStatus(s);
      setDraft(s.shortcut);
    });
    const p = events.shortcut(setStatus);
    return () => void p.then((f) => f());
  }, []);

  const apply = async () => {
    const result = await ipc.setShortcut(draft.trim());
    setLastError(result.registered ? null : result.error);
  };

  return (
    <section className="panel shortcut" aria-label="Atajo global">
      <h2 className="panel__title">Panel rápido</h2>
      {status && (
        <p className="ollama__status" role="status">
          <span className={`status-dot status-dot--${status.registered ? "ok" : "err"}`} aria-hidden="true" />
          {status.registered ? (
            <>
              Atajo <kbd>{status.shortcut.replace("Cmd", "⌘")}</kbd> activo
            </>
          ) : (
            "Atajo no registrado"
          )}
        </p>
      )}
      {status?.error && <p className="hint hint--error">{status.error}</p>}
      {status?.registered && (
        <p className="hint">
          Un atajo global captura la combinación en todas las aplicaciones (p. ej. ⌘J en VS Code o en el navegador).
        </p>
      )}
      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          void apply();
        }}
      >
        <label className="field field--grow">
          <span className="field__label">Atajo (p. ej. Cmd+J, Cmd+Shift+Space)</span>
          <input value={draft} onChange={(e) => setDraft(e.target.value)} spellCheck={false} />
        </label>
        <button className="btn" type="submit" disabled={!draft.trim() || draft.trim() === status?.shortcut}>
          Aplicar
        </button>
      </form>
      {lastError && (
        <p className="hint hint--error" role="alert">
          {lastError}
          {status?.registered && ` Se mantiene ${status.shortcut}.`}
        </p>
      )}
    </section>
  );
}
