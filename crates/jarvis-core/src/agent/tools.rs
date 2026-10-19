//! Explicit tool registry. The agent can only call tools registered here,
//! and every call passes through the [`PolicyEngine`].

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::Serialize;
use thiserror::Error;

use super::policy::{PolicyEngine, PolicyError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    ReadOnly,
    SystemAffecting,
    Destructive,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolError {
    #[error("La herramienta `{0}` no está registrada.")]
    NotRegistered(String),
    #[error(transparent)]
    Denied(#[from] PolicyError),
    #[error("Argumentos inválidos para `{tool}`: {reason}")]
    InvalidArgs { tool: &'static str, reason: String },
    #[error("{message}")]
    Failed { message: String, retryable: bool },
    #[error("Operación cancelada.")]
    Cancelled,
}

impl ToolError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, ToolError::Failed { retryable: true, .. })
    }
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn invoke(&self, args: serde_json::Value) -> BoxFuture<'_, Result<serde_json::Value, ToolError>>;
}

pub struct ToolRegistry {
    tools: BTreeMap<&'static str, Arc<dyn Tool>>,
    policy: PolicyEngine,
}

impl ToolRegistry {
    pub fn new(policy: PolicyEngine) -> Self {
        Self { tools: BTreeMap::new(), policy }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.spec().name, tool);
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    pub async fn invoke(&self, name: &str, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let tool = self.tools.get(name).ok_or_else(|| ToolError::NotRegistered(name.to_string()))?;
        self.policy.authorize(&tool.spec())?;
        tool.invoke(args).await
    }
}
