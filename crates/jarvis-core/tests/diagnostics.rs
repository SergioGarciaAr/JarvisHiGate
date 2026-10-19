//! Diagnostics tests.
//!
//! - Analysis tests feed hand-built data to the pure analysis functions.
//! - Runner tests use `FakeProbe` to inject controlled failures (permission
//!   errors, timeouts, network failures). They exercise the real runner and
//!   analysis code, but the system responses are simulated.
//! - `real_*` tests run against this Mac (read-only, no network).
//! - `live_network_*` tests need Internet and are `#[ignore]`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use jarvis_core::command::DiagnosticoScope;
use jarvis_core::diagnostics::finding::{Finding, FindingKind};
use jarvis_core::diagnostics::memory::{MemoryData, analyze_memory};
use jarvis_core::diagnostics::network::{DnsCheck, GatewayCheck, NetworkData, RouteState, TcpCheck, TcpOutcome, analyze_network};
use jarvis_core::diagnostics::parse::{DefaultRoute, PingStats, PressureLevel, SwapUsage};
use jarvis_core::diagnostics::probe::{MacProbe, SystemProbe};
use jarvis_core::diagnostics::sysexec::{CommandOutput, FixedCommand, ProbeError};
use jarvis_core::diagnostics::system::analyze_storage;
use jarvis_core::diagnostics::{DiagnosticOutcome, DiagnosticReport, default_limits, run_diagnostic};
use jarvis_core::events::{EventKind, EventStatus, MemorySink, Reporter};
use jarvis_core::telemetry::{DiskInfo, InterfaceInfo, ProcessMem, ProcessSummary, TelemetryService, TelemetrySnapshot};
use tokio_util::sync::CancellationToken;

const GIB: u64 = 1024 * 1024 * 1024;

fn ids(findings: &[Finding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.id).collect()
}

fn find<'a>(findings: &'a [Finding], id: &str) -> &'a Finding {
    findings.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("falta {id} en {:?}", ids(findings)))
}

// ---------- memory analysis ----------

fn mem(pressure: PressureLevel, swap_used: u64) -> MemoryData {
    MemoryData {
        total: Some(16 * GIB),
        used: Some(10 * GIB),
        available: Some(6 * GIB),
        pressure: Some(pressure),
        free_level: Some(60),
        vm: None,
        swap: Some(SwapUsage { total: 2 * GIB, used: swap_used, free: 2 * GIB - swap_used, encrypted: true }),
        processes: None,
    }
}

#[test]
fn memory_normal() {
    let f = analyze_memory(&mem(PressureLevel::Normal, 0));
    assert_eq!(find(&f, "mem.pressure.normal").kind, FindingKind::Observation);
    assert_eq!(find(&f, "mem.swap.none").kind, FindingKind::Observation);
    assert!(f.iter().all(|f| f.kind == FindingKind::Observation));
    assert!(f.iter().all(|f| !f.evidence.is_empty() && !f.recommendation.is_empty()));
}

#[test]
fn memory_critical_pressure_is_confirmed_problem() {
    let f = analyze_memory(&mem(PressureLevel::Critical, GIB));
    let p = find(&f, "mem.pressure.critical");
    assert_eq!(p.kind, FindingKind::Problem);
    assert_eq!(find(&f, "mem.swap.pressure").kind, FindingKind::Warning);
}

#[test]
fn memory_residual_swap_is_only_observation() {
    let f = analyze_memory(&mem(PressureLevel::Normal, GIB));
    assert_eq!(find(&f, "mem.swap.residual").kind, FindingKind::Observation);
}

#[test]
fn memory_heavy_process_warns_but_never_acts() {
    let mut d = mem(PressureLevel::Normal, 0);
    d.processes = Some(ProcessSummary {
        top: vec![
            ProcessMem { pid: 42, name: "Chrome".into(), memory: 6 * GIB },
            ProcessMem { pid: 7, name: "Finder".into(), memory: GIB / 4 },
        ],
        total: 300,
        without_data: 120,
    });
    let f = analyze_memory(&d);
    let heavy = find(&f, "mem.process.heavy");
    assert_eq!(heavy.kind, FindingKind::Warning);
    assert!(heavy.proposed_action.as_deref().unwrap().contains("manualmente"));
    assert!(heavy.recommendation.contains("Jarvis no cierra procesos"));
    assert_eq!(find(&f, "mem.process.top").evidence.len(), 2);
}

