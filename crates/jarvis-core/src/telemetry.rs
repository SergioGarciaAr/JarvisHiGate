//! Real system metrics via `sysinfo`. A metric that cannot be read is
//! reported as unavailable with a reason, never as zero.

use std::time::Instant;

use serde::Serialize;
use std::net::IpAddr;

use sysinfo::{Disks, InterfaceOperationalState, Networks, ProcessesToUpdate, System};

#[derive(Debug, Clone, Serialize)]
pub struct Unavailable {
    pub metric: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskInfo {
    pub mount: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySnapshot {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub hostname: Option<String>,
    pub os_version: Option<String>,
    pub uptime_secs: u64,
    pub cpu_percent: Option<f32>,
    pub cpu_cores: usize,
    pub load_avg: Option<[f64; 3]>,
    pub mem_total: Option<u64>,
    pub mem_used: Option<u64>,
    pub mem_available: Option<u64>,
    pub swap_total: Option<u64>,
    pub swap_used: Option<u64>,
    pub disk: Option<DiskInfo>,
    /// Bytes per second since the previous snapshot (excluding loopback).
    pub net_rx_per_sec: Option<f64>,
    pub net_tx_per_sec: Option<f64>,
    pub unavailable: Vec<Unavailable>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessMem {
    pub pid: u32,
    pub name: String,
    /// Resident memory in bytes.
    pub memory: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessSummary {
    pub top: Vec<ProcessMem>,
    pub total: usize,
    /// Processes whose memory could not be read (usually other users' or
    /// system processes, which macOS hides from unprivileged apps).
    pub without_data: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InterfaceInfo {
    pub name: String,
    pub mac: String,
    pub mtu: u64,
    pub state: &'static str,
    pub addresses: Vec<String>,
    /// Has an address that is neither loopback nor link-local.
    pub has_routable_ip: bool,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub total_rx_packets: u64,
    pub total_tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
}

fn is_routable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified(),
        IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unspecified() && (v6.segments()[0] & 0xffc0) != 0xfe80,
    }
}

pub struct TelemetryService {
    sys: System,
    disks: Disks,
    networks: Networks,
    last_net: Option<Instant>,
    last: Option<TelemetrySnapshot>,
}

impl Default for TelemetryService {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryService {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        Self {
            sys,
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            last_net: None,
            last: None,
        }
    }

    /// Most recent periodic snapshot, or a fresh one if none exists yet.
    /// Reusing the cached value avoids skewing the CPU and network-rate
    /// intervals of the periodic sampler.
    pub fn latest(&mut self) -> TelemetrySnapshot {
        match &self.last {
            Some(s) => s.clone(),
            None => self.snapshot(),
        }
    }

    /// Processes sorted by resident memory.
    pub fn processes_by_memory(&mut self, limit: usize) -> ProcessSummary {
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        let mut all: Vec<ProcessMem> = self
            .sys
            .processes()
            .values()
            .map(|p| ProcessMem { pid: p.pid().as_u32(), name: p.name().to_string_lossy().into_owned(), memory: p.memory() })
            .collect();
        let total = all.len();
        let without_data = all.iter().filter(|p| p.memory == 0).count();
        all.sort_by_key(|p| std::cmp::Reverse(p.memory));
        all.truncate(limit);
        ProcessSummary { top: all, total, without_data }
    }

    /// Network interfaces as of the last periodic refresh (totals are cumulative).
    pub fn interfaces(&self) -> Vec<InterfaceInfo> {
        let mut list: Vec<InterfaceInfo> = self
            .networks
            .iter()
            .map(|(name, d)| {
                let ips: Vec<_> = d.ip_networks().to_vec();
                InterfaceInfo {
                    name: name.clone(),
                    mac: d.mac_address().to_string(),
                    mtu: d.mtu(),
                    state: match d.operational_state() {
                        InterfaceOperationalState::Up => "up",
                        InterfaceOperationalState::Down => "down",
                        InterfaceOperationalState::Dormant => "dormant",
                        _ => "unknown",
                    },
                    has_routable_ip: ips.iter().any(|n| is_routable(&n.addr)),
                    addresses: ips.iter().map(|n| format!("{}/{}", n.addr, n.prefix)).collect(),
                    total_rx_bytes: d.total_received(),
                    total_tx_bytes: d.total_transmitted(),
                    total_rx_packets: d.total_packets_received(),
                    total_tx_packets: d.total_packets_transmitted(),
                    rx_errors: d.total_errors_on_received(),
                    tx_errors: d.total_errors_on_transmitted(),
                }
            })
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Call at intervals ≥ `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`; the first
    /// call has no CPU or network rate baseline and reports them unavailable.
    pub fn snapshot(&mut self) -> TelemetrySnapshot {
        let first = self.last_net.is_none();
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.disks.refresh(true);
        self.networks.refresh(true);
        let now = Instant::now();
        let elapsed = self.last_net.map(|t| now.duration_since(t).as_secs_f64());
        self.last_net = Some(now);

        let mut unavailable = Vec::new();

        let cpu_percent = if first {
            unavailable.push(Unavailable { metric: "cpu", reason: "Calculando la primera muestra".into() });
            None
        } else {
            Some(self.sys.global_cpu_usage())
        };

        let load = System::load_average();
        let load_avg = Some([load.one, load.five, load.fifteen]);

        let total = self.sys.total_memory();
        let (mem_total, mem_used, mem_available) = if total == 0 {
            unavailable.push(Unavailable { metric: "memoria", reason: "El sistema no informó de la memoria total".into() });
            (None, None, None)
        } else {
            (Some(total), Some(self.sys.used_memory()), Some(self.sys.available_memory()))
        };

        // On macOS the writable APFS volume is mounted at /System/Volumes/Data.
        let disk = self
            .disks
            .list()
            .iter()
            .find(|d| d.mount_point().as_os_str() == "/System/Volumes/Data")
            .or_else(|| self.disks.list().iter().find(|d| d.mount_point().as_os_str() == "/"))
            .map(|d| DiskInfo {
                mount: d.mount_point().display().to_string(),
                total_bytes: d.total_space(),
                available_bytes: d.available_space(),
            });
        if disk.is_none() {
            unavailable.push(Unavailable { metric: "disco", reason: "No se encontró el volumen principal".into() });
        }

        let (net_rx_per_sec, net_tx_per_sec) = match elapsed {
            Some(secs) if secs > 0.0 => {
                let (rx, tx) = self
                    .networks
                    .iter()
                    .filter(|(name, _)| !name.starts_with("lo"))
                    .fold((0u64, 0u64), |(rx, tx), (_, d)| (rx + d.received(), tx + d.transmitted()));
                (Some(rx as f64 / secs), Some(tx as f64 / secs))
            }
            _ => {
                unavailable.push(Unavailable { metric: "red", reason: "Calculando la primera muestra".into() });
                (None, None)
            }
        };

        let snapshot = TelemetrySnapshot {
            timestamp: chrono::Utc::now(),
            hostname: System::host_name(),
            os_version: System::long_os_version(),
            uptime_secs: System::uptime(),
            cpu_percent,
            cpu_cores: self.sys.cpus().len(),
            load_avg,
            mem_total,
            mem_used,
            mem_available,
            swap_total: Some(self.sys.total_swap()),
            swap_used: Some(self.sys.used_swap()),
            disk,
            net_rx_per_sec,
            net_tx_per_sec,
            unavailable,
        };
        self.last = Some(snapshot.clone());
        snapshot
    }
}
