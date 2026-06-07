use std::collections::{HashMap, HashSet};

use ironclaw_host_api::CapabilityId;
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopDriverHost, AppendCapabilityResultRef, CapabilityCallCandidate,
        CapabilityDescriptorView, CapabilityFailureDetail, CapabilityInputIssue,
        CapabilityInputIssueCode, CapabilityInputRepair, CapabilityInvocation,
        CapabilityRecoveryDetail, CapabilityRecoveryHint, CapabilityResultMessage,
        CapabilitySurfaceVersion, LoopSafeSummary, ModelVisibleCapabilityError,
        ProviderToolCallReference, RecoveryConstraints, SameCallRetryConstraint,
        VisibleCapabilitySurface,
    },
};

use crate::{
    state::{CapabilityCallSignature, LoopExecutionState},
    strategies::{CapabilityCallSummary, CapabilityErrorSummary, CapabilityFilter, GateKind},
};

use super::{AgentLoopExecutorError, capability_host_error};

pub(super) fn capability_invocation_from_candidate(
    call: CapabilityCallCandidate,
) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
    }
}

pub(super) struct CapabilitySurfaceIndex<'a> {
    version: &'a CapabilitySurfaceVersion,
    descriptors: HashMap<&'a CapabilityId, &'a CapabilityDescriptorView>,
}

impl<'a> CapabilitySurfaceIndex<'a> {
    pub(super) fn new(surface: &'a VisibleCapabilitySurface) -> Self {
        let descriptors = surface
            .descriptors
            .iter()
            .map(|descriptor| (&descriptor.capability_id, descriptor))
            .collect();
        Self {
            version: &surface.version,
            descriptors,
        }
    }
}

pub(super) fn capability_summary(
    surface: &CapabilitySurfaceIndex<'_>,
    call: &CapabilityCallCandidate,
) -> CapabilityCallSummary {
    let concurrency_hint = surface
        .descriptors
        .get(&call.capability_id)
        .map(|descriptor| descriptor.concurrency_hint)
        .unwrap_or(ironclaw_turns::run_profile::ConcurrencyHint::Exclusive);
    CapabilityCallSummary {
        name: call.capability_id.clone(),
        concurrency_hint,
    }
}

pub(super) fn capability_is_visible(
    surface: &CapabilitySurfaceIndex<'_>,
    call: &CapabilityCallCandidate,
) -> bool {
    if &call.surface_version != surface.version {
        return false;
    }
    surface.descriptors.contains_key(&call.capability_id)
}

pub(super) fn apply_capability_filter(
    surface: &mut VisibleCapabilitySurface,
    filter: &CapabilityFilter,
) {
    surface
        .descriptors
        .retain(|descriptor| filter.permits(&descriptor.capability_id));
}

pub(super) fn push_call_signature_once(
    state: &mut LoopExecutionState,
    signatures: &mut HashSet<CapabilityCallSignature>,
    call: &CapabilityCallCandidate,
) -> Result<CapabilityCallSignature, AgentLoopExecutorError> {
    let signature = capability_call_signature(call)?;
    if signatures.insert(signature.clone()) {
        state.recent_call_signatures.push(signature.clone());
    }
    Ok(signature)
}

pub(super) fn capability_call_signature(
    call: &CapabilityCallCandidate,
) -> Result<CapabilityCallSignature, AgentLoopExecutorError> {
    let args = call
        .provider_replay
        .as_ref()
        .map(|replay| replay.arguments.clone())
        .unwrap_or_else(|| serde_json::json!({ "input_ref": call.input_ref.as_str() }));
    CapabilityCallSignature::from_call(call.capability_id.clone(), &args).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "capability call signature could not be built",
        }
    })
}

pub(super) async fn append_capability_result_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    call: &CapabilityCallCandidate,
    result: &CapabilityResultMessage,
) -> Result<(), AgentLoopExecutorError> {
    host.append_capability_result_ref(AppendCapabilityResultRef {
        result_ref: result.result_ref.clone(),
        safe_summary: result.safe_summary.clone(),
        provider_call: provider_tool_call_reference(call),
    })
    .await
    .map_err(capability_host_error)?;
    Ok(())
}

