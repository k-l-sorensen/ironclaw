#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelStrategyState {
    pub fallback_index: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryStrategyState {
    pub attempts: u32,
}

impl RecoveryStrategyState {
    /// Returns a new slot value with `attempts` incremented by one
    /// (saturating at `u32::MAX`).
    ///
    /// Used by `DefaultRecoveryStrategy` when classifying a fresh error so
    /// the next retry/abort decision sees the updated attempt count. See
    /// `docs/reborn/agent-loop-skeleton.md` §6 ("RecoveryStrategy") and §10
    /// ("Production-safe escape" — per-error retry budget).
    pub fn with_incremented_attempts(&self) -> Self {
        Self {
            attempts: self.attempts.saturating_add(1),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ControlStrategyState {
    pub turns_completed: u32,
    pub terminate_hints_in_last_batch: u32,
    pub last_batch_total: u32,
}
