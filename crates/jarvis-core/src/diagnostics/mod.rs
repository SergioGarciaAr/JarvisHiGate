//! Read-only system diagnostics (`jarvis diagnostico`).
//!
//! Collection runs real probes step by step (each one reported as a progress
//! event), then pure analysis functions turn the data into findings. Nothing
//! here kills processes, changes settings or writes files.

pub mod finding;
pub mod memory;
pub mod network;
pub mod parse;
pub mod probe;
pub mod sysexec;
pub mod system;

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::agent::{Budget, LimitError, Limits};
use crate::command::DiagnosticoScope;
use crate::events::{EventKind, EventStatus, Reporter};
use crate::telemetry::{TelemetryService, TelemetrySnapshot};
use finding::{DataGap, Finding, FindingKind, sort_findings};
use memory::MemoryData;
use network::{DnsCheck, GatewayCheck, NetworkData, RouteState, TcpCheck, TcpOutcome};
use parse::RouteResult;
use probe::SystemProbe;
use sysexec::{FixedCommand, ProbeError};

pub use finding::{Confidence, Evidence, Severity};

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub scope: DiagnosticoScope,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub system: Option<TelemetrySnapshot>,
    pub memory: Option<MemoryData>,
    pub network: Option<NetworkData>,
    pub findings: Vec<Finding>,
    pub gaps: Vec<DataGap>,
    pub notes: Vec<String>,
}

impl DiagnosticReport {
    pub fn count(&self, kind: FindingKind) -> usize {
        self.findings.iter().filter(|f| f.kind == kind).count()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DiagnosticOutcome {
    Completed { report: Box<DiagnosticReport> },
    Cancelled,
    Failed { message: String },
}

pub fn default_limits() -> Limits {
    Limits { max_steps: 30, step_timeout: Duration::from_secs(10), total_timeout: Duration::from_secs(90), max_retries: 0 }
}

pub fn scope_label(scope: DiagnosticoScope) -> &'static str {
    match scope {
        DiagnosticoScope::General => "general",
        DiagnosticoScope::Full => "completo",
        DiagnosticoScope::Red => "de red",
        DiagnosticoScope::Ram => "de memoria",
    }
}

enum Stop {
    Cancelled,
    Aborted(String),
}

struct Ctx<'a> {
    budget: Budget,
    reporter: &'a Reporter,
    gaps: Vec<DataGap>,
}

impl Ctx<'_> {
    fn phase(&self, name: &str) {
        self.reporter.emit(EventKind::Phase, EventStatus::Running, name, None);
    }

    /// Runs one real probe, reporting it as a tool call and its result.
    async fn step<T>(
        &mut self,
        label: &str,
        fut: impl Future<Output = Result<T, ProbeError>>,
        summary: impl FnOnce(&T) -> String,
    ) -> Result<Result<T, ProbeError>, Stop> {
        use EventKind as K;
        use EventStatus as S;
        self.reporter.emit(K::ToolCall, S::Running, label, Some(json!({ "tool": label })));
        match self.budget.step(fut).await {
            Err(LimitError::Cancelled) => Err(Stop::Cancelled),
            Err(LimitError::StepTimeout) => {
                let ms = self.budget.limits().step_timeout.as_millis() as u64;
                self.reporter.emit(K::ToolResult, S::Failed, format!("{label}: superó el tiempo máximo del paso"), None);
                Ok(Err(ProbeError::Timeout { ms }))
            }
            Err(e) => Err(Stop::Aborted(e.to_string())),
            Ok(Ok(v)) => {
                self.reporter.emit(K::ToolResult, S::Ok, summary(&v), None);
                Ok(Ok(v))
            }
            Ok(Err(e)) => {
                self.reporter.emit(K::ToolResult, S::Failed, format!("{label}: {e}"), None);
                Ok(Err(e))
            }
        }
    }

    fn gap(&mut self, what: &str, source: &str, err: &ProbeError) {
        self.gaps.push(DataGap::new(what, source, err.to_string()));
    }
}

/// Runs a closure against the shared telemetry service off the async runtime.
async fn with_telemetry<T: Send + 'static>(
    telemetry: &Arc<Mutex<TelemetryService>>,
    f: impl FnOnce(&mut TelemetryService) -> T + Send + 'static,
) -> Result<T, ProbeError> {
    let t = telemetry.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = t.lock().map_err(|_| ProbeError::Io { message: "servicio de telemetría bloqueado".into() })?;
        Ok(f(&mut guard))
    })
    .await
    .map_err(|e| ProbeError::Io { message: e.to_string() })?
}

async fn command_stdout(probe: &dyn SystemProbe, cmd: FixedCommand) -> Result<String, ProbeError> {
    probe.command(cmd).await?.into_stdout()
}

