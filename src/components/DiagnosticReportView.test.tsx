// Renders the report JSON produced by the Rust runner (see
// crates/jarvis-core/tests/diagnostics.rs, `write_or_check_contract_fixture`).
// The Rust test fails if the report changes shape without updating this fixture.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import fixture from "../lib/fixtures/diagnostic-report.json";
import { conversationReducer, type Entry } from "../lib/conversation";
import type { DiagnosticReport, ProgressEvent } from "../lib/types";
import { DiagnosticReportView } from "./DiagnosticReportView";
import { RunView } from "./RunView";

const report = fixture as unknown as DiagnosticReport;

function text(html: string): string {
  return html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
}

describe("DiagnosticReportView with the real Rust report", () => {
  it("renders every finding with kind, severity, confidence, evidence and recommendation", () => {
    const out = text(renderToStaticMarkup(<DiagnosticReportView report={report} />));
    expect(report.findings.length).toBeGreaterThan(0);
    for (const f of report.findings) {
      expect(out).toContain(f.title);
      expect(out).toContain(f.recommendation);
      for (const e of f.evidence) expect(out).toContain(e.value);
    }
    expect(out).toMatch(/Advertencia/);
    expect(out).toMatch(/Observación/);
    expect(out).toMatch(/Confianza (alta|media|baja)/);
  });

  it("shows missing data as unavailable with its reason, never as zero", () => {
    expect(report.memory?.swap).toBeNull();
    expect(report.memory?.vm).toBeNull();
    const html = renderToStaticMarkup(<DiagnosticReportView report={report} />);
    const out = text(html);
    expect(out).toContain("Datos no disponibles");
    for (const g of report.gaps) expect(out).toContain(g.reason);
    expect(out).toMatch(/Swap\s+no disponible/);
    expect(out).toMatch(/Comprimida · inmovilizada\s+no disponible/);
    expect(out).not.toMatch(/Swap\s+0 B/);
  });

  it("states the network caveat and that proposed actions are not executed", () => {
    const out = text(renderToStaticMarkup(<DiagnosticReportView report={report} />));
    expect(out).toContain("no demuestra por sí solo");
    expect(out).toContain("solo lectura");
    if (report.findings.some((f) => f.proposed_action)) expect(out).toContain("no se ejecuta automáticamente");
  });

  it("compact mode (quick panel) hides observations but keeps warnings", () => {
    const out = text(renderToStaticMarkup(<DiagnosticReportView report={report} compact />));
    for (const f of report.findings) {
      if (f.kind === "observation") expect(out).not.toContain(f.title);
      else expect(out).toContain(f.title);
    }
  });

  it("flows from progress events through the reducer into RunView", () => {
    let seq = 0;
    const ev = (kind: ProgressEvent["kind"], status: ProgressEvent["status"], summary: string, data?: unknown): ProgressEvent => ({
      id: `e${seq}`,
      run_id: "d1",
      seq: seq++,
      kind,
      status,
      timestamp: "2026-10-18T00:00:00Z",
      summary,
      data,
    });
    let s: Entry[] = conversationReducer([], { type: "user", id: "d1", text: "jarvis diagnostico --full" });
    s = conversationReducer(s, { type: "outcome", id: "d1", sourceText: "jarvis diagnostico --full", outcome: { kind: "diagnostic_started", request_id: "d1", scope: "full" } });
    for (const e of [
      ev("goal", "ok", "Diagnóstico completo (solo lectura)"),
      ev("phase", "running", "Memoria"),
      ev("tool_call", "running", "sysctl -n vm.swapusage"),
      ev("tool_result", "failed", "sysctl -n vm.swapusage: permiso denegado"),
      ev("result", "ok", "1 advertencias", report),
      ev("completed", "ok", "Diagnóstico completado"),
    ]) {
      s = conversationReducer(s, { type: "progress", event: e });
    }
    const entry = s.find((e) => e.type === "run");
    if (entry?.type !== "run") throw new Error("no run");
    const out = text(renderToStaticMarkup(<RunView run={entry.run} />));
    expect(out).toContain("Diagnóstico completo (solo lectura)");
    expect(out).toContain("1 ejecutadas · 1 sin datos");
    expect(out).toContain("Completado");
    expect(out).not.toContain("Modelo");
    expect(out).toContain(report.findings[0].title);
  });
});