#[test]
fn memory_missing_data_produces_no_invented_findings() {
    assert!(analyze_memory(&MemoryData::default()).is_empty());
    let only_total = MemoryData { total: Some(16 * GIB), ..Default::default() };
    assert!(analyze_memory(&only_total).is_empty());
}

// ---------- storage analysis ----------

fn snapshot(free: u64, total: u64) -> TelemetrySnapshot {
    TelemetrySnapshot {
        timestamp: chrono::Utc::now(),
        hostname: None,
        os_version: None,
        uptime_secs: 3600,
        cpu_percent: Some(12.0),
        cpu_cores: 8,
        load_avg: Some([1.0, 1.0, 1.0]),
        mem_total: Some(16 * GIB),
        mem_used: Some(8 * GIB),
        mem_available: Some(8 * GIB),
        swap_total: Some(0),
        swap_used: Some(0),
        disk: Some(DiskInfo { mount: "/System/Volumes/Data".into(), total_bytes: total, available_bytes: free }),
        net_rx_per_sec: None,
        net_tx_per_sec: None,
        unavailable: vec![],
    }
}

#[test]
fn storage_thresholds() {
    assert_eq!(analyze_storage(&snapshot(3 * GIB, 500 * GIB))[0].id, "disk.critical");
    assert_eq!(analyze_storage(&snapshot(30 * GIB, 500 * GIB))[0].id, "disk.low");
    assert_eq!(analyze_storage(&snapshot(200 * GIB, 500 * GIB))[0].id, "disk.ok");
    let mut s = snapshot(0, 0);
    s.disk = None;
    assert!(analyze_storage(&s).is_empty());
}

// ---------- network analysis ----------

fn iface(name: &str, ip: &str) -> InterfaceInfo {
    InterfaceInfo {
        name: name.into(),
        mac: "00:00:00:00:00:00".into(),
        mtu: 1500,
        state: "up",
        addresses: vec![ip.into()],
        has_routable_ip: !ip.starts_with("fe80") && !ip.starts_with("127"),
        total_rx_bytes: 0,
        total_tx_bytes: 0,
        total_rx_packets: 10_000,
        total_tx_packets: 10_000,
        rx_errors: 0,
        tx_errors: 0,
    }
}

fn tcp(label: &str, outcome: TcpOutcome, ms: Option<u64>, via_dns: bool) -> TcpCheck {
    TcpCheck { label: label.into(), target: "1.1.1.1:443".into(), via_dns, outcome, ms, detail: None }
}

fn dns(host: &str, ok: bool) -> DnsCheck {
    DnsCheck {
        host: host.into(),
        addresses: if ok { vec!["17.253.144.10".into()] } else { vec![] },
        ms: ok.then_some(20),
        error: (!ok).then(|| "timeout".into()),
    }
}

fn healthy_net() -> NetworkData {
    NetworkData {
        interfaces: vec![iface("lo0", "127.0.0.1/8"), iface("en0", "192.168.1.54/24")],
        services: vec![("en0".into(), "Wi-Fi".into())],
        route: RouteState::Default(DefaultRoute { gateway: "192.168.1.1".into(), interface: Some("en0".into()) }),
        dns_servers: Some(vec!["200.48.225.130".into()]),
        gateway: Some(GatewayCheck {
            gateway: "192.168.1.1".into(),
            stats: PingStats { transmitted: 2, received: 2, avg_ms: Some(3.0), send_error: None },
        }),
        dns: vec![dns("apple.com", true), dns("cloudflare.com", true)],
        tcp: vec![
            tcp("Cloudflare", TcpOutcome::Connected, Some(40), false),
            tcp("Google", TcpOutcome::Connected, Some(45), false),
            tcp("apple.com", TcpOutcome::Connected, Some(60), true),
        ],
    }
}

#[test]
fn network_normal() {
    let f = analyze_network(&healthy_net());
    assert!(f.iter().all(|f| f.kind == FindingKind::Observation), "{:?}", ids(&f));
    assert!(find(&f, "net.route.ok").title.contains("Wi-Fi"));
    find(&f, "net.external.ok");
    find(&f, "net.gateway.ok");
}

