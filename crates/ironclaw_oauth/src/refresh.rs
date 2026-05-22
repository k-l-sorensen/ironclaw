use std::collections::HashMap;
use std::sync::Arc;

use ironclaw_host_api::ResourceScope;
use tokio::sync::Mutex;

use crate::{OAuthError, OAuthFlow, TokenSet};

#[derive(Clone)]
pub struct RefreshScheduler {
    flow: OAuthFlow,
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl RefreshScheduler {
    pub fn new(flow: OAuthFlow) -> Self {
        Self {
            flow,
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn refresh(
        &self,
        provider_id: &str,
        scope: &ResourceScope,
    ) -> Result<TokenSet, OAuthError> {
        let credential_name = self.flow.credential_name(provider_id)?;
        let lock = {
            let mut locks = self.locks.lock().await;
            locks
                .entry(credential_name)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        self.flow.refresh(provider_id, scope).await
    }
}
