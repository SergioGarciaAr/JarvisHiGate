import { formatBytes } from "../lib/format";
import type { Confidence, DiagnosticReport, Finding, FindingKind, Severity } from "../lib/types";

const KIND_LABEL: Record<FindingKind, string> = {
  problem: "Problema confirmado",
  warning: "Advertencia",
  observation: "Observación",
};

const SEVERITY_LABEL: Record<Severity, string> = {
  info: "informativa",
  low: "baja",
  medium: "media",
  high: "alta",
  critical: "crítica",
};

const CONFIDENCE_LABEL: Record<Confidence, string> = { high: "alta", medium: "media", low: "baja" };

const PRESSURE_LABEL = { normal: "normal", warn: "advertencia", critical: "crítica" } as const;

function FindingCard({ f }: { f: Finding }) {
  return (
    <li className={`finding finding--${f.kind}`}>
      <header className="finding__head">
        <span className={`badge badge--${f.kind}`}>{KIND_LABEL[f.kind]}</span>
        <h4 className="finding__title">{f.title}</h4>
      </header>
      <p className="finding__meta">
        Gravedad <span className={`sev sev--${f.severity}`}>{SEVERITY_LABEL[f.severity]}</span> · Confianza{" "}
        {CONFIDENCE_LABEL[f.confidence]}
      </p>
      {f.evidence.length > 0 && (
        <dl className="evidence">
          {f.evidence.map((e, i) => (
            <div key={i} className="evidence__row">
              <dt>{e.label}</dt>
              <dd>
                {e.value} <span className="evidence__src">· {e.source}</span>
              </dd>
            </div>
          ))}
        </dl>
      )}
      <p className="finding__rec">
        <strong>Recomendación:</strong> {f.recommendation}
      </p>
      {f.proposed_action && (
        <p className="finding__action">
          <strong>Acción propuesta</strong> (no se ejecuta automáticamente): {f.proposed_action}
        </p>
      )}
    </li>
  );
}

