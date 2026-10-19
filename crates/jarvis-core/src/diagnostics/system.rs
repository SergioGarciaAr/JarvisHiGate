//! System load and storage analysis, from the shared telemetry snapshot.

use super::finding::{Confidence as C, Evidence, Finding, FindingKind as K, Severity as S, gib, pct};
use crate::telemetry::TelemetrySnapshot;

const GIB: u64 = 1024 * 1024 * 1024;

pub fn analyze_system(s: &TelemetrySnapshot) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some([one, five, fifteen]) = s.load_avg {
        let cores = s.cpu_cores.max(1) as f64;
        let ratio = five / cores;
        let mut evidence = vec![
            Evidence::new("Carga media (1 · 5 · 15 min)", format!("{one:.2} · {five:.2} · {fifteen:.2}"), "sysinfo"),
            Evidence::new("Núcleos", s.cpu_cores.to_string(), "sysinfo"),
        ];
        if let Some(cpu) = s.cpu_percent {
            evidence.push(Evidence::new("CPU en este momento", format!("{cpu:.0} %"), "sysinfo"));
        }
        out.push(if ratio > 2.0 {
            Finding {
                id: "sys.load.high",
                kind: K::Warning,
                severity: S::Medium,
                title: "Carga de CPU sostenida alta".into(),
                evidence,
                confidence: C::Medium,
                recommendation: "Hay más trabajo en cola del que la CPU puede atender. Revisa en Monitor de Actividad \
(pestaña CPU) qué proceso la ocupa."
                    .into(),
                proposed_action: None,
            }
        } else {
            Finding {
                id: "sys.load.ok",
                kind: K::Observation,
                severity: if ratio > 1.0 { S::Low } else { S::Info },
                title: if ratio > 1.0 { "Carga de CPU moderada".into() } else { "Carga de CPU normal".into() },
                evidence,
                confidence: C::Medium,
                recommendation: "No hace falta ninguna acción.".into(),
                proposed_action: None,
            }
        });
    }
    let days = s.uptime_secs / 86_400;
    if days >= 30 {
        out.push(Finding {
            id: "sys.uptime.long",
            kind: K::Observation,
            severity: S::Low,
            title: format!("Sin reiniciar desde hace {days} días"),
            evidence: vec![Evidence::new("Tiempo encendido", format!("{days} días"), "sysinfo")],
            confidence: C::Low,
            recommendation: "Reiniciar de vez en cuando aplica actualizaciones pendientes y libera recursos; no es obligatorio.".into(),
            proposed_action: None,
        });
    }
    out
}

pub fn analyze_storage(s: &TelemetrySnapshot) -> Vec<Finding> {
    let Some(disk) = &s.disk else { return Vec::new() };
    let free = disk.available_bytes;
    let total = disk.total_bytes;
    let evidence = vec![Evidence::new(
        format!("Volumen {}", disk.mount),
        format!("{} libres de {} ({})", gib(free), gib(total), pct(free, total)),
        "sysinfo",
    )];
    let free_ratio = if total > 0 { free as f64 / total as f64 } else { 1.0 };
    let finding = if free_ratio < 0.05 || free < 5 * GIB {
        Finding {
            id: "disk.critical",
            kind: K::Problem,
            severity: S::High,
            title: "Disco casi lleno".into(),
            evidence,
            confidence: C::High,
            recommendation: "Con tan poco espacio macOS no puede crear swap ni instalar actualizaciones. Libera espacio \
desde Ajustes del Sistema › General › Almacenamiento."
                .into(),
            proposed_action: Some("Revisar Almacenamiento y eliminar manualmente lo que no necesites".into()),
        }
    } else if free_ratio < 0.10 || free < 15 * GIB {
        Finding {
            id: "disk.low",
            kind: K::Warning,
            severity: S::Medium,
            title: "Poco espacio libre en disco".into(),
            evidence,
            confidence: C::High,
            recommendation: "Conviene liberar espacio antes de que afecte al rendimiento o a las actualizaciones.".into(),
            proposed_action: None,
        }
    } else {
        Finding {
            id: "disk.ok",
            kind: K::Observation,
            severity: S::Info,
            title: "Espacio en disco suficiente".into(),
            evidence,
            confidence: C::High,
            recommendation: "No hace falta ninguna acción.".into(),
            proposed_action: None,
        }
    };
    vec![finding]
}
