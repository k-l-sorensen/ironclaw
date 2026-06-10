//! Declarative tenant/operator blueprint format and parser for IronClaw Reborn.
//!
//! This crate implements the first slice of epic
//! [#3036](https://github.com/nearai/ironclaw/issues/3036): the
//! `ironclaw.config/v1` blueprint *format*, its parser, validation, and the
//! file-reference lockfile. It is deliberately a boundary crate — it turns
//! operator-authored source into a validated [`Blueprint`] AST and a
//! [`Lockfile`], and stops there.
//!
//! A blueprint is **never** the runtime source of truth. It is an *input* that
//! a later apply slice reconciles into the typed Reborn repositories. Reading a
//! setting back from "the last applied blueprint" is a bug; read from the repo.
//!
//! Invariants enforced here (per the epic acceptance criteria):
//!
//! - `api_version` is mandatory and locks the schema major
//!   ([`SUPPORTED_API_VERSION`]); a different major is a hard error.
//! - Unknown keys at any level are a hard error (`deny_unknown_fields`).
//! - Inline secret material is rejected with the offending path; the only
//!   legitimate secret reference is a `${secret:<name>}` handle.
//! - File references are root-relative, cannot escape the blueprint root, and
//!   are embedded in the lockfile by SHA-256.
//! - Parsing is round-trippable: `parse → serialize → parse` yields an equal
//!   AST.
//!
//! ```
//! let src = r#"
//! api_version = "ironclaw.config/v1"
//! kind = "Blueprint"
//!
//! [scope]
//! tenant = "acme"
//!
//! [providers]
//! default_llm = "anthropic"
//!
//! [providers.anthropic]
//! model = "claude-opus-4-7"
//! api_key = "${secret:anthropic_api_key}"
//! "#;
//! let blueprint = ironclaw_blueprint::parse(src).expect("valid blueprint");
//! assert_eq!(blueprint.scope.tenant.as_deref(), Some("acme"));
//! ```

mod error;
mod lockfile;
mod parser;
mod schema;
mod secret_scan;

pub use error::BlueprintError;
pub use lockfile::{FileRefSite, LockedFile, Lockfile};
pub use parser::{SUPPORTED_API_VERSION, parse};
pub use schema::{
    AgentLoop, AppliesTo, Blueprint, BlueprintKind, CapabilitySurface, Extension, ExtensionAuth,
    HarnessBinding, InlineHarness, Mission, Project, ProjectSeed, PromptOverlay, ProviderEntry,
    Providers, RequiredRef, Runtime, Scope, Skill, SystemPrompt,
};

/// Re-serialize a blueprint back to TOML. Primarily for round-trip tests and
/// the `config diff` surface; the apply path reconciles the AST, it does not
/// re-emit source.
pub fn to_toml(blueprint: &Blueprint) -> Result<String, toml::ser::Error> {
    toml::to_string(blueprint)
}