function MemoryDetails({ report }: { report: DiagnosticReport }) {
  const m = report.memory;
  if (!m) return null;
  return (
    <details className="details">
      <summary>Datos de memoria</summary>
      <dl className="kv kv--wide">
        <dt>Total · disponible</dt>
        <dd>
          {formatBytes(m.total)} · {formatBytes(m.available)}
        </dd>
        <dt>Presión</dt>
        <dd>{m.pressure ? PRESSURE_LABEL[m.pressure] : "no disponible"}</dd>
        <dt>Libre según el kernel</dt>
        <dd>{m.free_level != null ? `${m.free_level} %` : "no disponible"}</dd>
        <dt>Swap</dt>
        <dd>{m.swap ? `${formatBytes(m.swap.used)} de ${formatBytes(m.swap.total)}` : "no disponible"}</dd>
        <dt>Comprimida · inmovilizada</dt>
        <dd>
          {m.vm ? `${formatBytes(m.vm.compressor_occupied * m.vm.page_size)} · ${formatBytes(m.vm.wired * m.vm.page_size)}` : "no disponible"}
        </dd>
      </dl>
      {m.processes && m.processes.top.length > 0 && (
        <table className="table">
          <caption>Procesos por memoria residente</caption>
          <thead>
            <tr>
              <th scope="col">Proceso</th>
              <th scope="col">PID</th>
              <th scope="col">Memoria</th>
            </tr>
          </thead>
          <tbody>
            {m.processes.top.map((p) => (
              <tr key={p.pid}>
                <td>{p.name}</td>
                <td>{p.pid}</td>
                <td>{formatBytes(p.memory)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </details>
  );
}

const TCP_LABEL = {
  connected: "conectado",
  refused: "rechazado (responde)",
  timeout: "tiempo agotado",
  unreachable: "inalcanzable",
  error: "error",
} as const;

function NetworkDetails({ report }: { report: DiagnosticReport }) {
  const n = report.network;
  if (!n) return null;
  const active = n.interfaces.filter((i) => i.has_routable_ip && !i.name.startsWith("lo"));
  return (
    <details className="details">
      <summary>Datos de red</summary>
      <dl className="kv kv--wide">
        <dt>Interfaces con IP</dt>
        <dd>{active.length ? active.map((i) => i.name).join(", ") : "ninguna"}</dd>
        <dt>Ruta por defecto</dt>
        <dd>
          {n.route.status === "default"
            ? `${n.route.gateway} vía ${n.route.interface ?? "?"}`
            : n.route.status === "no_default"
              ? "no existe"
              : "no disponible"}
        </dd>
        <dt>Servidores DNS</dt>
        <dd>{n.dns_servers ? n.dns_servers.join(", ") || "ninguno" : "no disponible"}</dd>
      </dl>
      <table className="table">
        <caption>Pruebas con tiempo límite</caption>
        <thead>
          <tr>
            <th scope="col">Prueba</th>
            <th scope="col">Resultado</th>
          </tr>
        </thead>
        <tbody>
          {n.dns.map((d) => (
            <tr key={`dns-${d.host}`}>
              <td>DNS {d.host}</td>
              <td className={d.error ? "bad" : "good"}>{d.error ? `falló: ${d.error}` : `${d.addresses[0]} · ${d.ms} ms`}</td>
            </tr>
          ))}
          {n.tcp.map((t) => (
            <tr key={`tcp-${t.target}`}>
              <td>
                TCP {t.label} <span className="muted">{t.target}</span>
              </td>
              <td className={t.outcome === "connected" || t.outcome === "refused" ? "good" : "bad"}>
                {TCP_LABEL[t.outcome]}
                {t.ms != null ? ` · ${t.ms} ms` : ""}
                {t.detail ? ` · ${t.detail}` : ""}
              </td>
            </tr>
          ))}
          {n.gateway && (
            <tr>
              <td>Ping router {n.gateway.gateway}</td>
              <td className={n.gateway.stats.received > 0 ? "good" : "bad"}>
                {n.gateway.stats.received}/{n.gateway.stats.transmitted}
                {n.gateway.stats.send_error ? ` · ${n.gateway.stats.send_error}` : ""}
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </details>
  );
}

export function DiagnosticReportView({ report, compact }: { report: DiagnosticReport; compact?: boolean }) {
  const count = (k: FindingKind) => report.findings.filter((f) => f.kind === k).length;
  const shown = compact ? report.findings.filter((f) => f.kind !== "observation") : report.findings;
  return (
    <div className="report">
      <p className="report__counts" aria-label="Resumen del diagnóstico">
        <span className="badge badge--problem">{count("problem")} problemas</span>
        <span className="badge badge--warning">{count("warning")} advertencias</span>
        <span className="badge badge--observation">{count("observation")} observaciones</span>
        <span className="muted small">{(report.duration_ms / 1000).toFixed(1)} s</span>
      </p>
      {compact && shown.length === 0 && <p className="muted">Sin problemas ni advertencias. Abre Jarvis para ver el informe completo.</p>}
      <ul className="findings">
        {shown.map((f, i) => (
          <FindingCard key={`${f.id}-${i}`} f={f} />
        ))}
      </ul>
      {report.gaps.length > 0 && (
        <section className="gaps" aria-label="Datos no disponibles">
          <h4>Datos no disponibles</h4>
          <ul>
            {report.gaps.map((g, i) => (
              <li key={i}>
                <strong>{g.what}</strong> <span className="muted">({g.source})</span>: {g.reason}
              </li>
            ))}
          </ul>
        </section>
      )}
      {!compact && (
        <>
          <MemoryDetails report={report} />
          <NetworkDetails report={report} />
          {report.notes.map((n, i) => (
            <p key={i} className="note">
              {n}
            </p>
          ))}
        </>
      )}
    </div>
  );
}