#[test]
fn network_no_default_route() {
    let mut d = healthy_net();
    d.route = RouteState::NoDefault;
    assert_eq!(find(&analyze_network(&d), "net.route.none").kind, FindingKind::Problem);
}

#[test]
fn network_no_usable_interface() {
    let mut d = healthy_net();
    d.interfaces = vec![iface("lo0", "127.0.0.1/8"), iface("en0", "fe80::1/64")];
    assert_eq!(find(&analyze_network(&d), "net.iface.none").kind, FindingKind::Problem);
}

#[test]
fn network_everything_down_includes_caveat() {
    let mut d = healthy_net();
    d.gateway = None;
    d.dns = vec![dns("apple.com", false), dns("cloudflare.com", false)];
    d.tcp = vec![tcp("Cloudflare", TcpOutcome::Timeout, None, false), tcp("Google", TcpOutcome::Unreachable, None, false)];
    let f = analyze_network(&d);
    let down = find(&f, "net.external.down");
    assert_eq!(down.kind, FindingKind::Problem);
    assert!(down.recommendation.contains("no demuestra por sí solo"));
}

#[test]
fn network_partial_failure_is_low_confidence_warning() {
    let mut d = healthy_net();
    d.tcp[1] = tcp("Google", TcpOutcome::Timeout, None, false);
    let f = analyze_network(&d);
    let partial = find(&f, "net.external.partial");
    assert_eq!(partial.kind, FindingKind::Warning);
    assert!(partial.recommendation.contains("no demuestra por sí solo"));
}

#[test]
fn network_refused_counts_as_reachable() {
    let mut d = healthy_net();
    d.tcp[0] = tcp("Cloudflare", TcpOutcome::Refused, None, false);
    find(&analyze_network(&d), "net.external.ok");
}

#[test]
fn network_dns_failure_with_ip_connectivity() {
    let mut d = healthy_net();
    d.dns = vec![dns("apple.com", false), dns("cloudflare.com", false)];
    d.tcp.truncate(2);
    let f = analyze_network(&d);
    assert_eq!(find(&f, "net.dns.failing").kind, FindingKind::Problem);
    assert!(!ids(&f).contains(&"net.external.down"));
}

#[test]
fn network_local_network_permission_is_not_reported_as_outage() {
    let mut d = healthy_net();
    d.gateway = Some(GatewayCheck {
        gateway: "192.168.1.1".into(),
        stats: PingStats { transmitted: 2, received: 0, avg_ms: None, send_error: Some("No route to host".into()) },
    });
    let f = analyze_network(&d);
    let g = find(&f, "net.gateway.local_network_permission");
    assert_eq!(g.kind, FindingKind::Observation);
    assert!(g.recommendation.contains("Red local"));
}

#[test]
fn network_empty_dns_and_slow_latency() {
    let mut d = healthy_net();
    d.dns_servers = Some(vec![]);
    d.tcp[0].ms = Some(800);
    let f = analyze_network(&d);
    assert_eq!(find(&f, "net.dns.none").kind, FindingKind::Warning);
    assert_eq!(find(&f, "net.latency.high").kind, FindingKind::Warning);
}

// ---------- runner with controlled failures ----------

type CmdFn = dyn Fn(FixedCommand) -> Result<CommandOutput, ProbeError> + Send + Sync;
type ResolveFn = dyn Fn(&str) -> Result<Vec<IpAddr>, ProbeError> + Send + Sync;
type TcpFn = dyn Fn(SocketAddr) -> Result<Duration, ProbeError> + Send + Sync;

/// Simulated system responses (see module docs).
struct FakeProbe {
    command: Box<CmdFn>,
    resolve: Box<ResolveFn>,
    tcp: Box<TcpFn>,
    delay: Duration,
}

impl SystemProbe for FakeProbe {
    fn command(&self, cmd: FixedCommand) -> BoxFuture<'_, Result<CommandOutput, ProbeError>> {
        Box::pin(async move {
            tokio::time::sleep(self.delay).await;
            (self.command)(cmd)
        })
    }
    fn resolve(&self, host: &'static str) -> BoxFuture<'_, Result<Vec<IpAddr>, ProbeError>> {
        Box::pin(async move { (self.resolve)(host) })
    }
    fn tcp_connect(&self, addr: SocketAddr) -> BoxFuture<'_, Result<Duration, ProbeError>> {
        Box::pin(async move { (self.tcp)(addr) })
    }
    fn net_timeout(&self) -> Duration {
        Duration::from_secs(3)
    }
}

