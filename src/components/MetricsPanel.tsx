import { useEffect, useState } from "react";
import { events } from "../lib/ipc";
import { formatBytes, formatRate, formatUptime } from "../lib/format";
import type { TelemetrySnapshot } from "../lib/types";

function Meter({ label, value, max, text }: { label: string; value: number | null; max: number; text: string }) {
  const pct = value == null || max <= 0 ? null : Math.min(100, (value / max) * 100);
  return (
    <div className="metric">
      <div className="metric__head">
        <span className="metric__label">{label}</span>
        <span className="metric__value">{text}</span>
      </div>
      <div
        className="metric__bar"
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct == null ? undefined : Math.round(pct)}
        aria-valuetext={text}
      >
        <div className={`metric__fill ${pct != null && pct > 85 ? "metric__fill--hot" : ""}`} style={{ width: `${pct ?? 0}%` }} />
      </div>
    </div>
  );
}

function reasonFor(snap: TelemetrySnapshot, metric: string): string | undefined {
  return snap.unavailable.find((u) => u.metric === metric)?.reason;
}

export function MetricsPanel() {
  const [snap, setSnap] = useState<TelemetrySnapshot | null>(null);

  useEffect(() => {
    const p = events.telemetry(setSnap);
    return () => void p.then((f) => f());
  }, []);

  if (!snap) {
    return (
      <section className="panel metrics" aria-label="Métricas del sistema">
        <h2 className="panel__title">Sistema</h2>
        <p className="muted">Esperando la primera lectura de telemetría…</p>
      </section>
    );
  }

  const memUsed = snap.mem_used;
  const disk = snap.disk;
  const diskUsed = disk ? disk.total_bytes - disk.available_bytes : null;
  const cpuText = snap.cpu_percent == null ? (reasonFor(snap, "cpu") ?? "no disponible") : `${snap.cpu_percent.toFixed(0)} %`;

  return (
    <section className="panel metrics" aria-label="Métricas del sistema">
      <h2 className="panel__title">
        Sistema <span className="live-dot" aria-hidden="true" />
      </h2>
      <Meter label={`CPU · ${snap.cpu_cores} núcleos`} value={snap.cpu_percent} max={100} text={cpuText} />
      <Meter
        label="Memoria"
        value={memUsed}
        max={snap.mem_total ?? 0}
        text={snap.mem_total == null ? (reasonFor(snap, "memoria") ?? "no disponible") : `${formatBytes(memUsed)} / ${formatBytes(snap.mem_total)}`}
      />
      <Meter
        label="Swap"
        value={snap.swap_used}
        max={snap.swap_total ?? 0}
        text={snap.swap_total ? `${formatBytes(snap.swap_used)} / ${formatBytes(snap.swap_total)}` : "sin swap en uso"}
      />
      <Meter
        label={`Disco · ${disk?.mount ?? "?"}`}
        value={diskUsed}
        max={disk?.total_bytes ?? 0}
        text={disk ? `${formatBytes(disk.available_bytes)} libres` : (reasonFor(snap, "disco") ?? "no disponible")}
      />
      <dl className="kv">
        <dt>Red ↓</dt>
        <dd>{snap.net_rx_per_sec == null ? (reasonFor(snap, "red") ?? "—") : formatRate(snap.net_rx_per_sec)}</dd>
        <dt>Red ↑</dt>
        <dd>{snap.net_tx_per_sec == null ? "—" : formatRate(snap.net_tx_per_sec)}</dd>
        <dt>Carga</dt>
        <dd>{snap.load_avg ? snap.load_avg.map((l) => l.toFixed(2)).join(" · ") : "no disponible"}</dd>
        <dt>Activo</dt>
        <dd>{formatUptime(snap.uptime_secs)}</dd>
        <dt>Equipo</dt>
        <dd title={snap.os_version ?? undefined}>{snap.hostname ?? "—"}</dd>
      </dl>
    </section>
  );
}
