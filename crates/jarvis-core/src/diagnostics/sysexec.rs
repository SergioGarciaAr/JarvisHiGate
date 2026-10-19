//! Execution of a closed set of read-only system commands.
//!
//! Every command is an enum variant with an absolute executable path and
//! fixed arguments. There is no shell and no way to pass arbitrary strings:
//! the only variable argument is a gateway address, typed as `Ipv4Addr`.
//! `memory_pressure` is deliberately absent: it applies or simulates
//! pressure instead of just reading it.

use std::net::Ipv4Addr;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixedCommand {
    VmStat,
    SysctlPressureLevel,
    SysctlFreeLevel,
    SysctlSwapUsage,
    RouteDefault,
    ScutilDns,
    NetworkServiceOrder,
    PingGateway(Ipv4Addr),
    /// Lists installed voices (`say -v ?`); used by the voice module.
    SayListVoices,
}

impl FixedCommand {
    pub fn program(&self) -> &'static str {
        match self {
            FixedCommand::VmStat => "/usr/bin/vm_stat",
            FixedCommand::SysctlPressureLevel | FixedCommand::SysctlFreeLevel | FixedCommand::SysctlSwapUsage => {
                "/usr/sbin/sysctl"
            }
            FixedCommand::RouteDefault => "/sbin/route",
            FixedCommand::ScutilDns => "/usr/sbin/scutil",
            FixedCommand::NetworkServiceOrder => "/usr/sbin/networksetup",
            FixedCommand::PingGateway(_) => "/sbin/ping",
            FixedCommand::SayListVoices => "/usr/bin/say",
        }
    }

    pub fn args(&self) -> Vec<String> {
        let fixed: &[&str] = match self {
            FixedCommand::VmStat => &[],
            FixedCommand::SysctlPressureLevel => &["-n", "kern.memorystatus_vm_pressure_level"],
            FixedCommand::SysctlFreeLevel => &["-n", "kern.memorystatus_level"],
            FixedCommand::SysctlSwapUsage => &["-n", "vm.swapusage"],
            FixedCommand::RouteDefault => &["-n", "get", "default"],
            FixedCommand::ScutilDns => &["--dns"],
            FixedCommand::NetworkServiceOrder => &["-listnetworkserviceorder"],
            // 2 packets, 3 s overall deadline, numeric output.
            FixedCommand::PingGateway(_) => &["-n", "-c", "2", "-t", "3"],
            FixedCommand::SayListVoices => &["-v", "?"],
        };
        let mut args: Vec<String> = fixed.iter().map(|s| s.to_string()).collect();
        if let FixedCommand::PingGateway(ip) = self {
            args.push(ip.to_string());
        }
        args
    }

    /// Human-readable form for progress events and evidence.
    pub fn display(&self) -> String {
        let name = self.program().rsplit('/').next().unwrap_or_default();
        let args = self.args();
        if args.is_empty() { name.to_string() } else { format!("{name} {}", args.join(" ")) }
    }
}