fn out(stdout: &str) -> Result<CommandOutput, ProbeError> {
    Ok(CommandOutput { code: Some(0), stdout: stdout.into(), stderr: String::new() })
}

fn real_outputs(cmd: FixedCommand) -> Result<CommandOutput, ProbeError> {
    match cmd {
        FixedCommand::SysctlPressureLevel => out("1\n"),
        FixedCommand::SysctlFreeLevel => out("78\n"),
        FixedCommand::SysctlSwapUsage => out("total = 0.00M  used = 0.00M  free = 0.00M  (encrypted)\n"),
        FixedCommand::VmStat => out(include_str!("fixtures/vm_stat.txt")),
        FixedCommand::RouteDefault => out("    gateway: 192.168.1.1\n  interface: en0\n"),
        FixedCommand::ScutilDns => out("resolver #1\n  nameserver[0] : 200.48.225.130\n"),
        FixedCommand::NetworkServiceOrder => out("(1) Wi-Fi\n(Hardware Port: Wi-Fi, Device: en0)\n"),
        FixedCommand::PingGateway(_) => Ok(CommandOutput {
            code: Some(0),
            stdout: "2 packets transmitted, 2 packets received, 0.0% packet loss\nround-trip min/avg/max/stddev = 1/2/3/0.5 ms\n".into(),
            stderr: String::new(),
        }),
        FixedCommand::SayListVoices => panic!("diagnostics never list voices"),
    }
}

fn healthy_probe() -> FakeProbe {
    FakeProbe {
        command: Box::new(real_outputs),
        resolve: Box::new(|_| Ok(vec![IpAddr::V4(Ipv4Addr::new(17, 253, 144, 10))])),
        tcp: Box::new(|_| Ok(Duration::from_millis(30))),
        delay: Duration::ZERO,
    }
}

fn telemetry() -> Arc<Mutex<TelemetryService>> {
    Arc::new(Mutex::new(TelemetryService::new()))
}

async fn run(scope: DiagnosticoScope, probe: &FakeProbe) -> (DiagnosticOutcome, Arc<MemorySink>) {
    let sink = Arc::new(MemorySink::default());
    let reporter = Reporter::new("diag", sink.clone());
    let out = run_diagnostic(scope, probe, telemetry(), &reporter, default_limits(), CancellationToken::new()).await;
    (out, sink)
}