pub(super) fn provider_tool_call_reference(
    call: &CapabilityCallCandidate,
) -> Option<ProviderToolCallReference> {
    let provider_replay = call.provider_replay.as_ref()?;
    Some(ProviderToolCallReference {
        provider_id: provider_replay.provider_id.clone(),
        provider_model_id: provider_replay.provider_model_id.clone(),
        provider_turn_id: provider_replay.provider_turn_id.clone(),
        provider_call_id: provider_replay.provider_call_id.clone(),
        provider_tool_name: provider_replay.provider_tool_name.clone(),
        capability_id: call.capability_id.clone(),
        arguments: provider_replay.arguments.clone(),
        response_reasoning: provider_replay.response_reasoning.clone(),
        reasoning: provider_replay.reasoning.clone(),
        signature: provider_replay.signature.clone(),
    })
}

pub(super) async fn append_capability_error_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    summary: &CapabilityErrorSummary,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_safe_summary_ref(host, state, call, capability_error_model_summary(summary))
        .await
}

fn capability_error_model_summary(summary: &CapabilityErrorSummary) -> String {
    let Some(model_error) = model_visible_capability_error(summary) else {
        return summary.safe_summary.as_str().to_string();
    };
    render_model_visible_capability_error(&model_error)
}

fn model_visible_capability_error(
    summary: &CapabilityErrorSummary,
) -> Option<ModelVisibleCapabilityError> {
    let details = summary.detail.clone()?;
    let recovery = recovery_detail_for_model_error(&details);
    let recovery_hint = recovery_hint_for_model_error(&details);
    Some(ModelVisibleCapabilityError {
        kind: summary.kind.clone(),
        message: summary.safe_summary.as_str().to_string(),
        details,
        constraints: recovery_constraints_for_model_error(summary),
        recovery,
        recovery_hint,
    })
}

fn recovery_constraints_for_model_error(
    summary: &CapabilityErrorSummary,
) -> Option<RecoveryConstraints> {
    match summary.kind {
        ironclaw_turns::run_profile::CapabilityFailureKind::InvalidInput => {
            Some(RecoveryConstraints {
                same_call_retry: Some(SameCallRetryConstraint::RequiresChangedInput),
            })
        }
        _ => None,
    }
}

fn render_model_visible_capability_error(error: &ModelVisibleCapabilityError) -> String {
    let issues = match &error.details {
        CapabilityFailureDetail::InvalidInput { issues } => issues,
        _ => return error.message.clone(),
    };
    if issues.is_empty() {
        return error.message.clone();
    }

    let mut parts = Vec::new();
    parts.push(format!("capability failed with {}", error.kind.as_str()));
    if let Some(retry) = error
        .constraints
        .as_ref()
        .and_then(|constraints| constraints.same_call_retry.as_ref())
    {
        parts.push(format!(
            "same call retry {}",
            render_same_call_retry_constraint(retry)
        ));
    }
    let mut input_issues = format!("input issues: {}", render_input_issue(&issues[0]));
    for issue in issues.iter().skip(1).take(2) {
        input_issues.push_str("; ");
        input_issues.push_str(&render_input_issue(issue));
    }
    if issues.len() > 3 {
        input_issues.push_str("; more issues omitted");
    }
    parts.push(input_issues);
    if let Some(next_actions) = render_recovery_next_actions(error.recovery.as_ref()) {
        parts.push(next_actions);
    }
    if let Some(recovery_hint) = error.recovery_hint.as_ref() {
        parts.push(format!(
            "recovery hint: {}",
            render_recovery_hint(recovery_hint)
        ));
    }
    parts.push(format!("summary: {}", error.message));
    render_priority_safe_summary(parts)
}

fn render_same_call_retry_constraint(constraint: &SameCallRetryConstraint) -> String {
    match constraint {
        SameCallRetryConstraint::Allowed => "allowed".to_string(),
        SameCallRetryConstraint::AllowedAfterDelay { retry_after_ms } => {
            format!("allowed after {retry_after_ms} ms")
        }
        SameCallRetryConstraint::RequiresChangedInput => "requires changed input".to_string(),
        SameCallRetryConstraint::NotUseful => "not useful".to_string(),
        SameCallRetryConstraint::Forbidden => "forbidden".to_string(),
    }
}