/// `relevant` lists the telemetry metrics this scope uses; only their gaps are reported.
async fn collect_snapshot(
    ctx: &mut Ctx<'_>,
    telemetry: &Arc<Mutex<TelemetryService>>,
    relevant: &[&str],
) -> Result<Option<TelemetrySnapshot>, Stop> {
    let r = ctx
        .step("telemetría del sistema (sysinfo)", with_telemetry(telemetry, |t| t.latest()), |s| {
            format!("Lectura de telemetría de {}", s.timestamp.format("%H:%M:%S"))
        })
        .await?;
    match r {
        Ok(snap) => {
            for u in snap.unavailable.iter().filter(|u| relevant.contains(&u.metric)) {
                ctx.gaps.push(DataGap::new(u.metric, "sysinfo", u.reason.clone()));
            }
            Ok(Some(snap))
        }
        Err(e) => {
            ctx.gap("Telemetría del sistema", "sysinfo", &e);
            Ok(None)
        }
    }
}

async fn collect_memory(
    ctx: &mut Ctx<'_>,
    probe: &dyn SystemProbe,
    telemetry: &Arc<Mutex<TelemetryService>>,
    snapshot: Option<&TelemetrySnapshot>,
    with_processes: bool,
) -> Result<MemoryData, Stop> {
    ctx.phase("Memoria");
    let mut d = MemoryData {
        total: snapshot.and_then(|s| s.mem_total),
        used: snapshot.and_then(|s| s.mem_used),
        available: snapshot.and_then(|s| s.mem_available),
        ..Default::default()
    };

    let c = FixedCommand::SysctlPressureLevel;
    match ctx
        .step(&c.display(), async { parse::parse_pressure_level(&command_stdout(probe, c).await?) }, |p| format!("Presión: {p:?}"))
        .await?
    {
        Ok(p) => d.pressure = Some(p),
        Err(e) => ctx.gap("Presión de memoria", &c.display(), &e),
    }

    let c = FixedCommand::SysctlFreeLevel;
    match ctx
        .step(&c.display(), async { parse::parse_free_level(&command_stdout(probe, c).await?) }, |v| format!("Memoria libre según el kernel: {v} %"))
        .await?
    {
        Ok(v) => d.free_level = Some(v),
        Err(e) => ctx.gap("Nivel de memoria libre", &c.display(), &e),
    }

    let c = FixedCommand::SysctlSwapUsage;
    match ctx
        .step(&c.display(), async { parse::parse_swap_usage(&command_stdout(probe, c).await?) }, |s| {
            format!("Swap: {} usados", finding::gib(s.used))
        })
        .await?
    {
        Ok(s) => d.swap = Some(s),
        Err(e) => ctx.gap("Uso de swap", &c.display(), &e),
    }

    let c = FixedCommand::VmStat;
    match ctx
        .step(&c.display(), async { parse::parse_vm_stat(&command_stdout(probe, c).await?) }, |v| {
            format!("Estadísticas de memoria virtual (páginas de {} bytes)", v.page_size)
        })
        .await?
    {
        Ok(v) => d.vm = Some(v),
        Err(e) => ctx.gap("Estadísticas de memoria virtual", &c.display(), &e),
    }

    if with_processes {
        match ctx
            .step("procesos por memoria (sysinfo)", with_telemetry(telemetry, |t| t.processes_by_memory(10)), |p| {
                format!("{} procesos analizados", p.total)
            })
            .await?
        {
            Ok(p) => {
                if p.without_data > 0 {
                    ctx.gaps.push(DataGap::new(
                        format!("Memoria de {} de {} procesos", p.without_data, p.total),
                        "sysinfo",
                        "macOS no permite a una app sin privilegios leer la memoria de procesos de otros usuarios o del sistema",
                    ));
                }
                d.processes = Some(p);
            }
            Err(e) => ctx.gap("Lista de procesos", "sysinfo", &e),
        }
    }
    Ok(d)
}

fn tcp_check(label: &str, addr: SocketAddr, via_dns: bool, r: Result<Duration, ProbeError>) -> TcpCheck {
    let (outcome, ms, detail) = match r {
        Ok(d) => (TcpOutcome::Connected, Some(d.as_millis() as u64), None),
        Err(ProbeError::Refused) => (TcpOutcome::Refused, None, None),
        Err(ProbeError::Timeout { ms }) => (TcpOutcome::Timeout, None, Some(format!("sin respuesta en {ms} ms"))),
        Err(ProbeError::Unreachable { message }) => (TcpOutcome::Unreachable, None, Some(message)),
        Err(e) => (TcpOutcome::Error, None, Some(e.to_string())),
    };
    TcpCheck { label: label.to_string(), target: addr.to_string(), via_dns, outcome, ms, detail }
}

