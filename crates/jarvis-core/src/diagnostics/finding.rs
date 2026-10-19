//! Structured diagnostic results.

use serde::Serialize;

/// Distinguishes a confirmed problem from a warning or a plain observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Problem,
    Warning,
    Observation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// How directly the evidence supports the finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Direct reading from the kernel or a definitive probe result.
    High,
    /// Threshold-based interpretation of real data.
    Medium,
    /// Inference with plausible alternative explanations.
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evidence {
    pub label: String,
    pub value: String,
    /// Where the value came from (command or API).
    pub source: String,
}

impl Evidence {
    pub fn new(label: impl Into<String>, value: impl Into<String>, source: impl Into<String>) -> Self {
        Self { label: label.into(), value: value.into(), source: source.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub id: &'static str,
    pub kind: FindingKind,
    pub severity: Severity,
    pub title: String,
    pub evidence: Vec<Evidence>,
    pub confidence: Confidence,
    pub recommendation: String,
    /// Descriptive only; diagnostics never execute it.
    pub proposed_action: Option<String>,
}

/// A piece of data that could not be obtained, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataGap {
    pub what: String,
    pub source: String,
    pub reason: String,
}

impl DataGap {
    pub fn new(what: impl Into<String>, source: impl Into<String>, reason: impl Into<String>) -> Self {
        Self { what: what.into(), source: source.into(), reason: reason.into() }
    }
}

/// Problems first, then warnings, then observations; higher severity first.
pub fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| a.kind.cmp(&b.kind).then(b.severity.cmp(&a.severity)));
}

pub fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1024f64.powi(3))
}

pub fn pct(part: u64, total: u64) -> String {
    if total == 0 { "—".into() } else { format!("{:.0} %", part as f64 * 100.0 / total as f64) }
}