fn report(o: DiagnosticOutcome) -> DiagnosticReport {
    match o {
        DiagnosticOutcome::Completed { report } => *report,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn runner_events_match_steps_actually_run() {
    let (o, sink) = run(DiagnosticoScope::Red, &healthy_probe()).await;
    let r = report(o);
    let events = sink.0.lock().unwrap();
    let calls = events.iter().filter(|e| e.kind == EventKind::ToolCall).count();
    let results = events.iter().filter(|e| e.kind == EventKind::ToolResult).count();
    // interfaces, services, route, dns servers, ping, 2 resolves, 2 tcp, 1 tcp via dns
    assert_eq!(calls, 10);
    assert_eq!(calls, results, "every tool call has exactly one result");
    assert_eq!(events.last().unwrap().kind, EventKind::Completed);
    assert!(r.memory.is_none() && r.system.is_none());
    assert!(r.notes.iter().any(|n| n.contains("no demuestra por sí solo")));
}

#[tokio::test]
async fn runner_permission_errors_become_gaps_not_values() {
    let probe = FakeProbe {
        command: Box::new(|cmd| Err(ProbeError::PermissionDenied { path: cmd.program().into() })),
        ..healthy_probe()
    };
    let r = report(run(DiagnosticoScope::Ram, &probe).await.0);
    let m = r.memory.as_ref().unwrap();
    assert!(m.pressure.is_none() && m.swap.is_none() && m.vm.is_none() && m.free_level.is_none());
    for what in ["Presión de memoria", "Uso de swap", "Estadísticas de memoria virtual", "Nivel de memoria libre"] {
        let gap = r.gaps.iter().find(|g| g.what == what).unwrap_or_else(|| panic!("falta gap {what}: {:?}", r.gaps));
        assert!(gap.reason.contains("permiso denegado"), "{gap:?}");
    }
    assert!(!r.findings.iter().any(|f| f.id.starts_with("mem.pressure") || f.id.starts_with("mem.swap")));
}

#[tokio::test]
async fn runner_unparseable_output_is_a_gap() {
    let probe = FakeProbe { command: Box::new(|_| out("salida inesperada")), ..healthy_probe() };
    let r = report(run(DiagnosticoScope::Ram, &probe).await.0);
    assert!(r.gaps.iter().any(|g| g.what == "Presión de memoria" && g.reason.contains("no reconocida")));
}

#[tokio::test]
async fn runner_network_failure() {
    let probe = FakeProbe {
        command: Box::new(|cmd| match cmd {
            FixedCommand::PingGateway(_) => Ok(CommandOutput {
                code: Some(2),
                stdout: "2 packets transmitted, 0 packets received, 100.0% packet loss\n".into(),
                stderr: String::new(),
            }),
            other => real_outputs(other),
        }),
        resolve: Box::new(|_| Err(ProbeError::Timeout { ms: 3000 })),
        tcp: Box::new(|_| Err(ProbeError::Unreachable { message: "Network is unreachable".into() })),
        delay: Duration::ZERO,
    };
    let r = report(run(DiagnosticoScope::Red, &probe).await.0);
    let n = r.network.as_ref().unwrap();
    assert_eq!(n.tcp.len(), 2, "no DNS-based TCP check when DNS fails");
    assert!(n.tcp.iter().all(|c| c.outcome == TcpOutcome::Unreachable));
    let down = r.findings.iter().find(|f| f.id == "net.external.down").expect("external down");
    assert_eq!(down.kind, FindingKind::Problem);
    assert_eq!(r.findings[0].kind, FindingKind::Problem, "problems sorted first");
}

#[tokio::test]
async fn runner_no_default_route_skips_ping() {
    let probe = FakeProbe {
        command: Box::new(|cmd| match cmd {
            FixedCommand::RouteDefault => Ok(CommandOutput {
                code: Some(1),
                stdout: String::new(),
                stderr: "route: writing to routing socket: not in table\n".into(),
            }),
            FixedCommand::PingGateway(_) => panic!("ping must not run without a gateway"),
            other => real_outputs(other),
        }),
        ..healthy_probe()
    };
    let r = report(run(DiagnosticoScope::Red, &probe).await.0);
    assert_eq!(r.network.as_ref().unwrap().route, RouteState::NoDefault);
    assert!(r.findings.iter().any(|f| f.id == "net.route.none"));
}

#[tokio::test]
async fn runner_can_be_cancelled_mid_run() {
    let probe = FakeProbe { delay: Duration::from_millis(200), ..healthy_probe() };
    let sink = Arc::new(MemorySink::default());
    let reporter = Reporter::new("diag", sink.clone());
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c2.cancel();
    });
    let o = run_diagnostic(DiagnosticoScope::Full, &probe, telemetry(), &reporter, default_limits(), cancel).await;
    assert!(matches!(o, DiagnosticOutcome::Cancelled));
    let events = sink.0.lock().unwrap();
    assert_eq!(events.last().unwrap().kind, EventKind::Cancelled);
    assert!(!events.iter().any(|e| e.kind == EventKind::Result));
}

#[tokio::test]
async fn runner_slow_step_hits_step_timeout_and_continues() {
    let probe = FakeProbe { delay: Duration::from_millis(300), ..healthy_probe() };
    let sink = Arc::new(MemorySink::default());
    let reporter = Reporter::new("diag", sink.clone());
    let limits = jarvis_core::agent::Limits { step_timeout: Duration::from_millis(100), ..default_limits() };
    let r = report(run_diagnostic(DiagnosticoScope::Ram, &probe, telemetry(), &reporter, limits, CancellationToken::new()).await);
    assert!(r.gaps.iter().any(|g| g.what == "Presión de memoria" && g.reason.contains("sin respuesta")));
    let failed = sink.0.lock().unwrap().iter().filter(|e| e.kind == EventKind::ToolResult && e.status == EventStatus::Failed).count();
    assert!(failed >= 4);
}

