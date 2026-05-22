//! Provider-agnostic OAuth substrate for Reborn native extensions.
//!
//! This crate owns provider registration, OAuth state/PKCE, brokered/direct
//! token exchange, token persistence row shape, refresh serialization, callback
//! routing, and resume notification signals. Concrete providers live outside
//! this crate.
#![warn(unreachable_pub)]

mod callback;
mod error;
mod flow;
mod provider;
mod refresh;
mod resume;
mod state;
mod storage;

pub use callback::{CallbackResponse, OAuthCallbackQuery, router};
pub use error::OAuthError;
pub use flow::{
    OAuthFlow, OAuthRuntime, OAuthRuntimeBuilder, ProviderMode, StartedFlow, broker_auth_from_env,
};
pub use provider::{OAuthProvider, ProviderRegistry};
pub use refresh::RefreshScheduler;
pub use resume::{OAuthCallbackOutcome, OAuthResumeNotifier, ResumeSignal};
pub use state::{OAUTH_STATE_TTL, OAuthStateStore, PendingOAuthState};
pub use storage::{TokenPersister, TokenSet};

#[allow(dead_code)]
fn _assert_runtime_is_route_state() {
    fn assert_bounds<T: Clone + Send + Sync + 'static>() {}
    assert_bounds::<OAuthRuntime>();
}
