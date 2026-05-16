//! Product-live adapter bundle for planned AgentLoop composition.
//!
//! This module does not cut app or gateway traffic over to Reborn. It provides
//! the explicit adapter bundle the eventual app/gateway entrypoint can pass
//! into `ironclaw_reborn::runtime::build_product_live_planned_runtime` once
//! durable thread/checkpoint stores are selected by that caller.

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::VisibleCapabilityRequest;
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    HostIdentityContextSource, HostInputQueue, HostRuntimeLoopCapabilityPortFactory,
    LoopCapabilityInputResolver, LoopCapabilityResultWriter, RunCancellationFactory,
};
use ironclaw_reborn::{
    loop_driver_host::LoopCapabilityPortFactory,
    model_routes::{
        ModelRoute, ModelRouteError, ModelRoutePolicy, ModelRouteResolver, ModelSelectionMode,
        ModelSlot, StaticModelRouteResolver,
    },
};
use ironclaw_turns::run_profile::{
    InstructionSafetyContext, LoopHostMilestoneSink, LoopModelBudgetAccountant,
    LoopModelPolicyGuard, LoopRunContext,
};

use crate::RebornServices;

#[derive(Debug, Error)]
pub enum ProductLivePlannedRuntimeAdapterError {
    #[error("product-live planned runtime adapters require a host runtime facade")]
    MissingHostRuntime,
    #[error("product-live model route is invalid: {0}")]
    ModelRoute(#[from] ModelRouteError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProductLiveModelRoute {
    provider_id: String,
    model_id: String,
}

impl ProductLiveModelRoute {
    fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        let route = ModelRoute::new(provider_id, model_id)?;
        Ok(Self {
            provider_id: route.provider_id().to_string(),
            model_id: route.model_id().to_string(),
        })
    }

    fn to_model_route(&self) -> Result<ModelRoute, ModelRouteError> {
        ModelRoute::new(self.provider_id.clone(), self.model_id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductLiveModelRouteSettings {
    selection_mode: ModelSelectionMode,
    default_route: ProductLiveModelRoute,
    mission_route: Option<ProductLiveModelRoute>,
}

impl ProductLiveModelRouteSettings {
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        Ok(Self {
            selection_mode: ModelSelectionMode::ManagedOnly,
            default_route: ProductLiveModelRoute::new(provider_id, model_id)?,
            mission_route: None,
        })
    }

    pub fn with_selection_mode(mut self, selection_mode: ModelSelectionMode) -> Self {
        self.selection_mode = selection_mode;
        self
    }

    pub fn with_mission_route(
        mut self,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        self.mission_route = Some(ProductLiveModelRoute::new(provider_id, model_id)?);
        Ok(self)
    }

    fn into_resolver(self) -> Result<StaticModelRouteResolver, ModelRouteError> {
        let default_route = self.default_route.to_model_route()?;
        let mission_route = self
            .mission_route
            .map(|route| route.to_model_route())
            .transpose()?;

        let mut policy =
            ModelRoutePolicy::new(self.selection_mode).with_approved_route(default_route.clone());
        if let Some(route) = mission_route.clone() {
            policy = policy.with_approved_route(route);
        }

        let mut resolver =
            StaticModelRouteResolver::new(policy).with_route(ModelSlot::Default, default_route);
        if let Some(route) = mission_route {
            resolver = resolver.with_route(ModelSlot::Mission, route);
        }
        Ok(resolver)
    }
}

pub struct ProductLivePlannedRuntimeAdapterConfig {
    pub visible_capability_request: VisibleCapabilityRequest,
    pub capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub capability_allow_set: CapabilityAllowSet,
    pub model_routes: ProductLiveModelRouteSettings,
    pub cancellation_factory: Arc<dyn RunCancellationFactory>,
    pub input_queue: Arc<dyn HostInputQueue>,
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    pub model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    pub model_budget_accountant: Arc<dyn LoopModelBudgetAccountant>,
    pub safety_context: InstructionSafetyContext,
    pub milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
}

#[derive(Clone)]
pub struct ProductLivePlannedRuntimeAdapters {
    pub capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    pub model_route_resolver: Arc<dyn ModelRouteResolver>,
    pub cancellation_factory: Arc<dyn RunCancellationFactory>,
    pub input_queue: Arc<dyn HostInputQueue>,
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    pub model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    pub model_budget_accountant: Arc<dyn LoopModelBudgetAccountant>,
    pub safety_context: InstructionSafetyContext,
}

impl ProductLivePlannedRuntimeAdapters {
    pub fn from_services(
        services: &RebornServices,
        config: ProductLivePlannedRuntimeAdapterConfig,
    ) -> Result<Self, ProductLivePlannedRuntimeAdapterError> {
        let host_runtime = services
            .host_runtime
            .clone()
            .ok_or(ProductLivePlannedRuntimeAdapterError::MissingHostRuntime)?;

        let capability_factory = HostRuntimeLoopCapabilityPortFactory::new(
            host_runtime,
            config.visible_capability_request,
            config.capability_input_resolver,
            config.capability_result_writer,
            config.milestone_sink,
        );
        let model_route_resolver: Arc<dyn ModelRouteResolver> =
            Arc::new(config.model_routes.into_resolver()?);

        Ok(Self {
            capability_factory: Arc::new(capability_factory),
            capability_surface_resolver: Arc::new(StaticCapabilitySurfaceResolver::new(
                config.capability_allow_set,
            )),
            model_route_resolver,
            cancellation_factory: config.cancellation_factory,
            input_queue: config.input_queue,
            identity_context_source: config.identity_context_source,
            model_policy_guard: config.model_policy_guard,
            model_budget_accountant: config.model_budget_accountant,
            safety_context: config.safety_context,
        })
    }
}

struct StaticCapabilitySurfaceResolver {
    allow_set: CapabilityAllowSet,
}

impl StaticCapabilitySurfaceResolver {
    fn new(allow_set: CapabilityAllowSet) -> Self {
        Self { allow_set }
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for StaticCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(self.allow_set.clone())
    }
}

pub fn capability_allowlist(ids: impl IntoIterator<Item = CapabilityId>) -> CapabilityAllowSet {
    CapabilityAllowSet::allowlist(ids)
}
