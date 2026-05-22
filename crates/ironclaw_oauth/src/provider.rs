use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::{OAuthError, TokenSet};

#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn auth_url(&self) -> &str;
    fn token_url(&self) -> &str;
    fn credential_name(&self) -> &str;
    fn public_client_id(&self) -> &str;
    fn direct_client_secret(&self) -> Option<&SecretString>;
    fn build_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        scopes: &[String],
        redirect_uri: &str,
    ) -> String;
    fn parse_token_response(&self, body: &serde_json::Value) -> Result<TokenSet, OAuthError>;
    fn detect_scope_mismatch(&self, stored: &[String], required: &[String]) -> Vec<String>;
}

#[derive(Default, Clone)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn OAuthProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, provider: Arc<dyn OAuthProvider>) -> Result<(), OAuthError> {
        let id = provider.provider_id().to_string();
        if self.providers.contains_key(&id) {
            return Err(OAuthError::DuplicateProvider { provider: id });
        }
        self.providers.insert(id, provider);
        Ok(())
    }

    pub fn get(&self, provider_id: &str) -> Result<Arc<dyn OAuthProvider>, OAuthError> {
        self.providers
            .get(provider_id)
            .cloned()
            .ok_or_else(|| OAuthError::UnknownProvider {
                provider: provider_id.to_string(),
            })
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }
}
