#![allow(dead_code)]

use ironclaw_turns::{
    TurnEventProjectionCursor, TurnEventProjectionRequest, TurnEventProjectionService,
    TurnEventProjectionSnapshot,
};

use crate::reborn_support::harness::{HarnessResult, RebornBinaryE2EHarness, SubmittedTurn};

pub async fn turn_event_snapshot(
    harness: &RebornBinaryE2EHarness,
    submitted: &SubmittedTurn,
) -> HarnessResult<TurnEventProjectionSnapshot> {
    Ok(TurnEventProjectionService::new(harness.turn_store())
        .snapshot(TurnEventProjectionRequest {
            scope: submitted.scope.clone(),
            owner_user_id: owner_user_id(submitted),
            after: None,
            limit: 100,
        })
        .await?)
}

pub async fn turn_event_updates(
    harness: &RebornBinaryE2EHarness,
    submitted: &SubmittedTurn,
    after: Option<TurnEventProjectionCursor>,
    limit: usize,
) -> HarnessResult<TurnEventProjectionSnapshot> {
    Ok(TurnEventProjectionService::new(harness.turn_store())
        .updates(TurnEventProjectionRequest {
            scope: submitted.scope.clone(),
            owner_user_id: owner_user_id(submitted),
            after,
            limit,
        })
        .await?)
}

fn owner_user_id(submitted: &SubmittedTurn) -> Option<ironclaw_host_api::UserId> {
    submitted
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| Some(submitted.actor.user_id.clone()))
}
