//! Network analysis. Pure functions over collected data.

use serde::Serialize;

use super::finding::{Confidence as C, Evidence, Finding, FindingKind as K, Severity as S};
use super::parse::{DefaultRoute, PingStats};
use crate::telemetry::InterfaceInfo;

pub const NOTE: &str = "Las pruebas de red usan destinos concretos con un límite de tiempo. Que un destino no responda no \
demuestra por sí solo que toda la conexión a Internet esté caída: puede deberse a un cortafuegos, a un filtrado de la red \
o a un problema puntual de ese destino.";

/// Fixed destinations, independent of DNS.
pub const TCP_TARGETS: [(&str, [u8; 4], u16); 2] = [("Cloudflare", [1, 1, 1, 1], 443), ("Google", [8, 8, 8, 8], 443)];
pub const DNS_HOSTS: [&str; 2] = ["apple.com", "cloudflare.com"];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RouteState {
    Unknown,
    NoDefault,
    Default(DefaultRoute),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DnsCheck {
    pub host: String,
    pub addresses: Vec<String>,
    pub ms: Option<u64>,
    pub error: Option<String>,
}

impl DnsCheck {
    pub fn ok(&self) -> bool {
        self.error.is_none() && !self.addresses.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TcpOutcome {
    Connected,
    /// The host answered with a reset: it is reachable, the port is closed.
    Refused,
    Timeout,
    Unreachable,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TcpCheck {
    pub label: String,
    pub target: String,
    /// Whether the target was resolved through DNS first.
    pub via_dns: bool,
    pub outcome: TcpOutcome,
    pub ms: Option<u64>,
    pub detail: Option<String>,
}

impl TcpCheck {
    pub fn reached(&self) -> bool {
        matches!(self.outcome, TcpOutcome::Connected | TcpOutcome::Refused)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GatewayCheck {
    pub gateway: String,
    pub stats: PingStats,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NetworkData {
    pub interfaces: Vec<InterfaceInfo>,
    /// (device, service name), e.g. ("en0", "Wi-Fi").
    pub services: Vec<(String, String)>,
    pub route: RouteState,
    pub dns_servers: Option<Vec<String>>,
    pub gateway: Option<GatewayCheck>,
    pub dns: Vec<DnsCheck>,
    pub tcp: Vec<TcpCheck>,
}

impl Default for NetworkData {
    fn default() -> Self {
        Self {
            interfaces: Vec::new(),
            services: Vec::new(),
            route: RouteState::Unknown,
            dns_servers: None,
            gateway: None,
            dns: Vec::new(),
            tcp: Vec::new(),
        }
    }
}

impl NetworkData {
    fn service_name(&self, device: &str) -> Option<&str> {
        self.services.iter().find(|(d, _)| d == device).map(|(_, n)| n.as_str())
    }

    pub fn active_interfaces(&self) -> impl Iterator<Item = &InterfaceInfo> {
        self.interfaces.iter().filter(|i| i.has_routable_ip && !i.name.starts_with("lo") && i.state != "down")
    }
}

fn tcp_evidence(c: &TcpCheck) -> Evidence {
    let value = match (c.outcome, c.ms) {
        (TcpOutcome::Connected, Some(ms)) => format!("conectado en {ms} ms"),
        (TcpOutcome::Refused, _) => "rechazado (el destino responde)".into(),
        (TcpOutcome::Timeout, _) => "sin respuesta (tiempo agotado)".into(),
        (TcpOutcome::Unreachable, _) => format!("inalcanzable: {}", c.detail.as_deref().unwrap_or("")),
        _ => c.detail.clone().unwrap_or_else(|| "error".into()),
    };
    Evidence::new(format!("{} ({})", c.label, c.target), value, "conexión TCP")
}

fn dns_evidence(d: &DnsCheck) -> Evidence {
    let value = match (&d.error, d.ms) {
        (Some(e), _) => format!("falló: {e}"),
        (None, Some(ms)) => format!("{} en {ms} ms", d.addresses.first().cloned().unwrap_or_default()),
        (None, None) => d.addresses.join(", "),
    };
    Evidence::new(format!("Resolver {}", d.host), value, "resolución DNS del sistema")
}

const CAVEAT: &str = " Recuerda que un destino inaccesible no demuestra por sí solo que toda la conexión esté caída.";

pub fn analyze_network(d: &NetworkData) -> Vec<Finding> {
    let mut out = Vec::new();
    let active: Vec<&InterfaceInfo> = d.active_interfaces().collect();

    if !d.interfaces.is_empty() && active.is_empty() {
        out.push(Finding {
            id: "net.iface.none",
            kind: K::Problem,
            severity: S::High,
            title: "Ninguna interfaz de red tiene una dirección utilizable".into(),
            evidence: d
                .interfaces
                .iter()
                .filter(|i| i.name.starts_with("en"))
                .map(|i| Evidence::new(i.name.clone(), format!("{} · {}", i.state, if i.addresses.is_empty() { "sin IP".into() } else { i.addresses.join(", ") }), "sysinfo"))
                .collect(),
            confidence: C::High,
            recommendation: "Comprueba que el Wi-Fi está activado y conectado a una red, o que el cable está conectado.".into(),
            proposed_action: None,
        });
    }

    match &d.route {
        RouteState::NoDefault => out.push(Finding {
            id: "net.route.none",
            kind: K::Problem,
            severity: S::High,
            title: "No hay ruta por defecto".into(),
            evidence: vec![Evidence::new("Ruta por defecto", "no existe", "route -n get default")],
            confidence: C::High,
            recommendation: "El Mac no sabe por dónde salir a Internet. Reconecta la red Wi-Fi o el cable; si usas una \
VPN, desconéctala y vuelve a probar."
                .into(),
            proposed_action: None,
        }),
        RouteState::Default(r) => {
            let iface = r.interface.clone().unwrap_or_else(|| "?".into());
            let service = d.service_name(&iface).map(|s| format!(" ({s})")).unwrap_or_default();
            out.push(Finding {
                id: "net.route.ok",
                kind: K::Observation,
                severity: S::Info,
                title: format!("Salida por {iface}{service} hacia {}", r.gateway),
                evidence: vec![
                    Evidence::new("Puerta de enlace", r.gateway.clone(), "route -n get default"),
                    Evidence::new("Interfaz", format!("{iface}{service}"), "route -n get default"),
                ],
                confidence: C::High,
                recommendation: "No hace falta ninguna acción.".into(),
                proposed_action: None,
            });
        }
        RouteState::Unknown => {}
    }

    let ip_checks: Vec<&TcpCheck> = d.tcp.iter().filter(|c| !c.via_dns).collect();
    let any_ip_ok = ip_checks.iter().any(|c| c.reached());
    let all_ip_failed = !ip_checks.is_empty() && !any_ip_ok;
    let dns_ran = !d.dns.is_empty();
    let dns_ok = d.dns.iter().any(DnsCheck::ok);

    if !d.tcp.is_empty() {
        let evidence: Vec<Evidence> = d.tcp.iter().map(tcp_evidence).collect();
        if all_ip_failed && dns_ran && !dns_ok {
            out.push(Finding {
                id: "net.external.down",
                kind: K::Problem,
                severity: S::High,
                title: "No se alcanzó ningún destino externo".into(),
                evidence: evidence.into_iter().chain(d.dns.iter().map(dns_evidence)).collect(),
                confidence: C::Medium,
                recommendation: format!(
                    "Fallaron tanto las conexiones directas por IP como la resolución DNS. Comprueba la conexión del router \
o del proveedor; si usas VPN o proxy, revísalos.{CAVEAT}"
                ),
                proposed_action: None,
            });
        } else if all_ip_failed {
            out.push(Finding {
                id: "net.external.ip_blocked",
                kind: K::Warning,
                severity: S::Medium,
                title: "Los destinos por IP no respondieron, pero DNS sí".into(),
                evidence,
                confidence: C::Low,
                recommendation: format!(
                    "Puede haber un cortafuegos o filtrado que bloquee esos destinos concretos. Prueba a abrir una web \
en el navegador.{CAVEAT}"
                ),
                proposed_action: None,
            });
        } else if d.tcp.iter().any(|c| !c.reached()) {
            out.push(Finding {
                id: "net.external.partial",
                kind: K::Warning,
                severity: S::Low,
                title: "Algún destino no respondió".into(),
                evidence,
                confidence: C::Low,
                recommendation: format!("Otros destinos sí respondieron, así que la conexión funciona.{CAVEAT}"),
                proposed_action: None,
            });
        } else {
            out.push(Finding {
                id: "net.external.ok",
                kind: K::Observation,
                severity: S::Info,
                title: "Conectividad externa correcta".into(),
                evidence,
                confidence: C::High,
                recommendation: "No hace falta ninguna acción.".into(),
                proposed_action: None,
            });
        }
        let slow: Vec<&TcpCheck> = d.tcp.iter().filter(|c| c.outcome == TcpOutcome::Connected && c.ms.unwrap_or(0) > 300).collect();
        if !slow.is_empty() {
            out.push(Finding {
                id: "net.latency.high",
                kind: K::Warning,
                severity: S::Low,
                title: "Latencia de conexión elevada".into(),
                evidence: slow.iter().map(|c| tcp_evidence(c)).collect(),
                confidence: C::Medium,
                recommendation: "Más de 300 ms para abrir una conexión. Puede deberse a una red Wi-Fi saturada o a la \
distancia al servidor; repite la prueba para confirmarlo."
                    .into(),
                proposed_action: None,
            });
        }
    }

    if dns_ran && !dns_ok && any_ip_ok {
        let mut evidence: Vec<Evidence> = d.dns.iter().map(dns_evidence).collect();
        if let Some(servers) = &d.dns_servers {
            evidence.push(Evidence::new("Servidores DNS", servers.join(", "), "scutil --dns"));
        }
        out.push(Finding {
            id: "net.dns.failing",
            kind: K::Problem,
            severity: S::Medium,
            title: "La resolución DNS falla".into(),
            evidence,
            confidence: C::High,
            recommendation: "Hay conexión por IP pero los nombres no se resuelven. Revisa los servidores DNS en Ajustes \
del Sistema › Red › Detalles › DNS."
                .into(),
            proposed_action: Some("Probar manualmente otros servidores DNS (p. ej. los del proveedor o 1.1.1.1)".into()),
        });
    } else if dns_ok {
        let slow: Vec<&DnsCheck> = d.dns.iter().filter(|c| c.ms.unwrap_or(0) > 1000).collect();
        if !slow.is_empty() {
            out.push(Finding {
                id: "net.dns.slow",
                kind: K::Warning,
                severity: S::Low,
                title: "Resolución DNS lenta".into(),
                evidence: slow.iter().map(|c| dns_evidence(c)).collect(),
                confidence: C::Medium,
                recommendation: "Más de un segundo para resolver un nombre. Si se repite, prueba otros servidores DNS.".into(),
                proposed_action: None,
            });
        }
    }
    if matches!(&d.dns_servers, Some(s) if s.is_empty()) {
        out.push(Finding {
            id: "net.dns.none",
            kind: K::Warning,
            severity: S::Medium,
            title: "No hay servidores DNS configurados".into(),
            evidence: vec![Evidence::new("Servidores DNS", "ninguno", "scutil --dns")],
            confidence: C::High,
            recommendation: "Sin servidores DNS no se pueden resolver nombres. Reconecta la red para obtenerlos por DHCP.".into(),
            proposed_action: None,
        });
    }

    if let Some(gw) = &d.gateway {
        let external_ok = any_ip_ok || dns_ok;
        let mut evidence = vec![Evidence::new(
            format!("Ping a {}", gw.gateway),
            format!(
                "{}/{} respuestas{}",
                gw.stats.received,
                gw.stats.transmitted,
                gw.stats.avg_ms.map(|a| format!(", media {a:.1} ms")).unwrap_or_default()
            ),
            "ping",
        )];
        if let Some(e) = &gw.stats.send_error {
            evidence.push(Evidence::new("Error de envío", e.clone(), "ping"));
        }
        let no_route = gw.stats.send_error.as_deref().is_some_and(|e| e.contains("No route to host"));
        if gw.stats.received > 0 {
            out.push(Finding {
                id: "net.gateway.ok",
                kind: K::Observation,
                severity: S::Info,
                title: "El router responde".into(),
                evidence,
                confidence: C::High,
                recommendation: "No hace falta ninguna acción.".into(),
                proposed_action: None,
            });
        } else if no_route {
            out.push(Finding {
                id: "net.gateway.local_network_permission",
                kind: if external_ok { K::Observation } else { K::Warning },
                severity: if external_ok { S::Info } else { S::Medium },
                title: "No se pudo contactar con el router desde Jarvis".into(),
                evidence,
                confidence: C::Low,
                recommendation: "En macOS reciente, «No route to host» hacia la red local suele significar que la app no \
tiene el permiso de Red local (Ajustes del Sistema › Privacidad y seguridad › Red local). También es posible que el \
router no responda a ping."
                    .into(),
                proposed_action: None,
            });
        } else {
            out.push(Finding {
                id: "net.gateway.no_reply",
                kind: if external_ok { K::Observation } else { K::Warning },
                severity: if external_ok { S::Info } else { S::Medium },
                title: "El router no respondió al ping".into(),
                evidence,
                confidence: C::Low,
                recommendation: if external_ok {
                    "Muchos routers no responden a ping; como hay conexión externa, no indica un problema.".into()
                } else {
                    "Comprueba que el router está encendido y que estás conectado a la red correcta.".into()
                },
                proposed_action: None,
            });
        }
    }

    for i in &active {
        let packets = i.total_rx_packets + i.total_tx_packets;
        let errors = i.rx_errors + i.tx_errors;
        if packets > 1000 && errors * 100 > packets {
            out.push(Finding {
                id: "net.iface.errors",
                kind: K::Warning,
                severity: S::Low,
                title: format!("Errores de transmisión en {}", i.name),
                evidence: vec![Evidence::new(i.name.clone(), format!("{errors} errores en {packets} paquetes"), "sysinfo")],
                confidence: C::Medium,
                recommendation: "Más del 1 % de paquetes con error. Puede deberse a interferencias Wi-Fi o a un cable dañado.".into(),
                proposed_action: None,
            });
        }
    }

    out
}
