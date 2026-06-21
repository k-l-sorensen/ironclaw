use std::collections::HashMap;

use ironclaw_auth::AuthProductScope;
use ironclaw_host_api::ExtensionId;

use crate::{
    ConnectorReadError, ConnectorReadPort, LifecycleExtensionCredentialRequirement,
    LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductResponse, LifecycleProductSurfaceContext,
    ProductWorkflowError, RebornServicesError, RebornServicesErrorCode,
    RebornSetupExtensionResponse, WebUiAuthenticatedCaller, WebUiInboundValidationCode,
    WebUiInboundValidationError, WebUiSetupExtensionRequest,
};

use super::{
    ExtensionCredentialSetupService, extension_credentials::credential_scope, extension_onboarding,
    extension_setup_credentials,
};

/// Extension id whose `configure` action carries a connector API key the
/// connector port persists server-side.
const COMPOSIO_EXTENSION_NAME: &str = "composio";

/// If this is a `composio` `configure` request, persist the supplied secrets
/// through the connector port and return an `Active` projection. Returns
/// `Ok(None)` for every other extension/action so the caller falls through to
/// the lifecycle facade.
///
/// The connector port owns both the secret store and the read path, so writing
/// the key here keeps the configure write and the connector reads on a single
/// owner scope by construction.
pub(super) async fn try_configure_composio_secrets(
    connector_port: Option<&dyn ConnectorReadPort>,
    package_ref: &LifecyclePackageRef,
    request: &WebUiSetupExtensionRequest,
) -> Result<Option<RebornSetupExtensionResponse>, RebornServicesError> {
    if package_ref.id.as_str() != COMPOSIO_EXTENSION_NAME {
        return Ok(None);
    }
    if !is_configure_action(request.action.as_deref()) {
        return Ok(None);
    }
    let Some(connector_port) = connector_port else {
        // No connector subsystem in this profile: report it the same way the
        // connector routes do (503), rather than silently no-op'ing.
        return Err(RebornServicesError::service_unavailable(false));
    };

    let secrets = extract_payload_secrets(request.payload.as_ref());
    connector_port
        .configure_secrets(secrets)
        .await
        .map_err(connector_error_to_services)?;

    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, COMPOSIO_EXTENSION_NAME)
            .map_err(map_lifecycle_error)?;
    Ok(Some(RebornSetupExtensionResponse {
        package_ref,
        phase: LifecyclePhase::Active,
        blockers: Vec::new(),
        onboarding: None,
        payload: None,
        secrets: Vec::new(),
        fields: Vec::new(),
    }))
}

fn is_configure_action(action: Option<&str>) -> bool {
    action
        .map(str::trim)
        .map(|action| action.replace('-', "_").to_ascii_lowercase())
        .is_some_and(|normalized| {
            matches!(
                normalized.as_str(),
                "configure" | "config" | "extension_configure"
            )
        })
}

