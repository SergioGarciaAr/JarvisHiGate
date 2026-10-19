//! Typed progress events for analysis runs. Emitted only from real points in
//! the runtime; the UI derives its visual state from them.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Goal,
    Phase,
    Plan,
    ToolCall,
    ToolResult,
    Result,
    Error,
    Retry,
    NextStep,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Pending,
    Running,
    Ok,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub id: Uuid,
    pub run_id: String,
    pub seq: u32,
    pub kind: EventKind,
    pub status: EventStatus,
    pub timestamp: DateTime<Utc>,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: ProgressEvent);
}

/// Builds events for one run with a monotonically increasing `seq`.
#[derive(Clone)]
pub struct Reporter {
    run_id: String,
    seq: Arc<AtomicU32>,
    sink: Arc<dyn EventSink>,
}

impl Reporter {
    pub fn new(run_id: impl Into<String>, sink: Arc<dyn EventSink>) -> Self {
        Self { run_id: run_id.into(), seq: Arc::new(AtomicU32::new(0)), sink }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn emit(&self, kind: EventKind, status: EventStatus, summary: impl Into<String>, data: Option<serde_json::Value>) {
        self.sink.emit(ProgressEvent {
            id: Uuid::new_v4(),
            run_id: self.run_id.clone(),
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            kind,
            status,
            timestamp: Utc::now(),
            summary: summary.into(),
            data,
        });
    }
}

/// Sink that stores events in memory; used by tests.
#[derive(Default)]
pub struct MemorySink(pub std::sync::Mutex<Vec<ProgressEvent>>);

impl EventSink for MemorySink {
    fn emit(&self, event: ProgressEvent) {
        self.0.lock().expect("sink poisoned").push(event);
    }
}

impl MemorySink {
    pub fn kinds(&self) -> Vec<EventKind> {
        self.0.lock().expect("sink poisoned").iter().map(|e| e.kind).collect()
    }
}