async fn collect_network(ctx: &mut Ctx<'_>, probe: &dyn SystemProbe, telemetry: &Arc<Mutex<TelemetryService>>) -> Result<NetworkData, Stop> {
    ctx.phase("Red");
    let mut d = NetworkData::default();

    match ctx
        .step("interfaces de red (sysinfo)", with_telemetry(telemetry, |t| t.interfaces()), |i| {
            format!("{} interfaces encontradas", i.len())
        })
        .await?
    {
        Ok(i) => d.interfaces = i,
        Err(e) => ctx.gap("Interfaces de red", "sysinfo", &e),
    }

    let c = FixedCommand::NetworkServiceOrder;
    match ctx
        .step(&c.display(), async { Ok(parse::parse_service_order(&command_stdout(probe, c).await?)) }, |s| {
            format!("{} servicios de red", s.len())
        })
        .await?
    {
        Ok(s) => d.services = s,
        Err(e) => ctx.gap("Nombres de servicios de red", &c.display(), &e),
    }

    let c = FixedCommand::RouteDefault;
    match ctx
        .step(
            &c.display(),
            async {
                let out = probe.command(c).await?;
                parse::parse_route(&out.stdout, &out.stderr, out.success())
            },
            |r| match r {
                RouteResult::Route(r) => format!("Puerta de enlace {}", r.gateway),
                RouteResult::NoDefaultRoute => "No hay ruta por defecto".into(),
            },
        )
        .await?
    {
        Ok(RouteResult::Route(r)) => d.route = RouteState::Default(r),
        Ok(RouteResult::NoDefaultRoute) => d.route = RouteState::NoDefault,
        Err(e) => ctx.gap("Ruta por defecto", &c.display(), &e),
    }

    let c = FixedCommand::ScutilDns;
    match ctx
        .step(&c.display(), async { Ok(parse::parse_dns_servers(&command_stdout(probe, c).await?)) }, |s| {
            if s.is_empty() { "Sin servidores DNS".into() } else { format!("Servidores DNS: {}", s.join(", ")) }
        })
        .await?
    {
        Ok(s) => d.dns_servers = Some(s),
        Err(e) => ctx.gap("Servidores DNS", &c.display(), &e),
    }

    if let RouteState::Default(route) = &d.route {
        match route.gateway.parse::<Ipv4Addr>() {
            Ok(ip) if !sysexec::is_pingable_gateway(ip) => ctx.gaps.push(DataGap::new(
                "Ping a la puerta de enlace",
                "ping",
                format!("la puerta de enlace «{ip}» no es una dirección unicast; no se envía ping"),
            )),
            Ok(ip) => {
                let c = FixedCommand::PingGateway(ip);
                match ctx
                    .step(
                        &c.display(),
                        async {
                            let out = probe.command(c).await?;
                            parse::parse_ping(&out.stdout, &out.stderr)
                        },
                        |p| format!("Router: {}/{} respuestas", p.received, p.transmitted),
                    )
                    .await?
                {
                    Ok(stats) => d.gateway = Some(GatewayCheck { gateway: route.gateway.clone(), stats }),
                    Err(e) => ctx.gap("Ping a la puerta de enlace", &c.display(), &e),
                }
            }
            Err(_) => ctx.gaps.push(DataGap::new(
                "Ping a la puerta de enlace",
                "ping",
                format!("la puerta de enlace «{}» no es una dirección IPv4", route.gateway),
            )),
        }
    }

    for host in network::DNS_HOSTS {
        let started = Instant::now();
        let r = ctx
            .step(&format!("resolver {host}"), async { probe.resolve(host).await.map(|ips| (ips, started.elapsed())) }, |(ips, t)| {
                format!("{host} → {} en {} ms", ips.first().map(IpAddr::to_string).unwrap_or_default(), t.as_millis())
            })
            .await?;
        d.dns.push(match r {
            Ok((ips, t)) => DnsCheck {
                host: host.into(),
                addresses: ips.iter().map(IpAddr::to_string).collect(),
                ms: Some(t.as_millis() as u64),
                error: if ips.is_empty() { Some("sin direcciones".into()) } else { None },
            },
            Err(e) => DnsCheck { host: host.into(), addresses: vec![], ms: None, error: Some(e.to_string()) },
        });
    }

    for (name, ip, port) in network::TCP_TARGETS {
        let addr = SocketAddr::from((ip, port));
        let label = format!("{name} {addr}");
        let r = ctx.step(&format!("conexión TCP a {addr}"), probe.tcp_connect(addr), |t| format!("{label}: conectado en {} ms", t.as_millis())).await?;
        d.tcp.push(tcp_check(name, addr, false, r));
    }

    if let Some(ip) = d.dns.iter().find(|c| c.ok()).and_then(|c| c.addresses.first()?.parse::<IpAddr>().ok().map(|ip| (c.host.clone(), ip))) {
        let (host, ip) = ip;
        let addr = SocketAddr::new(ip, 443);
        let r = ctx.step(&format!("conexión TCP a {host}:443"), probe.tcp_connect(addr), |t| format!("{host}: conectado en {} ms", t.as_millis())).await?;
        d.tcp.push(tcp_check(&host, addr, true, r));
    }
    Ok(d)
}