// ---------- real Mac, read-only, no network ----------

#[tokio::test]
async fn real_memory_diagnostic_on_this_mac() {
    let sink = Arc::new(MemorySink::default());
    let reporter = Reporter::new("real", sink.clone());
    let probe = MacProbe::default();
    let r = report(run_diagnostic(DiagnosticoScope::Ram, &probe, telemetry(), &reporter, default_limits(), CancellationToken::new()).await);
    let m = r.memory.as_ref().unwrap();
    assert!(m.pressure.is_some(), "gaps: {:?}", r.gaps);
    assert!(m.swap.is_some(), "gaps: {:?}", r.gaps);
    assert!(m.vm.as_ref().is_some_and(|v| v.page_size > 0), "gaps: {:?}", r.gaps);
    assert!(m.processes.as_ref().is_some_and(|p| p.total > 0 && !p.top.is_empty()));
    assert!(r.findings.iter().any(|f| f.id.starts_with("mem.pressure")));
    println!("{}", serde_json::to_string_pretty(&r.findings).unwrap());
    println!("gaps: {:?}", r.gaps);
}

#[tokio::test]
async fn real_tcp_probe_against_local_sockets() {
    let probe = MacProbe { net_timeout: Duration::from_secs(1), ..Default::default() };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let open = listener.local_addr().unwrap();
    assert!(probe.tcp_connect(open).await.is_ok());
    drop(listener);
    assert_eq!(probe.tcp_connect(open).await.unwrap_err(), ProbeError::Refused);
    assert!(!probe.resolve("localhost").await.unwrap().is_empty());
}

#[tokio::test]
#[ignore]
async fn live_network_diagnostic() {
    let sink = Arc::new(MemorySink::default());
    let reporter = Reporter::new("live", sink.clone());
    let r = report(
        run_diagnostic(DiagnosticoScope::Full, &MacProbe::default(), telemetry(), &reporter, default_limits(), CancellationToken::new()).await,
    );
    println!("{}", serde_json::to_string_pretty(&r.findings).unwrap());
    println!("gaps: {:?}", r.gaps);
    println!("{} ms", r.duration_ms);
}

// ---------- gateway validation ----------

fn probe_with_gateway(gateway: &'static str) -> FakeProbe {
    FakeProbe {
        command: Box::new(move |cmd| match cmd {
            FixedCommand::RouteDefault => out(&format!("    gateway: {gateway}\n  interface: en0\n")),
            FixedCommand::PingGateway(ip) => panic!("ping must not be sent to {ip}"),
            other => real_outputs(other),
        }),
        ..healthy_probe()
    }
}

#[tokio::test]
async fn runner_never_pings_non_unicast_gateways() {
    for gw in ["255.255.255.255", "0.0.0.0", "224.0.0.1", "127.0.0.1"] {
        let r = report(run(DiagnosticoScope::Red, &probe_with_gateway(gw)).await.0);
        assert!(r.network.as_ref().unwrap().gateway.is_none(), "{gw}");
        assert!(r.gaps.iter().any(|g| g.what == "Ping a la puerta de enlace" && g.reason.contains("unicast")), "{gw}: {:?}", r.gaps);
    }
}

#[tokio::test]
async fn runner_non_ipv4_gateway_is_a_gap() {
    let r = report(run(DiagnosticoScope::Red, &probe_with_gateway("link#14")).await.0);
    assert!(r.gaps.iter().any(|g| g.what == "Ping a la puerta de enlace" && g.reason.contains("IPv4")));
}

// ---------- end to end: text input -> router -> runner -> events -> report ----------

/// Expected probes per scope, in execution order (each is one ToolCall).
fn expected_steps(scope: DiagnosticoScope) -> Vec<&'static str> {
    let system = vec!["telemetría del sistema (sysinfo)"];
    let memory = vec![
        "sysctl -n kern.memorystatus_vm_pressure_level",
        "sysctl -n kern.memorystatus_level",
        "sysctl -n vm.swapusage",
        "vm_stat",
    ];
    let network = vec![
        "interfaces de red (sysinfo)",
        "networksetup -listnetworkserviceorder",
        "route -n get default",
        "scutil --dns",
        "ping -n -c 2 -t 3 192.168.1.1",
        "resolver apple.com",
        "resolver cloudflare.com",
        "conexión TCP a 1.1.1.1:443",
        "conexión TCP a 8.8.8.8:443",
        "conexión TCP a apple.com:443",
    ];
    match scope {
        DiagnosticoScope::General => [system, memory].concat(),
        DiagnosticoScope::Ram => [system, memory, vec!["procesos por memoria (sysinfo)"]].concat(),
        DiagnosticoScope::Red => network,
        DiagnosticoScope::Full => [system, memory, vec!["procesos por memoria (sysinfo)"], network].concat(),
    }
}

