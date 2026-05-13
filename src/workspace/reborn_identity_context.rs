use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostIdentityMessageContent, IdentityApplicability, IdentityFileName, identity_message_ref,
};
use ironclaw_memory::DEFAULT_PROMPT_PROTECTED_PATHS;
use ironclaw_turns::{LoopMessageRef, run_profile::LoopRunContext, run_profile::PromptMode};

use crate::{error::WorkspaceError, workspace::paths};

use super::Workspace;

const STABLE_IDENTITY_PATHS: &[&str] = &[
    paths::SOUL,
    paths::AGENTS,
    paths::USER,
    paths::IDENTITY,
    paths::TOOLS,
    paths::BOOTSTRAP,
    paths::ASSISTANT_DIRECTIVES,
];

#[derive(Clone)]
pub struct WorkspaceIdentityContextSource {
    workspace: Arc<Workspace>,
}

impl WorkspaceIdentityContextSource {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }

    pub fn stable_identity_paths() -> Vec<&'static str> {
        DEFAULT_PROMPT_PROTECTED_PATHS
            .iter()
            .copied()
            .filter(|path| STABLE_IDENTITY_PATHS.contains(path))
            .collect()
    }

    async fn read_identity_content(&self, path: &str) -> Result<Option<String>, WorkspaceError> {
        match self.workspace.read_primary(path).await {
            Ok(document) if document.content.is_empty() => Ok(None),
            Ok(document) => Ok(Some(document.content)),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn candidate_for_path(
        &self,
        path: &'static str,
    ) -> Result<Option<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let Some(content) = self
            .read_identity_content(path)
            .await
            .map_err(|_| HostIdentityContextBuildError::SourceUnavailable)?
        else {
            return Ok(None);
        };
        let name = IdentityFileName::new(path)?;
        let message_ref = identity_message_ref(&name, &content)
            .map_err(|_| HostIdentityContextBuildError::Internal)?;
        Ok(Some(HostIdentityContextCandidate::new_trusted(
            name,
            message_ref,
            format!("identity file {path} available"),
            applicability_for_path(path),
        )))
    }
}

#[async_trait]
impl HostIdentityContextSource for WorkspaceIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let mut candidates = Vec::new();
        for path in Self::stable_identity_paths() {
            if let Some(candidate) = self.candidate_for_path(path).await? {
                candidates.push(candidate);
            }
        }
        Ok(candidates)
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        for path in Self::stable_identity_paths() {
            let Some(content) = self
                .read_identity_content(path)
                .await
                .map_err(|_| HostIdentityContextBuildError::SourceUnavailable)?
            else {
                continue;
            };
            let name = IdentityFileName::new(path)?;
            let expected = identity_message_ref(&name, &content)
                .map_err(|_| HostIdentityContextBuildError::Internal)?;
            if &expected == message_ref {
                return Ok(Some(HostIdentityMessageContent { name, content }));
            }
        }
        Ok(None)
    }
}

fn applicability_for_path(path: &str) -> IdentityApplicability {
    if path == paths::TOOLS {
        IdentityApplicability::OnCodeAct
    } else {
        IdentityApplicability::Always
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_identity_context_uses_protected_path_canon() {
        let stable = WorkspaceIdentityContextSource::stable_identity_paths();
        assert_eq!(
            stable,
            vec![
                paths::SOUL,
                paths::AGENTS,
                paths::USER,
                paths::IDENTITY,
                paths::TOOLS,
                paths::BOOTSTRAP,
                paths::ASSISTANT_DIRECTIVES,
            ]
        );
        assert!(
            stable
                .iter()
                .all(|path| DEFAULT_PROMPT_PROTECTED_PATHS.contains(path))
        );
        assert!(!stable.contains(&paths::HEARTBEAT));
        assert!(!stable.contains(&paths::MEMORY));
        assert!(!stable.contains(&paths::PROFILE));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_identity_context_reads_primary_scope_only() {
        use ironclaw_host_api::{TenantId, ThreadId};
        use ironclaw_turns::{
            RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
            run_profile::InMemoryRunProfileResolver,
        };

        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("test.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("create db");
        crate::db::Database::run_migrations(&backend)
            .await
            .expect("run migrations");
        let db: Arc<dyn crate::db::Database> = Arc::new(backend);

        Workspace::new_with_db("secondary", db.clone())
            .write(paths::AGENTS, "secondary instructions")
            .await
            .unwrap();
        Workspace::new_with_db("primary", db.clone())
            .write(paths::AGENTS, "primary instructions")
            .await
            .unwrap();
        let workspace = Arc::new(
            Workspace::new_with_db("primary", db)
                .with_additional_read_scopes(vec!["secondary".to_string()]),
        );
        let source = WorkspaceIdentityContextSource::new(workspace);
        let context = run_context().await;
        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name.as_str(), paths::AGENTS);
        let content = source
            .resolve_identity_message_content(
                &context,
                candidates[0].message_ref.as_ref().expect("trusted ref"),
            )
            .await
            .unwrap()
            .expect("identity content");
        assert_eq!(content.content, "primary instructions");

        async fn run_context() -> LoopRunContext {
            let resolved_run_profile = InMemoryRunProfileResolver::default()
                .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
                .await
                .unwrap();
            let scope = TurnScope::new(
                TenantId::new("tenant-workspace-identity").unwrap(),
                None,
                None,
                ThreadId::new("thread-workspace-identity").unwrap(),
            );
            LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
        }
    }
}
