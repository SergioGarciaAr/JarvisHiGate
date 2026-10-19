import type { Jarvis } from "../lib/useJarvis";

export function OllamaStatus({ jarvis }: { jarvis: Jarvis }) {
  const { health, checking, model, setModel, refreshHealth } = jarvis;

  let dot = "pending";
  let title = "Comprobando Ollama…";
  let detail: string | null = null;
  if (health?.status === "available") {
    dot = "ok";
    title = `Ollama ${health.version}`;
  } else if (health?.status === "no_models") {
    dot = "warn";
    title = `Ollama ${health.version} · sin modelos`;
    detail = health.recovery;
  } else if (health?.status === "unavailable") {
    dot = "err";
    title = health.error.message;
    detail = health.error.recovery;
  }

  return (
    <section className="panel ollama" aria-label="Estado del modelo">
      <h2 className="panel__title">Modelo</h2>
      <p className="ollama__status" role="status" aria-live="polite">
        <span className={`status-dot status-dot--${dot}`} aria-hidden="true" />
        {title}
      </p>
      {detail && <p className="hint">{detail}</p>}
      {health?.status === "available" && (
        <label className="field">
          <span className="field__label">Modelo activo</span>
          <select value={model ?? ""} onChange={(e) => void setModel(e.target.value)} disabled={!!jarvis.pending}>
            {health.models.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {health?.status !== "available" && (
        <button className="btn" onClick={() => void refreshHealth()} disabled={checking}>
          {checking ? "Comprobando…" : "Reintentar"}
        </button>
      )}
    </section>
  );
}
