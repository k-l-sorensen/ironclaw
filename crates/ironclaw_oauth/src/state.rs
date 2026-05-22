use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use ironclaw_host_api::ResourceScope;
use rand::Rng;
use rand::distributions::Alphanumeric;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::OAuthError;

pub const OAUTH_STATE_TTL: Duration = Duration::minutes(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOAuthState {
    pub flow_id: Uuid,
    pub provider_id: String,
    pub scopes: Vec<String>,
    pub scope: ResourceScope,
    pub redirect_uri: String,
    pub code_verifier: String,
    pub code_challenge: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct OAuthStateStore {
    states: Mutex<HashMap<String, PendingOAuthState>>,
}

impl OAuthStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &self,
        provider_id: impl Into<String>,
        scopes: Vec<String>,
        scope: ResourceScope,
        redirect_uri: impl Into<String>,
    ) -> Result<(String, PendingOAuthState), OAuthError> {
        let state = random_url_token(32);
        let code_verifier = random_url_token(64);
        let code_challenge = pkce_challenge(&code_verifier);
        let pending = PendingOAuthState {
            flow_id: Uuid::new_v4(),
            provider_id: provider_id.into(),
            scopes,
            scope,
            redirect_uri: redirect_uri.into(),
            code_verifier,
            code_challenge,
            created_at: Utc::now(),
        };
        let mut states = self.lock_states()?;
        purge_expired(&mut states);
        states.insert(state.clone(), pending.clone());
        Ok((state, pending))
    }

    pub fn take(&self, state: &str) -> Result<PendingOAuthState, OAuthError> {
        let mut states = self.lock_states()?;
        purge_expired(&mut states);
        states.remove(state).ok_or(OAuthError::InvalidState)
    }

    pub fn len(&self) -> Result<usize, OAuthError> {
        let mut states = self.lock_states()?;
        purge_expired(&mut states);
        Ok(states.len())
    }

    pub fn is_empty(&self) -> Result<bool, OAuthError> {
        Ok(self.len()? == 0)
    }

    fn lock_states(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, PendingOAuthState>>, OAuthError> {
        self.states
            .lock()
            .map_err(|_| OAuthError::invalid_token("OAuth state store lock poisoned"))
    }
}

fn purge_expired(states: &mut HashMap<String, PendingOAuthState>) {
    let cutoff = Utc::now() - OAUTH_STATE_TTL;
    states.retain(|_, state| state.created_at > cutoff);
}

fn random_url_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}