fn render_input_issue(issue: &CapabilityInputIssue) -> String {
    let mut rendered = format!(
        "path {} {}",
        issue.path,
        capability_input_issue_message(issue.code)
    );
    if let Some(expected) = issue.expected.as_ref() {
        rendered.push_str(" expected ");
        rendered.push_str(expected);
    }
    if let Some(received) = issue.received.as_ref() {
        rendered.push_str(" received ");
        rendered.push_str(received);
    }
    if let Some(schema_path) = issue.schema_path.as_ref() {
        rendered.push_str(" at schema ");
        rendered.push_str(schema_path);
    }
    rendered
}

fn capability_input_issue_message(code: CapabilityInputIssueCode) -> &'static str {
    match code {
        CapabilityInputIssueCode::MissingRequired => "is missing a required value",
        CapabilityInputIssueCode::UnexpectedField => "has an unexpected field",
        CapabilityInputIssueCode::TypeMismatch => "has the wrong type",
        CapabilityInputIssueCode::InvalidValue => "has an invalid value",
    }
}

fn recovery_detail_for_model_error(
    details: &CapabilityFailureDetail,
) -> Option<CapabilityRecoveryDetail> {
    match details {
        CapabilityFailureDetail::InvalidInput { issues } if !issues.is_empty() => {
            Some(CapabilityRecoveryDetail::InvalidInput {
                repairs: issues.iter().take(3).map(input_repair_for_issue).collect(),
            })
        }
        CapabilityFailureDetail::InvalidInput { .. } => None,
        _ => None,
    }
}

fn input_repair_for_issue(issue: &CapabilityInputIssue) -> CapabilityInputRepair {
    match issue.code {
        CapabilityInputIssueCode::MissingRequired => CapabilityInputRepair::ProvideRequiredField {
            path: issue.path.clone(),
        },
        CapabilityInputIssueCode::UnexpectedField => CapabilityInputRepair::RemoveUnexpectedField {
            path: issue.path.clone(),
        },
        CapabilityInputIssueCode::TypeMismatch => CapabilityInputRepair::ChangeType {
            path: issue.path.clone(),
            expected: issue.expected.clone(),
        },
        CapabilityInputIssueCode::InvalidValue => CapabilityInputRepair::UseAllowedValue {
            path: issue.path.clone(),
        },
    }
}

fn render_recovery_next_actions(recovery: Option<&CapabilityRecoveryDetail>) -> Option<String> {
    let repairs = match recovery? {
        CapabilityRecoveryDetail::InvalidInput { repairs } => repairs,
        _ => return None,
    };
    if repairs.is_empty() {
        return None;
    }
    let mut rendered = format!("next actions: {}", render_input_repair(&repairs[0]));
    for repair in repairs.iter().skip(1).take(2) {
        rendered.push_str("; ");
        rendered.push_str(&render_input_repair(repair));
    }
    if repairs.len() > 3 {
        rendered.push_str("; more actions omitted");
    }
    Some(rendered)
}

fn render_input_repair(repair: &CapabilityInputRepair) -> String {
    match repair {
        CapabilityInputRepair::ProvideRequiredField { path } => {
            format!("provide missing value at {path}")
        }
        CapabilityInputRepair::RemoveUnexpectedField { path } => {
            format!("remove unexpected field at {path}")
        }
        CapabilityInputRepair::ChangeType { path, expected } => {
            if let Some(expected) = expected.as_ref() {
                format!("change value at {path} to expected type {expected}")
            } else {
                format!("change value at {path} to expected type")
            }
        }
        CapabilityInputRepair::UseAllowedValue { path } => {
            format!("change value at {path} to an allowed value")
        }
    }
}

