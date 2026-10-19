//! Execution limits and cancellation for agent runs.

use std::future::Future;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_steps: u32,
    pub step_timeout: Duration,
    pub total_timeout: Duration,
    pub max_retries: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 8,
            step_timeout: Duration::from_secs(180),
            total_timeout: Duration::from_secs(300),
            max_retries: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LimitError {
    #[error("Se alcanzó el límite de {0} pasos.")]
    StepLimit(u32),
    #[error("El paso superó el tiempo máximo.")]
    StepTimeout,
    #[error("La ejecución superó el tiempo total máximo.")]
    TotalTimeout,
    #[error("Ejecución cancelada.")]
    Cancelled,
}

/// Tracks steps and deadlines for one run.
pub struct Budget {
    limits: Limits,
    started: Instant,
    steps: u32,
    cancel: CancellationToken,
}

impl Budget {
    pub fn new(limits: Limits, cancel: CancellationToken) -> Self {
        Self { limits, started: Instant::now(), steps: 0, cancel }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Runs one step under the step limit, step/total timeouts and cancellation.
    pub async fn step<T>(&mut self, fut: impl Future<Output = T>) -> Result<T, LimitError> {
        if self.cancel.is_cancelled() {
            return Err(LimitError::Cancelled);
        }
        if self.steps >= self.limits.max_steps {
            return Err(LimitError::StepLimit(self.limits.max_steps));
        }
        self.steps += 1;
        let remaining = self
            .limits
            .total_timeout
            .checked_sub(self.started.elapsed())
            .ok_or(LimitError::TotalTimeout)?;
        let (timeout, err) = if remaining < self.limits.step_timeout {
            (remaining, LimitError::TotalTimeout)
        } else {
            (self.limits.step_timeout, LimitError::StepTimeout)
        };
        tokio::select! {
            _ = self.cancel.cancelled() => Err(LimitError::Cancelled),
            r = tokio::time::timeout(timeout, fut) => r.map_err(|_| err),
        }
    }
}
