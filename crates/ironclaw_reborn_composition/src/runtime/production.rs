use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
};
use ironclaw_product_workflow::{
    ApprovalInteractionService, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    ProductWorkflowError, ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
};
use ironclaw_turns::run_profile::{LoopRunContext, PromptMode};

#[derive(Default)]
pub(super) struct EmptyIdentityContextSource;

#[async_trait::async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

pub(super) struct UnavailableApprovalInteractionService;

#[async_trait::async_trait]
impl ApprovalInteractionService for UnavailableApprovalInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        Err(ProductWorkflowError::BeforeInboundPolicyFailed {
            reason: "approval interaction service is not wired for production runtime launch"
                .to_string(),
            permanent: true,
        })
    }

    async fn resolve(
        &self,
        _request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        Err(ProductWorkflowError::BeforeInboundPolicyFailed {
            reason: "approval interaction service is not wired for production runtime launch"
                .to_string(),
            permanent: true,
        })
    }
}
