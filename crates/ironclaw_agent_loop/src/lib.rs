//! Agent-loop framework state and strategy contracts for IronClaw Reborn.
//!
//! This crate owns the framework layer above `ironclaw_turns`. The master
//! architecture is `docs/reborn/agent-loop-skeleton.md`; workstream briefs live
//! under `docs/reborn/agent-loop-briefs/`.

pub mod state;
pub mod strategies;

pub use strategies::{
    BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary, CapabilityErrorClass,
    CapabilityErrorSummary, CapabilityFilter, CapabilityStrategy, ConcurrencyHint, ContextStrategy,
    GateHandlingStrategy, GateKind, GateOutcome, GateSummary, ModelErrorClass, ModelErrorSummary,
    ModelPreference, ModelStrategy, RecoveryOutcome, RecoveryStrategy, RetryAlteration,
};
