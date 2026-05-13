//! Strategy trait contracts for the Reborn agent-loop framework.
//!
//! Each strategy receives `&LoopExecutionState` and returns an outcome enum
//! that carries the new value of its own slot. The executor swaps the slot
//! into the next whole state. See `docs/reborn/agent-loop-skeleton.md` §6
//! ("Strategy decomposition") and §8 ("Outcome enums").
//!
//! WS-1 lands the context / capability / model axis (α).
//! WS-2 lands the batch / gate / recovery axis (β).
//! WS-3 lands the stop / drain / budget axis (γ).
//! WS-5 lands the `Default*` impls for all nine strategies.
//! The executor body that consumes these outcomes lands in WS-6.

pub mod batch;
pub mod budget;
mod capability;
mod context;
pub mod drain;
pub mod gate;
mod model;
pub mod recovery;
pub mod stop;

pub use batch::{
    BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary, ConcurrencyHint,
    DefaultBatchPolicyStrategy,
};
pub use budget::{BudgetStrategy, DefaultBudgetStrategy, UnlimitedBudget};
pub use capability::{CapabilityFilter, CapabilityStrategy, DefaultCapabilityStrategy};
pub use context::{ContextStrategy, DefaultContextStrategy};
pub use drain::{DefaultInputDrainStrategy, InputDrainStrategy};
pub use gate::{
    DefaultGateHandlingStrategy, GateHandlingStrategy, GateKind, GateOutcome, GateSummary,
};
pub use model::{DefaultModelStrategy, ModelPreference, ModelStrategy};
pub use recovery::{
    CapabilityErrorClass, CapabilityErrorSummary, DefaultRecoveryStrategy, ModelErrorClass,
    ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration,
};
pub use stop::{
    DefaultStopConditionStrategy, StopConditionStrategy, StopKind, StopOutcome, TurnEndKind,
    TurnSummary,
};