#[tokio::test]
async fn end_to_end_all_documented_commands() {
    use jarvis_core::command::{Availability, Command, availability};
    use jarvis_core::router::{Route, route_input};

    for (input, scope) in [
        ("jarvis diagnostico", DiagnosticoScope::General),
        ("jarvis diagnostico --ram", DiagnosticoScope::Ram),
        ("jarvis diagnostico --red", DiagnosticoScope::Red),
        ("jarvis diagnostico --full", DiagnosticoScope::Full),
    ] {
        let Route::Command { command } = route_input(input) else { panic!("{input} no es un comando") };
        assert_eq!(command, Command::Diagnostico { scope }, "{input}");
        assert_eq!(availability(&command), Availability::Ready);

        // No Ollama client exists anywhere in this path: diagnostics cannot depend on it.
        let (o, sink) = run(scope, &healthy_probe()).await;
        let r = report(o);
        let events = sink.0.lock().unwrap();

        // Progress mirrors the probes that actually ran, in order, each with one result.
        let calls: Vec<&str> = events.iter().filter(|e| e.kind == EventKind::ToolCall).map(|e| e.summary.as_str()).collect();
        assert_eq!(calls, expected_steps(scope), "{input}");
        let results = events.iter().filter(|e| e.kind == EventKind::ToolResult).count();
        assert_eq!(results, calls.len(), "{input}");
        let seqs: Vec<u32> = events.iter().map(|e| e.seq).collect();
        assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{input}: seq contiguo");

        // The Result event carries exactly the report the UI renders.
        let result = events.iter().find(|e| e.kind == EventKind::Result).expect("result event");
        assert_eq!(result.data.as_ref().unwrap(), &serde_json::to_value(&r).unwrap());
        assert_eq!(events.last().unwrap().kind, EventKind::Completed);
        assert_eq!(events.last().unwrap().status, EventStatus::Ok);

        // Sections present per scope.
        let has_sys_findings = r.findings.iter().any(|f| f.id.starts_with("sys.") || f.id.starts_with("disk."));
        let has_processes = r.memory.as_ref().is_some_and(|m| m.processes.is_some());
        match scope {
            DiagnosticoScope::General => {
                assert!(r.system.is_some() && r.memory.is_some() && r.network.is_none() && has_sys_findings && !has_processes)
            }
            DiagnosticoScope::Ram => {
                assert!(r.memory.is_some() && r.network.is_none() && !has_sys_findings && has_processes)
            }
            DiagnosticoScope::Red => assert!(r.system.is_none() && r.memory.is_none() && r.network.is_some()),
            DiagnosticoScope::Full => {
                assert!(r.system.is_some() && r.network.is_some() && has_sys_findings && has_processes)
            }
        }
    }
}