/// Only unicast addresses may be pinged: never broadcast, multicast,
/// unspecified or loopback, even if the routing table reports them.
pub fn is_pingable_gateway(ip: Ipv4Addr) -> bool {
    !(ip.is_broadcast() || ip.is_multicast() || ip.is_unspecified() || ip.is_loopback())
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProbeError {
    #[error("no se encontró el ejecutable {path}")]
    NotFound { path: String },
    #[error("permiso denegado al ejecutar {path}")]
    PermissionDenied { path: String },
    #[error("sin respuesta en {ms} ms")]
    Timeout { ms: u64 },
    #[error("el comando terminó con código {code:?}: {stderr}")]
    Failed { code: Option<i32>, stderr: String },
    #[error("error de E/S: {message}")]
    Io { message: String },
    #[error("salida no reconocida: {message}")]
    Parse { message: String },
    #[error("conexión rechazada (el equipo responde, pero el puerto está cerrado)")]
    Refused,
    #[error("destino inalcanzable: {message}")]
    Unreachable { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Returns stdout for exit code 0, otherwise a `Failed` error.
    pub fn into_stdout(self) -> Result<String, ProbeError> {
        if self.success() {
            Ok(self.stdout)
        } else {
            Err(ProbeError::Failed { code: self.code, stderr: self.stderr.trim().to_string() })
        }
    }
}

/// Runs an executable with fixed arguments, a clean locale and a deadline.
/// Non-zero exit codes are returned as output; callers decide what they mean.
pub(crate) async fn run_program(program: &str, args: &[String], timeout: Duration) -> Result<CommandOutput, ProbeError> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ProbeError::NotFound { path: program.to_string() },
        std::io::ErrorKind::PermissionDenied => ProbeError::PermissionDenied { path: program.to_string() },
        _ => ProbeError::Io { message: e.to_string() },
    })?;
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| ProbeError::Timeout { ms: timeout.as_millis() as u64 })?
        .map_err(|e| ProbeError::Io { message: e.to_string() })?;
    Ok(CommandOutput {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

pub async fn run_fixed(cmd: FixedCommand, timeout: Duration) -> Result<CommandOutput, ProbeError> {
    run_program(cmd.program(), &cmd.args(), timeout).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // These exercise real OS error paths, not simulations. Tests that are not
    // about timeouts use a generous deadline so machine load cannot flake them.
    const NOT_A_TIMEOUT_TEST: Duration = Duration::from_secs(15);

    #[tokio::test]
    async fn missing_executable_is_not_found() {
        let err = run_program("/nonexistent/jarvis-test-bin", &[], NOT_A_TIMEOUT_TEST).await.unwrap_err();
        assert!(matches!(err, ProbeError::NotFound { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn non_executable_file_is_permission_denied() {
        let dir = std::env::temp_dir().join(format!("jarvis-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not-executable");
        std::fs::write(&file, "#!/bin/sh\necho hi\n").unwrap();
        let err = run_program(file.to_str().unwrap(), &[], NOT_A_TIMEOUT_TEST).await.unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(err, ProbeError::PermissionDenied { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn slow_command_times_out_and_is_killed() {
        let started = std::time::Instant::now();
        let err = run_program("/bin/sleep", &["5".into()], Duration::from_millis(150)).await.unwrap_err();
        assert_eq!(err, ProbeError::Timeout { ms: 150 });
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn non_zero_exit_is_reported() {
        let out = run_program("/usr/bin/false", &[], NOT_A_TIMEOUT_TEST).await.unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!out.success());
        assert!(matches!(out.into_stdout(), Err(ProbeError::Failed { code: Some(1), .. })));
    }

    #[test]
    fn gateway_validation_rejects_non_unicast() {
        assert!(is_pingable_gateway(Ipv4Addr::new(192, 168, 1, 1)));
        assert!(is_pingable_gateway(Ipv4Addr::new(10, 0, 0, 1)));
        for bad in [
            Ipv4Addr::BROADCAST,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::new(224, 0, 0, 1),
            Ipv4Addr::new(239, 255, 255, 250),
        ] {
            assert!(!is_pingable_gateway(bad), "{bad}");
        }
    }

    #[test]
    fn commands_have_fixed_absolute_programs() {
        let all = [
            FixedCommand::VmStat,
            FixedCommand::SysctlPressureLevel,
            FixedCommand::SysctlFreeLevel,
            FixedCommand::SysctlSwapUsage,
            FixedCommand::RouteDefault,
            FixedCommand::ScutilDns,
            FixedCommand::NetworkServiceOrder,
            FixedCommand::PingGateway(Ipv4Addr::new(192, 168, 1, 1)),
            FixedCommand::SayListVoices,
        ];
        for c in all {
            assert!(c.program().starts_with('/'), "{c:?}");
            assert!(!c.program().contains("sh"), "{c:?}");
        }
        assert_eq!(FixedCommand::PingGateway(Ipv4Addr::new(10, 0, 0, 1)).display(), "ping -n -c 2 -t 3 10.0.0.1");
    }
}