pub async fn run_diagnostic(
    scope: DiagnosticoScope,
    probe: &dyn SystemProbe,
    telemetry: Arc<Mutex<TelemetryService>>,
    reporter: &Reporter,
    limits: Limits,
    cancel: CancellationToken,
) -> DiagnosticOutcome {
    use EventKind as K;
    use EventStatus as S;
    let started = Instant::now();
    let mut ctx = Ctx { budget: Budget::new(limits, cancel), reporter, gaps: Vec::new() };
    reporter.emit(K::Goal, S::Ok, format!("Diagnóstico {} (solo lectura)", scope_label(scope)), Some(json!({ "scope": scope })));

    let collected = async {
        let mut system = None;
        let mut memory = None;
        let mut network = None;
        let needs_snapshot = !matches!(scope, DiagnosticoScope::Red);
        if needs_snapshot {
            ctx.phase(if scope == DiagnosticoScope::Ram { "Telemetría del sistema" } else { "Sistema y almacenamiento" });
            let relevant: &[&str] = if scope == DiagnosticoScope::Ram { &["memoria"] } else { &["cpu", "memoria", "disco"] };
            system = collect_snapshot(&mut ctx, &telemetry, relevant).await?;
        }
        if scope != DiagnosticoScope::Red {
            let with_processes = scope != DiagnosticoScope::General;
            memory = Some(collect_memory(&mut ctx, probe, &telemetry, system.as_ref(), with_processes).await?);
        }
        if matches!(scope, DiagnosticoScope::Red | DiagnosticoScope::Full) {
            network = Some(collect_network(&mut ctx, probe, &telemetry).await?);
        }
        Ok::<_, Stop>((system, memory, network))
    }
    .await;

    let (system, memory, network) = match collected {
        Ok(v) => v,
        Err(Stop::Cancelled) => {
            reporter.emit(K::Cancelled, S::Cancelled, "Diagnóstico cancelado por el usuario", None);
            return DiagnosticOutcome::Cancelled;
        }
        Err(Stop::Aborted(message)) => {
            reporter.emit(K::Error, S::Failed, message.clone(), None);
            reporter.emit(K::Completed, S::Failed, "Finalizado con error", None);
            return DiagnosticOutcome::Failed { message };
        }
    };

    reporter.emit(K::Phase, S::Running, "Análisis de resultados", None);
    let mut findings = Vec::new();
    let mut notes = Vec::new();
    if let Some(s) = &system
        && scope != DiagnosticoScope::Ram {
            findings.extend(system::analyze_system(s));
            findings.extend(system::analyze_storage(s));
        }
    if let Some(m) = &memory {
        findings.extend(memory::analyze_memory(m));
        notes.push(memory::NOTE.to_string());
    }
    if let Some(n) = &network {
        findings.extend(network::analyze_network(n));
        notes.push(network::NOTE.to_string());
    }
    sort_findings(&mut findings);
    notes.push("Diagnóstico de solo lectura: Jarvis no ha cerrado procesos, cambiado ajustes ni modificado archivos.".into());

    let report = DiagnosticReport {
        scope,
        generated_at: chrono::Utc::now(),
        duration_ms: started.elapsed().as_millis() as u64,
        system,
        memory,
        network,
        findings,
        gaps: ctx.gaps,
        notes,
    };
    let summary = format!(
        "{} problemas · {} advertencias · {} observaciones · {} datos no disponibles",
        report.count(FindingKind::Problem),
        report.count(FindingKind::Warning),
        report.count(FindingKind::Observation),
        report.gaps.len()
    );
    reporter.emit(K::Result, S::Ok, summary, Some(serde_json::to_value(&report).expect("serializable report")));
    if let Some(top) = report.findings.iter().find(|f| f.kind != FindingKind::Observation) {
        reporter.emit(K::NextStep, S::Pending, top.recommendation.clone(), None);
    }
    reporter.emit(K::Completed, S::Ok, "Diagnóstico completado", None);
    DiagnosticOutcome::Completed { report: Box::new(report) }
}
