import { formatTime } from "../lib/format";
import type { RunState } from "../lib/progressReducer";
import type { EventKind, ThinkResult } from "../lib/types";
import { DiagnosticReportView } from "./DiagnosticReportView";

const KIND_LABEL: Record<EventKind, string> = {
  goal: "Objetivo",
  phase: "Fase",
  plan: "Plan",
  tool_call: "Herramienta",
  tool_result: "Resultado de herramienta",
  result: "Resultado",
  error: "Error",
  retry: "Reintento",
  next_step: "Próximo paso",
  completed: "Finalizado",
  cancelled: "Cancelado",
};

const MODE_LABEL = { plan: "Plan", comparar: "Comparación", analisis: "Análisis", diagnostico: "Diagnóstico" } as const;

function Result({ result }: { result: ThinkResult }) {
  switch (result.mode) {
    case "plan":
      return (
        <div className="result">
          <p>{result.resumen}</p>
          <ol className="result__steps">
            {result.pasos.map((p, i) => (
              <li key={i}>
                <strong>{p.titulo}</strong>
                <span>{p.detalle}</span>
              </li>
            ))}
          </ol>
          {result.riesgos.length > 0 && (
            <>
              <h4>Riesgos</h4>
              <ul>{result.riesgos.map((r, i) => <li key={i}>{r}</li>)}</ul>
            </>
          )}
        </div>
      );
    case "comparar":
      return (
        <div className="result">
          <div className="result__options">
            {result.opciones.map((o, i) => (
              <div key={i} className="option">
                <h4>{o.nombre}</h4>
                <ul className="pros">{o.ventajas.map((v, j) => <li key={j}>{v}</li>)}</ul>
                <ul className="cons">{o.inconvenientes.map((v, j) => <li key={j}>{v}</li>)}</ul>
              </div>
            ))}
          </div>
          <p>
            <strong>Recomendación:</strong> {result.recomendacion}
          </p>
          <p className="muted">{result.justificacion}</p>
        </div>
      );
    case "analisis":
      return (
        <div className="result">
          <p>{result.resumen}</p>
          <ul>{result.hallazgos.map((h, i) => <li key={i}>{h}</li>)}</ul>
        </div>
      );
  }
}

/** Diagnostics run many probes: show counts and the one in progress. */
function ToolCount({ run }: { run: RunState }) {
  const failed = run.tools.filter((t) => t.status === "failed").length;
  const current = run.tools.find((t) => t.status === "running");
  return (
    <>
      {run.tools.length} ejecutadas{failed > 0 ? ` · ${failed} sin datos` : ""}
      {current && <span className="chip chip--running">{current.name}</span>}
    </>
  );
}

interface Props {
  run: RunState;
  onCancel?: () => void;
  onRetry?: () => void;
  compact?: boolean;
}

/** Progress of one analysis run, built only from received events. */
export function RunView({ run, onCancel, onRetry, compact }: Props) {
  const statusLabel = { running: "En curso", completed: "Completado", failed: "Error", cancelled: "Cancelado" }[run.status];
  return (
    <article className={`run run--${run.status}`} aria-busy={run.status === "running"}>
      <header className="run__head">
        <span className="tag">{MODE_LABEL[run.mode]}</span>
        <span className="run__goal">{run.goal}</span>
        <span className={`run__status run__status--${run.status}`}>{statusLabel}</span>
      </header>
      <dl className="run__summary">
        <dt>Fase</dt>
        <dd>{run.phase ?? "—"}</dd>
        {run.model && (
          <>
            <dt>Modelo</dt>
            <dd>{run.model}</dd>
          </>
        )}
        <dt>Herramientas</dt>
        <dd>
          {run.mode === "diagnostico" ? (
            <ToolCount run={run} />
          ) : run.tools.length === 0
            ? "—"
            : run.tools.map((t, i) => (
                <span key={i} className={`chip chip--${t.status}`}>
                  {t.name}
                  {t.attempt > 1 ? ` #${t.attempt}` : ""}
                </span>
              ))}
        </dd>
        {run.retries > 0 && (
          <>
            <dt>Reintentos</dt>
            <dd>{run.retries}</dd>
          </>
        )}
      </dl>

      {run.result && <Result result={run.result} />}
      {run.report && <DiagnosticReportView report={run.report} compact={compact} />}
      {run.errors.map((e, i) => (
        <p key={i} className="error-text" role="alert">
          {e}
        </p>
      ))}
      {run.nextStep && (
        <p className="next-step">
          <span className="tag">Próximo paso</span> {run.nextStep}
        </p>
      )}

      {!compact && (
        <details className="timeline">
          <summary>Eventos ({run.events.length})</summary>
          <ol>
            {run.events.map((ev) => (
              <li key={ev.id} className={`timeline__item timeline__item--${ev.status}`}>
                <time dateTime={ev.timestamp}>{formatTime(ev.timestamp)}</time>
                <span className="timeline__kind">{KIND_LABEL[ev.kind]}</span>
                <span className="timeline__summary">{ev.kind === "result" && run.mode !== "diagnostico" ? "Datos estructurados recibidos" : ev.summary}</span>
              </li>
            ))}
          </ol>
        </details>
      )}

      <div className="actions">
        {run.status === "running" && onCancel && (
          <button className="btn btn--danger" onClick={onCancel}>
            Cancelar
          </button>
        )}
        {(run.status === "failed" || run.status === "cancelled") && onRetry && (
          <button className="btn" onClick={onRetry}>
            Reintentar
          </button>
        )}
      </div>
    </article>
  );
}