/// Pull `payload.secrets` (a flat `{ name: value }` object) into a string map.
/// Non-string values and a missing/non-object payload yield an empty map; the
/// connector port enforces that the required key is present.
fn extract_payload_secrets(payload: Option<&serde_json::Value>) -> HashMap<String, String> {
    payload
        .and_then(|payload| payload.get("secrets"))
        .and_then(serde_json::Value::as_object)
        .map(|secrets| {
            secrets
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn connector_error_to_services(error: ConnectorReadError) -> RebornServicesError {
    match error {
        ConnectorReadError::InvalidRequest { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        ConnectorReadError::Unavailable { retryable } => {
            RebornServicesError::service_unavailable(retryable)
        }
        ConnectorReadError::Upstream { .. } => RebornServicesError::service_unavailable(false),
        ConnectorReadError::Internal => RebornServicesError::internal_invariant(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupAction {
    View,
    Submit,
}

pub(super) async fn setup_extension(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
    request: WebUiSetupExtensionRequest,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let action = setup_action(&request)?;
    let scope = credential_scope(&caller, &package_ref);
    let extension_id = ExtensionId::new(package_ref.id.as_str())
        .map_err(|_| RebornServicesError::internal_invariant())?;
    let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
        tenant_id: caller.tenant_id,
        user_id: caller.user_id,
        agent_id: caller.agent_id,
        project_id: caller.project_id,
    });
    let lifecycle = project_package(facade, context.clone(), package_ref.clone()).await?;
    let requirements = extension_setup_credentials::requirements(&lifecycle);
    if action == SetupAction::Submit {
        extension_setup_credentials::submit_manual_tokens(
            extension_credentials,
            scope.clone(),
            &extension_id,
            &requirements,
            request,
        )
        .await?;
        let refreshed = project_package(facade, context, package_ref).await?;
        let refreshed_requirements = extension_setup_credentials::requirements(&refreshed);
        return setup_extension_response(
            extension_credentials,
            scope,
            &extension_id,
            refreshed,
            &refreshed_requirements,
        )
        .await;
    }
    setup_extension_response(
        extension_credentials,
        scope,
        &extension_id,
        lifecycle,
        &requirements,
    )
    .await
}

async fn project_package(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    package_ref: LifecyclePackageRef,
) -> Result<LifecycleProductResponse, RebornServicesError> {
    facade
        .project_package(context, package_ref)
        .await
        .map_err(map_lifecycle_error)
}

async fn setup_extension_response(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    lifecycle: LifecycleProductResponse,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let package_ref = lifecycle
        .package_ref
        .clone()
        .ok_or_else(RebornServicesError::internal_invariant)?;
    let secrets = extension_setup_credentials::project(
        extension_credentials,
        scope,
        extension_id,
        requirements,
    )
    .await?;
    let onboarding = extension_onboarding::from_lifecycle(&lifecycle).onboarding;
    Ok(RebornSetupExtensionResponse {
        package_ref,
        phase: lifecycle.phase,
        blockers: lifecycle.blockers,
        onboarding,
        payload: lifecycle.payload,
        secrets,
        fields: Vec::new(),
    })
}

fn setup_action(request: &WebUiSetupExtensionRequest) -> Result<SetupAction, RebornServicesError> {
    match request.action.as_deref() {
        None => Ok(SetupAction::View),
        Some("submit") => Ok(SetupAction::Submit),
        Some(_) => Err(validation_error(
            "action",
            WebUiInboundValidationCode::InvalidValue,
        )),
    }
}

pub(super) fn validation_error(
    field: &'static str,
    code: WebUiInboundValidationCode,
) -> RebornServicesError {
    RebornServicesError::from(WebUiInboundValidationError::new(field, code))
}

pub(super) fn map_lifecycle_error(error: ProductWorkflowError) -> RebornServicesError {
    match error {
        ProductWorkflowError::InvalidBindingRequest { .. }
        | ProductWorkflowError::UnsupportedActionKind { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        ProductWorkflowError::BindingAccessDenied => {
            RebornServicesError::from_status(RebornServicesErrorCode::Forbidden, 403, false)
        }
        ProductWorkflowError::Transient { .. } => RebornServicesError::service_unavailable(true),
        ProductWorkflowError::BindingResolutionFailed { .. }
        | ProductWorkflowError::BindingRequired { .. }
        | ProductWorkflowError::TurnSubmissionRejected { .. }
        | ProductWorkflowError::TurnSubmissionFailed { .. }
        | ProductWorkflowError::TurnResumeRejected { .. }
        | ProductWorkflowError::TurnResumeDenied { .. }
        | ProductWorkflowError::ApprovalInteractionRejected { .. }
        | ProductWorkflowError::AuthInteractionRejected { .. }
        | ProductWorkflowError::AuthContinuationRejected { .. }
        | ProductWorkflowError::BeforeInboundPolicyFailed { .. }
        | ProductWorkflowError::DuplicateAction { .. }
        | ProductWorkflowError::OutboundTargetNotDirectMessage
        | ProductWorkflowError::UnknownInstallation => RebornServicesError::internal_invariant(),
    }
}