fn recovery_hint_for_model_error(
    details: &CapabilityFailureDetail,
) -> Option<CapabilityRecoveryHint> {
    match details {
        CapabilityFailureDetail::InvalidInput { issues } if !issues.is_empty() => {
            Some(CapabilityRecoveryHint::CorrectArgumentsBeforeRetry)
        }
        CapabilityFailureDetail::InvalidInput { .. } => None,
        _ => None,
    }
}

fn render_recovery_hint(hint: &CapabilityRecoveryHint) -> &'static str {
    match hint {
        CapabilityRecoveryHint::CorrectArgumentsBeforeRetry => {
            "Correct the capability arguments and retry only if the action is still safe"
        }
    }
}

fn render_priority_safe_summary(parts: Vec<String>) -> String {
    let mut rendered = String::new();
    for part in parts {
        let candidate = if rendered.is_empty() {
            part
        } else {
            format!("{rendered}. {part}")
        };
        if LoopSafeSummary::new(candidate.clone()).is_ok() {
            rendered = candidate;
            continue;
        }
        if rendered.is_empty() {
            return truncate_loop_safe_summary(candidate);
        }
        if let Some(with_omission) = append_summary_part(&rendered, "more details omitted") {
            rendered = with_omission;
        }
        break;
    }
    if rendered.is_empty() {
        "capability failed".to_string()
    } else {
        rendered
    }
}

fn append_summary_part(current: &str, part: &str) -> Option<String> {
    let candidate = if current.is_empty() {
        part.to_string()
    } else {
        format!("{current}. {part}")
    };
    LoopSafeSummary::new(candidate.clone()).ok()?;
    Some(candidate)
}

fn truncate_loop_safe_summary(mut summary: String) -> String {
    const ELLIPSIS: &str = "...";
    if LoopSafeSummary::new(summary.clone()).is_ok() {
        return summary;
    }

    let max_len = 512_usize.saturating_sub(ELLIPSIS.len());
    while summary.len() > max_len || !summary.is_char_boundary(summary.len()) {
        summary.pop();
    }
    summary.push_str(ELLIPSIS);
    if LoopSafeSummary::new(summary.clone()).is_ok() {
        summary
    } else {
        "capability failed".to_string()
    }
}

pub(super) async fn append_capability_safe_summary_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    safe_summary: String,
) -> Result<(), AgentLoopExecutorError> {
    if call.provider_replay.is_none() {
        return Ok(());
    }
    let result_ref = synthetic_provider_error_result_ref(call)?;
    host.append_capability_result_ref(AppendCapabilityResultRef {
        result_ref: result_ref.clone(),
        safe_summary,
        provider_call: provider_tool_call_reference(call),
    })
    .await
    .map_err(capability_host_error)?;
    state.result_refs.push(result_ref);
    Ok(())
}

pub(super) fn synthetic_provider_error_result_ref(
    call: &CapabilityCallCandidate,
) -> Result<LoopResultRef, AgentLoopExecutorError> {
    let provider_replay =
        call.provider_replay
            .as_ref()
            .ok_or(AgentLoopExecutorError::PlannerContract {
                detail: "provider replay metadata is required for provider error result ref",
            })?;
    let mut suffix = format!(
        "provider-error-{}-{}",
        sanitize_result_ref_suffix(&provider_replay.provider_turn_id),
        sanitize_result_ref_suffix(&provider_replay.provider_call_id)
    );
    suffix.truncate(240);
    LoopResultRef::new(format!("result:{suffix}")).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "provider error result ref was invalid",
        }
    })
}

pub(super) fn sanitize_result_ref_suffix(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        sanitized.push_str("unknown");
    }
    sanitized
}

pub(super) fn gate_tool_result_summary(kind: GateKind, outcome: &'static str) -> String {
    let gate = match kind {
        GateKind::Approval => "approval",
        GateKind::Auth => "auth",
        GateKind::Resource => "resource",
        GateKind::AwaitDependentRun => "await_dependent_run",
    };
    format!("{gate} gate {outcome}")
}

pub(super) fn push_completed_result(
    state: &mut LoopExecutionState,
    result: CapabilityResultMessage,
) {
    state.recovery_state = state.recovery_state.cleared_attempts();
    state.result_refs.push(result.result_ref);
}
