//! Terminal-independent progress events and cooperative cancellation.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};

/// The quantity measured by a phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressUnit {
    Bytes,
    Entries,
    Operations,
}

/// Independently measured stages of a media operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    OpenImage,
    ScanSource,
    HashInputs,
    ApplyOverlay,
    PlanImage,
    EmitImage,
    CopyDevice,
    Flush,
    Verify,
    BootTest,
}

/// Lifecycle of a single measured phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState {
    Started,
    Advanced,
    Finished,
    Failed,
    Cancelled,
}

/// A measured observation, never evidence of successful publication by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub operation_id: u64,
    pub phase: Phase,
    pub completed: u64,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
    pub entry_id: Option<String>,
    pub state: ProgressState,
}

/// A progress sink cannot override operation results or request cancellation.
pub trait Observer {
    fn observe(&mut self, event: &ProgressEvent);
}

impl<F: FnMut(&ProgressEvent)> Observer for F {
    fn observe(&mut self, event: &ProgressEvent) {
        self(event);
    }
}

/// A sink for callers that do not need progress.
#[derive(Debug, Default)]
pub struct NoProgress;
impl Observer for NoProgress {
    fn observe(&mut self, _: &ProgressEvent) {}
}

/// Shared explicit cancellation, checked separately from observers.
#[derive(Debug, Default, Clone)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    /// Request cancellation at the next safe checkpoint.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    /// Check whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    /// Return an error at a cooperative checkpoint after cancellation.
    pub fn checkpoint(&self) -> Result<(), ProgressError> {
        if self.is_cancelled() {
            Err(ProgressError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Invalid progress accounting or explicit cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProgressError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("progress counter overflow")]
    Overflow,
    #[error("progress exceeds its declared total")]
    ExceedsTotal,
    #[error("progress phase is already terminal")]
    Terminal,
    #[error("progress phase has incomplete work")]
    Incomplete,
}

/// Checked accounting for one phase. Count actual work, never seek positions.
pub struct PhaseProgress<'a> {
    event: ProgressEvent,
    observer: &'a mut dyn Observer,
}
impl<'a> PhaseProgress<'a> {
    /// Start a phase with zero completed work and an optional known total.
    pub fn start(
        observer: &'a mut dyn Observer,
        operation_id: u64,
        phase: Phase,
        total: Option<u64>,
        unit: ProgressUnit,
        entry_id: Option<String>,
    ) -> Self {
        let event = ProgressEvent {
            operation_id,
            phase,
            completed: 0,
            total,
            unit,
            entry_id,
            state: ProgressState::Started,
        };
        observer.observe(&event);
        Self { event, observer }
    }
    /// Add actual completed work, preserving monotonicity and total bounds.
    pub fn advance(&mut self, amount: u64) -> Result<(), ProgressError> {
        self.ensure_active()?;
        let completed = self
            .event
            .completed
            .checked_add(amount)
            .ok_or(ProgressError::Overflow)?;
        if self.event.total.is_some_and(|total| completed > total) {
            return Err(ProgressError::ExceedsTotal);
        }
        self.event.completed = completed;
        self.event.state = ProgressState::Advanced;
        self.observer.observe(&self.event);
        Ok(())
    }
    /// Finish only after all declared work and required synchronization succeed.
    pub fn finish(&mut self) -> Result<(), ProgressError> {
        self.ensure_active()?;
        if self
            .event
            .total
            .is_some_and(|total| self.event.completed != total)
        {
            return Err(ProgressError::Incomplete);
        }
        self.terminate(ProgressState::Finished)
    }
    /// Mark a failed phase without inventing completed work.
    pub fn fail(&mut self) -> Result<(), ProgressError> {
        self.terminate(ProgressState::Failed)
    }
    /// Mark a cancelled phase without inventing completed work.
    pub fn cancel(&mut self) -> Result<(), ProgressError> {
        self.terminate(ProgressState::Cancelled)
    }
    fn terminate(&mut self, state: ProgressState) -> Result<(), ProgressError> {
        self.ensure_active()?;
        self.event.state = state;
        self.observer.observe(&self.event);
        Ok(())
    }
    fn ensure_active(&self) -> Result<(), ProgressError> {
        match self.event.state {
            ProgressState::Started | ProgressState::Advanced => Ok(()),
            _ => Err(ProgressError::Terminal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_phase_records_actual_work_and_terminal_total() {
        let mut events = Vec::new();
        let mut observer = |event: &ProgressEvent| events.push(event.clone());
        let mut phase = PhaseProgress::start(
            &mut observer,
            1,
            Phase::EmitImage,
            Some(3),
            ProgressUnit::Bytes,
            None,
        );
        phase.advance(1).unwrap();
        phase.advance(2).unwrap();
        phase.finish().unwrap();
        assert_eq!(phase.advance(1), Err(ProgressError::Terminal));
        assert_eq!(
            events
                .iter()
                .map(|e| (e.completed, e.state))
                .collect::<Vec<_>>(),
            vec![
                (0, ProgressState::Started),
                (1, ProgressState::Advanced),
                (3, ProgressState::Advanced),
                (3, ProgressState::Finished)
            ]
        );
    }
    #[test]
    fn failed_accounting_does_not_emit_false_completion() {
        let mut events = Vec::new();
        let mut observer = |event: &ProgressEvent| events.push(event.clone());
        let mut phase = PhaseProgress::start(
            &mut observer,
            1,
            Phase::Verify,
            Some(2),
            ProgressUnit::Bytes,
            None,
        );
        phase.advance(1).unwrap();
        assert_eq!(phase.advance(2), Err(ProgressError::ExceedsTotal));
        assert_eq!(phase.finish(), Err(ProgressError::Incomplete));
        phase.fail().unwrap();
        assert_eq!(events.last().unwrap().completed, 1);
        assert_eq!(events.last().unwrap().state, ProgressState::Failed);
    }
    #[test]
    fn unknown_total_detects_overflow_and_can_cancel() {
        let mut observer = NoProgress;
        let mut phase = PhaseProgress::start(
            &mut observer,
            1,
            Phase::ScanSource,
            None,
            ProgressUnit::Entries,
            None,
        );
        phase.advance(u64::MAX).unwrap();
        assert_eq!(phase.advance(1), Err(ProgressError::Overflow));
        phase.cancel().unwrap();
        assert_eq!(phase.finish(), Err(ProgressError::Terminal));
    }
    #[test]
    fn cancellation_is_shared_and_independent_of_progress() {
        let token = CancellationToken::default();
        assert_eq!(token.checkpoint(), Ok(()));
        token.clone().cancel();
        assert_eq!(token.checkpoint(), Err(ProgressError::Cancelled));
    }
    #[test]
    fn empty_phase_finishes_at_zero() {
        let mut observer = NoProgress;
        PhaseProgress::start(
            &mut observer,
            1,
            Phase::Flush,
            Some(0),
            ProgressUnit::Operations,
            None,
        )
        .finish()
        .unwrap();
    }
}
