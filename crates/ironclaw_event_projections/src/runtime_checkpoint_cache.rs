use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use ironclaw_events::EventCursor;

use crate::ProjectionScope;
use crate::runtime_projection::RuntimeProjectionState;

const RUNTIME_PROJECTION_CHECKPOINTS_PER_SCOPE: usize = 256;

#[derive(Clone)]
pub(crate) struct RuntimeProjectionCheckpoint {
    pub(crate) cursor: EventCursor,
    pub(crate) state: RuntimeProjectionState,
}

type RuntimeProjectionCheckpointMap =
    HashMap<ProjectionScope, BTreeMap<EventCursor, RuntimeProjectionState>>;

#[derive(Clone, Default)]
pub(crate) struct RuntimeProjectionCheckpointCache {
    checkpoints: Arc<Mutex<RuntimeProjectionCheckpointMap>>,
}

impl RuntimeProjectionCheckpointCache {
    pub(crate) fn latest(&self, scope: &ProjectionScope) -> RuntimeProjectionCheckpoint {
        let checkpoints = self.lock();
        checkpoints
            .get(scope)
            .and_then(|scope_checkpoints| {
                scope_checkpoints
                    .last_key_value()
                    .map(|(cursor, state)| checkpoint(*cursor, state.clone()))
            })
            .unwrap_or_else(origin_runtime_checkpoint)
    }

    pub(crate) fn at_or_before(
        &self,
        scope: &ProjectionScope,
        cursor: EventCursor,
    ) -> RuntimeProjectionCheckpoint {
        let checkpoints = self.lock();
        checkpoints
            .get(scope)
            .and_then(|scope_checkpoints| {
                scope_checkpoints
                    .range(..=cursor)
                    .next_back()
                    .map(|(cursor, state)| checkpoint(*cursor, state.clone()))
            })
            .unwrap_or_else(origin_runtime_checkpoint)
    }

    pub(crate) fn store(&self, scope: &ProjectionScope, checkpoint: &RuntimeProjectionCheckpoint) {
        if checkpoint.cursor == EventCursor::origin() {
            return;
        }
        let mut checkpoints = self.lock();
        let scope_checkpoints = checkpoints.entry(scope.clone()).or_default();
        scope_checkpoints.insert(checkpoint.cursor, checkpoint.state.clone());
        while scope_checkpoints.len() > RUNTIME_PROJECTION_CHECKPOINTS_PER_SCOPE {
            let Some(first) = scope_checkpoints.keys().next().copied() else {
                break;
            };
            scope_checkpoints.remove(&first);
        }
    }

    fn lock(&self) -> MutexGuard<'_, RuntimeProjectionCheckpointMap> {
        match self.checkpoints.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(crate) fn after_for_checkpoint(cursor: EventCursor) -> Option<EventCursor> {
    if cursor == EventCursor::origin() {
        None
    } else {
        Some(cursor)
    }
}

fn origin_runtime_checkpoint() -> RuntimeProjectionCheckpoint {
    checkpoint(
        EventCursor::origin(),
        RuntimeProjectionState::without_capability_activity_output_limit(),
    )
}

fn checkpoint(cursor: EventCursor, state: RuntimeProjectionState) -> RuntimeProjectionCheckpoint {
    RuntimeProjectionCheckpoint { cursor, state }
}
