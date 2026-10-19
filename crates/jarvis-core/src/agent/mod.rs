//! Read-only agent: explicit tool registry, permission policy and limits.
//! There is deliberately no shell tool.

mod limits;
mod policy;
mod runtime;
mod tools;

pub use limits::{Budget, LimitError, Limits};
pub use policy::{PolicyEngine, PolicyError, PolicyMode};
pub use runtime::{LLM_ASK, LlmAskTool, RunOutcome, ThinkMode, ThinkRequest, ThinkResult, parse_result, run_think};
pub use tools::{RiskLevel, Tool, ToolError, ToolRegistry, ToolSpec};
