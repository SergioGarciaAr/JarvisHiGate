//! Parsers for macOS command output. Fixtures in the tests were captured
//! from a real Mac (macOS 27, Apple Silicon).

use std::net::IpAddr;

use serde::Serialize;

use super::sysexec::ProbeError;

fn parse_err(message: impl Into<String>) -> ProbeError {
    ProbeError::Parse { message: message.into() }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VmStat {
    pub page_size: u64,
    pub free: u64,
    pub active: u64,
    pub inactive: u64,
    pub speculative: u64,
    pub wired: u64,
    pub purgeable: u64,
    pub file_backed: u64,
    pub anonymous: u64,
    /// Pages of data held compressed (logical size).
    pub compressed_stored: u64,
    /// Physical pages used by the compressor.
    pub compressor_occupied: u64,
    pub pageouts: u64,
    pub swapins: u64,
    pub swapouts: u64,
}

impl VmStat {
    pub fn bytes(&self, pages: u64) -> u64 {
        pages * self.page_size
    }
}

pub fn parse_vm_stat(out: &str) -> Result<VmStat, ProbeError> {
    let page_size = out
        .lines()
        .next()
        .and_then(|l| l.split("page size of ").nth(1))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse::<u64>().ok())
        .ok_or_else(|| parse_err("vm_stat: falta el tamaño de página"))?;
    let get = |key: &str| -> Result<u64, ProbeError> {
        out.lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                (k.trim().trim_matches('"') == key).then(|| v.trim().trim_end_matches('.').parse::<u64>().ok())?
            })
            .ok_or_else(|| parse_err(format!("vm_stat: falta «{key}»")))
    };
    Ok(VmStat {
        page_size,
        free: get("Pages free")?,
        active: get("Pages active")?,
        inactive: get("Pages inactive")?,
        speculative: get("Pages speculative")?,
        wired: get("Pages wired down")?,
        purgeable: get("Pages purgeable")?,
        file_backed: get("File-backed pages")?,
        anonymous: get("Anonymous pages")?,
        compressed_stored: get("Pages stored in compressor")?,
        compressor_occupied: get("Pages occupied by compressor")?,
        pageouts: get("Pageouts")?,
        swapins: get("Swapins")?,
        swapouts: get("Swapouts")?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureLevel {
    Normal,
    Warn,
    Critical,
}

/// `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warn, 4 critical.
pub fn parse_pressure_level(out: &str) -> Result<PressureLevel, ProbeError> {
    match out.trim() {
        "1" => Ok(PressureLevel::Normal),
        "2" => Ok(PressureLevel::Warn),
        "4" => Ok(PressureLevel::Critical),
        other => Err(parse_err(format!("nivel de presión desconocido: «{other}»"))),
    }
}

/// `kern.memorystatus_level`: percentage of memory the kernel considers free.
pub fn parse_free_level(out: &str) -> Result<u8, ProbeError> {
    out.trim()
        .parse::<u8>()
        .ok()
        .filter(|v| *v <= 100)
        .ok_or_else(|| parse_err(format!("nivel de memoria libre no válido: «{}»", out.trim())))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SwapUsage {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub encrypted: bool,
}

fn parse_size(token: &str) -> Option<u64> {
    let token = token.trim();
    let (num, unit) = token.split_at(token.len().checked_sub(1)?);
    // Accept both "1.50M" and the localized "1,50M".
    let value: f64 = num.replace(',', ".").parse().ok()?;
    let mult = match unit {
        "K" => 1024f64,
        "M" => 1024f64.powi(2),
        "G" => 1024f64.powi(3),
        "T" => 1024f64.powi(4),
        _ => return None,
    };
    Some((value * mult) as u64)
}

/// `sysctl -n vm.swapusage`: `total = 0.00M  used = 0.00M  free = 0.00M  (encrypted)`
pub fn parse_swap_usage(out: &str) -> Result<SwapUsage, ProbeError> {
    let field = |name: &str| -> Result<u64, ProbeError> {
        out.split(&format!("{name} = "))
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(parse_size)
            .ok_or_else(|| parse_err(format!("vm.swapusage: falta «{name}»")))
    };
    Ok(SwapUsage { total: field("total")?, used: field("used")?, free: field("free")?, encrypted: out.contains("encrypted") })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefaultRoute {
    pub gateway: String,
    pub interface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteResult {
    Route(DefaultRoute),
    NoDefaultRoute,
}

/// Parses `route -n get default` (stdout, stderr, exit code).
pub fn parse_route(stdout: &str, stderr: &str, success: bool) -> Result<RouteResult, ProbeError> {
    if !success {
        if stderr.contains("not in table") {
            return Ok(RouteResult::NoDefaultRoute);
        }
        return Err(ProbeError::Failed { code: None, stderr: stderr.trim().to_string() });
    }
    let value = |key: &str| {
        stdout.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    match value("gateway") {
        Some(gateway) => Ok(RouteResult::Route(DefaultRoute { gateway, interface: value("interface") })),
        None => Ok(RouteResult::NoDefaultRoute),
    }
}

/// Unique nameservers from `scutil --dns`, ignoring mDNS-only resolvers.
pub fn parse_dns_servers(out: &str) -> Vec<String> {
    let mut servers = Vec::new();
    for block in out.split("resolver #").skip(1) {
        if block.contains("options  : mdns") {
            continue;
        }
        for line in block.lines() {
            let line = line.trim();
            if line.starts_with("nameserver[")
                && let Some((_, v)) = line.split_once(':') {
                    let v = v.trim().to_string();
                    if v.parse::<IpAddr>().is_ok() && !servers.contains(&v) {
                        servers.push(v);
                    }
                }
        }
    }
    servers
}

/// Maps device names to service names from `networksetup -listnetworkserviceorder`.
pub fn parse_service_order(out: &str) -> Vec<(String, String)> {
    let mut map = Vec::new();
    let mut current: Option<String> = None;
    for line in out.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix('(') {
            if let Some((idx, name)) = rest.split_once(')')
                && idx.trim_start_matches('*').parse::<u32>().is_ok() {
                    current = Some(name.trim().to_string());
                    continue;
                }
            if let (Some(name), Some(dev)) = (current.take(), line.split("Device: ").nth(1)) {
                let dev = dev.trim_end_matches(')').trim();
                if !dev.is_empty() {
                    map.push((dev.to_string(), name));
                }
            }
        }
    }
    map
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PingStats {
    pub transmitted: u32,
    pub received: u32,
    pub avg_ms: Option<f64>,
    /// e.g. "No route to host", which on recent macOS often means the app
    /// lacks the Local Network permission.
    pub send_error: Option<String>,
}

pub fn parse_ping(stdout: &str, stderr: &str) -> Result<PingStats, ProbeError> {
    let line = stdout
        .lines()
        .find(|l| l.contains("packets transmitted"))
        .ok_or_else(|| parse_err(format!("ping: sin estadísticas ({})", stderr.trim())))?;
    let nums: Vec<u32> = line.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect();
    let (transmitted, received) = match nums.as_slice() {
        [t, r, ..] => (*t, *r),
        _ => return Err(parse_err("ping: estadísticas incompletas")),
    };
    let avg_ms = stdout
        .lines()
        .find(|l| l.starts_with("round-trip"))
        .and_then(|l| l.split('=').nth(1))
        .and_then(|v| v.trim().split('/').nth(1))
        .and_then(|v| v.parse().ok());
    let send_error = stderr
        .lines()
        .chain(stdout.lines())
        .find_map(|l| l.strip_prefix("ping: sendto: "))
        .map(str::to_string);
    Ok(PingStats { transmitted, received, avg_ms, send_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VM_STAT: &str = "Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                                   283059.
Pages active:                                 253993.
Pages inactive:                               243345.
Pages speculative:                             10233.
Pages throttled:                                   0.
Pages wired down:                              99243.
Pages purgeable:                                9440.
\"Translation faults\":                       40826960.
Pages copy-on-write:                         1112010.
Pages zero filled:                          31772328.
Pages reactivated:                           1110451.
Pages purged:                                 281678.
File-backed pages:                            178143.
Anonymous pages:                              329428.
Pages stored in compressor:                   282266.
Pages occupied by compressor:                 119346.
Decompressions:                               976818.
Compressions:                                1454870.
Pageins:                                     2084717.
Pageouts:                                       1067.
Swapins:                                           0.
Swapouts:                                          0.
";

    #[test]
    fn vm_stat_real_output() {
        let v = parse_vm_stat(VM_STAT).unwrap();
        assert_eq!(v.page_size, 16384);
        assert_eq!(v.free, 283059);
        assert_eq!(v.wired, 99243);
        assert_eq!(v.compressor_occupied, 119346);
        assert_eq!(v.swapouts, 0);
        assert_eq!(v.bytes(2), 32768);
    }

    #[test]
    fn vm_stat_truncated_is_error() {
        assert!(parse_vm_stat("Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free: 1.\n").is_err());
        assert!(parse_vm_stat("").is_err());
    }

    #[test]
    fn sysctl_values() {
        assert_eq!(parse_pressure_level("1\n").unwrap(), PressureLevel::Normal);
        assert_eq!(parse_pressure_level("4").unwrap(), PressureLevel::Critical);
        assert!(parse_pressure_level("7").is_err());
        assert_eq!(parse_free_level("78\n").unwrap(), 78);
        assert!(parse_free_level("130").is_err());
    }

    #[test]
    fn swap_both_locales() {
        let s = parse_swap_usage("total = 2048.00M  used = 1024,50M  free = 1023.50M  (encrypted)").unwrap();
        assert_eq!(s.total, 2048 * 1024 * 1024);
        assert_eq!(s.used, (1024.5 * 1024.0 * 1024.0) as u64);
        assert!(s.encrypted);
        let s = parse_swap_usage("total = 0.00M  used = 0.00M  free = 0.00M  (encrypted)").unwrap();
        assert_eq!(s.used, 0);
        assert!(parse_swap_usage("garbage").is_err());
    }

    #[test]
    fn route_real_output_and_no_route() {
        let out = "   route to: default
destination: default
       mask: default
    gateway: 192.168.1.1
  interface: en0
      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING,GLOBAL>
";
        assert_eq!(
            parse_route(out, "", true).unwrap(),
            RouteResult::Route(DefaultRoute { gateway: "192.168.1.1".into(), interface: Some("en0".into()) })
        );
        assert_eq!(
            parse_route("", "route: writing to routing socket: not in table\n", false).unwrap(),
            RouteResult::NoDefaultRoute
        );
        assert!(parse_route("", "route: must be root\n", false).is_err());
    }

    #[test]
    fn dns_real_output_skips_mdns() {
        let out = "DNS configuration

resolver #1
  nameserver[0] : 200.48.225.130
  nameserver[1] : 200.48.225.146
  if_index : 14 (en0)
  reach    : 0x00000002 (Reachable)

resolver #2
  domain   : local
  options  : mdns
  timeout  : 5
";
        assert_eq!(parse_dns_servers(out), vec!["200.48.225.130", "200.48.225.146"]);
        assert!(parse_dns_servers("No DNS configuration available\n").is_empty());
    }

    #[test]
    fn service_order_real_output() {
        let out = "An asterisk (*) denotes that a network service is disabled.
(1) Thunderbolt Bridge
(Hardware Port: Thunderbolt Bridge, Device: bridge0)

(2) Wi-Fi
(Hardware Port: Wi-Fi, Device: en0)

(*) VPN
(Hardware Port: L2TP, Device: )
";
        assert_eq!(
            parse_service_order(out),
            vec![("bridge0".into(), "Thunderbolt Bridge".into()), ("en0".into(), "Wi-Fi".into())]
        );
    }

    #[test]
    fn ping_outputs() {
        let ok = "PING 192.168.1.1 (192.168.1.1): 56 data bytes
64 bytes from 192.168.1.1: icmp_seq=0 ttl=64 time=3.012 ms

--- 192.168.1.1 ping statistics ---
2 packets transmitted, 2 packets received, 0.0% packet loss
round-trip min/avg/max/stddev = 2.900/3.100/3.300/0.200 ms
";
        let p = parse_ping(ok, "").unwrap();
        assert_eq!((p.transmitted, p.received), (2, 2));
        assert_eq!(p.avg_ms, Some(3.1));

        // Real output captured on this Mac: Local Network permission missing.
        let blocked_out = "--- 192.168.1.1 ping statistics ---
2 packets transmitted, 0 packets received, 100.0% packet loss
";
        let p = parse_ping(blocked_out, "ping: sendto: No route to host\nping: sendto: No route to host\n").unwrap();
        assert_eq!(p.received, 0);
        assert_eq!(p.send_error.as_deref(), Some("No route to host"));
        assert!(parse_ping("", "ping: cannot resolve x: Unknown host").is_err());
    }
}
