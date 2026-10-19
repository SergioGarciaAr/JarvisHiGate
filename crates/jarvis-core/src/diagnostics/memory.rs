//! Memory analysis. Pure functions over collected data.

use serde::Serialize;

use super::finding::{Confidence as C, Evidence, Finding, FindingKind as K, Severity as S, gib, pct};
use super::parse::{PressureLevel, SwapUsage, VmStat};
use crate::telemetry::ProcessSummary;

pub const NOTE: &str = "En macOS la memoria «usada» incluye caché de archivos y memoria comprimida, así que un porcentaje alto \
es normal. La presión de memoria que informa el kernel es el indicador más fiable.";

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryData {
    pub total: Option<u64>,
    pub used: Option<u64>,
    pub available: Option<u64>,
    pub pressure: Option<PressureLevel>,
    /// `kern.memorystatus_level`: % of memory the kernel considers free.
    pub free_level: Option<u8>,
    pub vm: Option<VmStat>,
    pub swap: Option<SwapUsage>,
    pub processes: Option<ProcessSummary>,
}

const SYSCTL_PRESSURE: &str = "sysctl kern.memorystatus_vm_pressure_level";

fn pressure_label(p: PressureLevel) -> &'static str {
    match p {
        PressureLevel::Normal => "normal",
        PressureLevel::Warn => "advertencia",
        PressureLevel::Critical => "crítica",
    }
}

pub fn analyze_memory(d: &MemoryData) -> Vec<Finding> {
    let mut out = Vec::new();

    if let Some(p) = d.pressure {
        let mut evidence = vec![Evidence::new("Presión de memoria", pressure_label(p), SYSCTL_PRESSURE)];
        if let Some(level) = d.free_level {
            evidence.push(Evidence::new("Memoria libre según el kernel", format!("{level} %"), "sysctl kern.memorystatus_level"));
        }
        if let (Some(avail), Some(total)) = (d.available, d.total) {
            evidence.push(Evidence::new("Disponible", format!("{} de {}", gib(avail), gib(total)), "sysinfo"));
        }
        out.push(match p {
            PressureLevel::Critical => Finding {
                id: "mem.pressure.critical",
                kind: K::Problem,
                severity: S::High,
                title: "Presión de memoria crítica".into(),
                evidence,
                confidence: C::High,
                recommendation: "Cierra las aplicaciones que no estés usando, empezando por las que más memoria consumen \
(ver lista de procesos). Si ocurre a menudo, el equipo necesita más RAM para tu carga de trabajo."
                    .into(),
                proposed_action: Some("Cerrar manualmente las aplicaciones de mayor consumo".into()),
            },
            PressureLevel::Warn => Finding {
                id: "mem.pressure.warn",
                kind: K::Warning,
                severity: S::Medium,
                title: "Presión de memoria elevada".into(),
                evidence,
                confidence: C::High,
                recommendation: "macOS está comprimiendo memoria o usando swap para mantenerse. Cerrar aplicaciones \
o pestañas que no uses reducirá la presión."
                    .into(),
                proposed_action: None,
            },
            PressureLevel::Normal => Finding {
                id: "mem.pressure.normal",
                kind: K::Observation,
                severity: S::Info,
                title: "Presión de memoria normal".into(),
                evidence,
                confidence: C::High,
                recommendation: "No hace falta ninguna acción.".into(),
                proposed_action: None,
            },
        });
    }

    if let Some(swap) = &d.swap {
        let mut evidence = vec![Evidence::new(
            "Swap",
            format!("{} usados de {}", gib(swap.used), gib(swap.total)),
            "sysctl vm.swapusage",
        )];
        if let Some(vm) = &d.vm {
            evidence.push(Evidence::new("Swapouts desde el arranque", vm.swapouts.to_string(), "vm_stat"));
        }
        let under_pressure = matches!(d.pressure, Some(PressureLevel::Warn | PressureLevel::Critical));
        out.push(if swap.used == 0 {
            Finding {
                id: "mem.swap.none",
                kind: K::Observation,
                severity: S::Info,
                title: "Sin uso de swap".into(),
                evidence,
                confidence: C::High,
                recommendation: "No hace falta ninguna acción.".into(),
                proposed_action: None,
            }
        } else if under_pressure {
            Finding {
                id: "mem.swap.pressure",
                kind: K::Warning,
                severity: S::Medium,
                title: "Uso de swap con presión de memoria".into(),
                evidence,
                confidence: C::Medium,
                recommendation: "El sistema está moviendo memoria a disco porque falta RAM, lo que ralentiza el equipo. \
Reduce las aplicaciones abiertas."
                    .into(),
                proposed_action: None,
            }
        } else {
            Finding {
                id: "mem.swap.residual",
                kind: K::Observation,
                severity: S::Low,
                title: "Swap en uso con presión normal".into(),
                evidence,
                confidence: C::Medium,
                recommendation: "macOS conserva el swap tras picos de uso; solo es preocupante si la presión de memoria sube."
                    .into(),
                proposed_action: None,
            }
        });
    }

    if let (Some(vm), Some(total)) = (&d.vm, d.total) {
        let occupied = vm.bytes(vm.compressor_occupied);
        if total > 0 && occupied * 4 > total {
            out.push(Finding {
                id: "mem.compressor.high",
                kind: K::Warning,
                severity: S::Low,
                title: "Mucha memoria comprimida".into(),
                evidence: vec![
                    Evidence::new("Ocupada por el compresor", format!("{} ({})", gib(occupied), pct(occupied, total)), "vm_stat"),
                    Evidence::new("Datos comprimidos", gib(vm.bytes(vm.compressed_stored)), "vm_stat"),
                ],
                confidence: C::Medium,
                recommendation: "Comprimir memoria es normal en macOS, pero un volumen alto indica que la RAM va justa. \
Vigila la presión de memoria."
                    .into(),
                proposed_action: None,
            });
        }
    }

    if let (Some(procs), Some(total)) = (&d.processes, d.total) {
        if let Some(top) = procs.top.first().filter(|p| total > 0 && p.memory * 4 > total) {
            out.push(Finding {
                id: "mem.process.heavy",
                kind: K::Warning,
                severity: S::Medium,
                title: format!("{} usa {} de memoria", top.name, gib(top.memory)),
                evidence: vec![Evidence::new(
                    format!("{} (PID {})", top.name, top.pid),
                    format!("{} · {} de la RAM", gib(top.memory), pct(top.memory, total)),
                    "sysinfo (memoria residente)",
                )],
                confidence: C::Medium,
                recommendation: "Si no lo estás usando activamente, ciérralo desde la propia aplicación o desde Monitor \
de Actividad. Jarvis no cierra procesos."
                    .into(),
                proposed_action: Some(format!("Cerrar {} (PID {}) manualmente si no lo necesitas", top.name, top.pid)),
            });
        }
        if !procs.top.is_empty() {
            out.push(Finding {
                id: "mem.process.top",
                kind: K::Observation,
                severity: S::Info,
                title: "Procesos con mayor consumo de memoria".into(),
                evidence: procs
                    .top
                    .iter()
                    .take(5)
                    .map(|p| Evidence::new(format!("{} (PID {})", p.name, p.pid), format!("{} · {}", gib(p.memory), pct(p.memory, total)), "sysinfo"))
                    .collect(),
                confidence: C::High,
                recommendation: "Lista informativa; la memoria residente de un proceso puede incluir memoria compartida.".into(),
                proposed_action: None,
            });
        }
    }

    out
}
