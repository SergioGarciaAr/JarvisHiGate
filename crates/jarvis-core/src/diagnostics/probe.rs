//! Access to the real system, behind a trait so analysis can be tested with
//! controlled failures (permission errors, timeouts) without touching the Mac.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;

use super::sysexec::{CommandOutput, FixedCommand, ProbeError, run_fixed};

pub trait SystemProbe: Send + Sync {
    fn command(&self, cmd: FixedCommand) -> BoxFuture<'_, Result<CommandOutput, ProbeError>>;
    fn resolve(&self, host: &'static str) -> BoxFuture<'_, Result<Vec<IpAddr>, ProbeError>>;
    /// Returns the time to establish the TCP connection.
    fn tcp_connect(&self, addr: SocketAddr) -> BoxFuture<'_, Result<Duration, ProbeError>>;
    fn net_timeout(&self) -> Duration;
}

/// Real probe for macOS.
pub struct MacProbe {
    pub command_timeout: Duration,
    pub net_timeout: Duration,
}

impl Default for MacProbe {
    fn default() -> Self {
        Self { command_timeout: Duration::from_secs(5), net_timeout: Duration::from_secs(3) }
    }
}

pub(crate) fn classify_io(e: &std::io::Error) -> ProbeError {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::ConnectionRefused => ProbeError::Refused,
        K::HostUnreachable | K::NetworkUnreachable | K::AddrNotAvailable => {
            ProbeError::Unreachable { message: e.to_string() }
        }
        K::PermissionDenied => ProbeError::PermissionDenied { path: "socket".into() },
        _ => ProbeError::Io { message: e.to_string() },
    }
}

impl SystemProbe for MacProbe {
    fn command(&self, cmd: FixedCommand) -> BoxFuture<'_, Result<CommandOutput, ProbeError>> {
        Box::pin(run_fixed(cmd, self.command_timeout))
    }

    fn resolve(&self, host: &'static str) -> BoxFuture<'_, Result<Vec<IpAddr>, ProbeError>> {
        let timeout = self.net_timeout;
        Box::pin(async move {
            let addrs = tokio::time::timeout(timeout, tokio::net::lookup_host((host, 443)))
                .await
                .map_err(|_| ProbeError::Timeout { ms: timeout.as_millis() as u64 })?
                .map_err(|e| ProbeError::Unreachable { message: e.to_string() })?;
            let mut ips: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
            ips.dedup();
            Ok(ips)
        })
    }

    fn tcp_connect(&self, addr: SocketAddr) -> BoxFuture<'_, Result<Duration, ProbeError>> {
        let timeout = self.net_timeout;
        Box::pin(async move {
            let started = Instant::now();
            match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
                Err(_) => Err(ProbeError::Timeout { ms: timeout.as_millis() as u64 }),
                Ok(Err(e)) => Err(classify_io(&e)),
                Ok(Ok(_stream)) => Ok(started.elapsed()),
            }
        })
    }

    fn net_timeout(&self) -> Duration {
        self.net_timeout
    }
}
