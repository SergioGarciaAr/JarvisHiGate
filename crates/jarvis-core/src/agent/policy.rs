//! Permission policy. This version is read-only: any tool that could
//! affect the system is denied, regardless of who asks for it.

use serde::Serialize;
use thiserror::Error;

use super::tools::{RiskLevel, ToolSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    #[error("Operación denegada: `{tool}` tiene riesgo {risk:?} y Jarvis funciona en modo de solo lectura.")]
    Denied { tool: &'static str, risk: RiskLevel },
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    mode: PolicyMode,
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self { mode: PolicyMode::ReadOnly }
    }
}

impl PolicyEngine {
    pub fn mode(&self) -> PolicyMode {
        self.mode
    }

    pub fn authorize(&self, spec: &ToolSpec) -> Result<(), PolicyError> {
        match (self.mode, spec.risk) {
            (PolicyMode::ReadOnly, RiskLevel::ReadOnly) => Ok(()),
            (PolicyMode::ReadOnly, risk) => Err(PolicyError::Denied { tool: spec.name, risk }),
        }
    }
}