#[tokio::test]
async fn partial_results_keep_successful_data_and_explain_the_rest() {
    let probe = FakeProbe {
        command: Box::new(|cmd| match cmd {
            FixedCommand::SysctlSwapUsage => Err(ProbeError::PermissionDenied { path: cmd.program().into() }),
            FixedCommand::VmStat => Err(ProbeError::Timeout { ms: 5000 }),
            other => real_outputs(other),
        }),
        tcp: Box::new(|addr| if addr.port() == 443 && addr.ip() == IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)) {
            Err(ProbeError::Timeout { ms: 3000 })
        } else {
            Ok(Duration::from_millis(30))
        }),
        ..healthy_probe()
    };
    let (o, sink) = run(DiagnosticoScope::Full, &probe).await;
    let r = report(o);
    let m = r.memory.as_ref().unwrap();
    assert!(m.pressure.is_some(), "datos que sí se obtuvieron se conservan");
    assert!(m.swap.is_none() && m.vm.is_none(), "los que fallaron quedan vacíos, no a cero");
    assert!(r.gaps.iter().any(|g| g.what == "Uso de swap" && g.reason.contains("permiso denegado")));
    assert!(r.gaps.iter().any(|g| g.what == "Estadísticas de memoria virtual" && g.reason.contains("sin respuesta")));
    assert!(!r.findings.iter().any(|f| f.id.starts_with("mem.swap")), "sin datos de swap no hay hallazgo de swap");
    let partial = r.findings.iter().find(|f| f.id == "net.external.partial").expect("partial network");
    assert!(partial.recommendation.contains("no demuestra por sí solo"));
    let failed = sink.0.lock().unwrap().iter().filter(|e| e.kind == EventKind::ToolResult && e.status == EventStatus::Failed).count();
    assert_eq!(failed, 3, "swap, vm_stat y TCP a 8.8.8.8");
    write_or_check_contract_fixture(&r);
}

// ---------- JSON contract with the frontend ----------

/// Replaces values that could identify this Mac before the report is stored.
fn redact(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(sys) = v.get_mut("system").filter(|s| s.is_object()) {
        sys["hostname"] = "mac-de-prueba".into();
        sys["os_version"] = "macOS".into();
    }
    if let Some(top) = v.pointer_mut("/memory/processes/top").and_then(|t| t.as_array_mut()) {
        for (i, p) in top.iter_mut().enumerate() {
            p["name"] = format!("proceso-{i}").into();
        }
    }
    if let Some(findings) = v.get_mut("findings").and_then(|f| f.as_array_mut()) {
        for f in findings.iter_mut().filter(|f| f["id"].as_str().is_some_and(|id| id.starts_with("mem.process"))) {
            f["title"] = "proceso de prueba".into();
            if let Some(ev) = f["evidence"].as_array_mut() {
                for (i, e) in ev.iter_mut().enumerate() {
                    e["label"] = format!("proceso-{i}").into();
                }
            }
        }
    }
    if let Some(ifaces) = v.pointer_mut("/network/interfaces").and_then(|t| t.as_array_mut()) {
        for i in ifaces {
            i["mac"] = "00:00:00:00:00:00".into();
            i["addresses"] = serde_json::json!(["192.0.2.10/24"]);
        }
    }
    v
}

/// Structural comparison: same object keys and value types; `null` matches anything.
fn same_shape(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Result<(), String> {
    use serde_json::Value as V;
    match (a, b) {
        (V::Null, _) | (_, V::Null) => Ok(()),
        (V::Object(x), V::Object(y)) => {
            let kx: Vec<_> = x.keys().collect();
            let ky: Vec<_> = y.keys().collect();
            if kx != ky {
                return Err(format!("{path}: claves {kx:?} != {ky:?}"));
            }
            x.iter().try_for_each(|(k, v)| same_shape(v, &y[k], &format!("{path}.{k}")))
        }
        (V::Array(x), V::Array(y)) => match (x.first(), y.first()) {
            (Some(p), Some(q)) => same_shape(p, q, &format!("{path}[0]")),
            _ => Ok(()),
        },
        (V::Number(_), V::Number(_)) | (V::String(_), V::String(_)) | (V::Bool(_), V::Bool(_)) => Ok(()),
        _ => Err(format!("{path}: tipo {a} != {b}")),
    }
}

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/lib/fixtures/diagnostic-report.json");

/// The frontend test renders this fixture with `DiagnosticReportView`. If the
/// Rust report changes shape, this fails until the fixture is regenerated with
/// `JARVIS_UPDATE_FIXTURES=1 cargo test -p jarvis-core --test diagnostics partial_results`.
fn write_or_check_contract_fixture(r: &DiagnosticReport) {
    let current = redact(serde_json::to_value(r).unwrap());
    if std::env::var("JARVIS_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(std::path::Path::new(FIXTURE).parent().unwrap()).unwrap();
        std::fs::write(FIXTURE, serde_json::to_string_pretty(&current).unwrap()).unwrap();
        return;
    }
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("falta el fixture de contrato")).unwrap();
    same_shape(&current, &stored, "report").expect("el JSON del informe cambió de forma respecto al fixture del frontend");
}
