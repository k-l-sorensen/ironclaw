//! Durable storage adapters for Reborn OpenAI-compatible refs.
//!
//! This crate keeps persistence behind the
//! [`OpenAiCompatRefStore`](ironclaw_reborn_openai_compat::OpenAiCompatRefStore)
//! port. The OpenAI-compatible contract crate stays side-effect free; Reborn
//! composition can choose this filesystem-backed adapter when wiring concrete
//! route behavior.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, RecordKind, RecordVersion, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use ironclaw_reborn_openai_compat::{
    OpenAiCompatActorScope, OpenAiCompatBindInternalRefs, OpenAiCompatIdempotencyConflict,
    OpenAiCompatIdempotencyKey, OpenAiCompatPublicId, OpenAiCompatRefError, OpenAiCompatRefLookup,
    OpenAiCompatRefReservation, OpenAiCompatRefReservationOutcome, OpenAiCompatRefStore,
    OpenAiCompatResourceBinding, OpenAiCompatResourceMapping, OpenAiCompatRouteSurface,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;

const DEFAULT_REF_ROOT: &str = "/engine/openai_compat/refs";
const STATE_FILE_NAME: &str = "state.json";
const REF_STATE_RECORD_KIND: &str = "openai_compat_ref_state";
const FILESYSTEM_CAS_RETRIES: usize = 5;

#[derive(Clone)]
pub struct FilesystemOpenAiCompatRefStore {
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
    cas_retries: usize,
}

impl FilesystemOpenAiCompatRefStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self::with_root(filesystem, default_ref_root())
    }

    pub fn with_root(filesystem: Arc<dyn RootFilesystem>, root: VirtualPath) -> Self {
        Self {
            filesystem,
            root,
            cas_retries: FILESYSTEM_CAS_RETRIES,
        }
    }

    pub fn with_cas_retries(mut self, cas_retries: usize) -> Self {
        self.cas_retries = cas_retries;
        self
    }

    fn state_path(&self) -> Result<VirtualPath, OpenAiCompatRefError> {
        state_path(&self.root)
    }

    async fn load_state(
        &self,
    ) -> Result<(StoredOpenAiCompatRefState, Option<RecordVersion>), OpenAiCompatRefError> {
        let path = self.state_path()?;
        let Some(entry) = self
            .filesystem
            .get(&path)
            .await
            .map_err(filesystem_error("load OpenAI-compatible ref state"))?
        else {
            return Ok((StoredOpenAiCompatRefState::default(), None));
        };
        let state: StoredOpenAiCompatRefState = entry
            .entry
            .parse_json()
            .map_err(corrupt_mapping("deserialize OpenAI-compatible ref state"))?;
        state.validate()?;
        Ok((state, Some(entry.version)))
    }

    async fn save_state(
        &self,
        state: &StoredOpenAiCompatRefState,
        version: Option<RecordVersion>,
    ) -> Result<(), SaveStateError> {
        let path = self.state_path()?;
        let cas = version.map_or(CasExpectation::Absent, CasExpectation::Version);
        match self
            .filesystem
            .put(&path, entry_for_state(state)?, cas)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(SaveStateError::CasConflict),
            Err(error) => Err(SaveStateError::Ref(filesystem_error(
                "save OpenAI-compatible ref state",
            )(error))),
        }
    }
}

#[cfg(feature = "libsql")]
pub struct RebornLibSqlOpenAiCompatRefStore {
    inner: FilesystemOpenAiCompatRefStore,
}

#[cfg(feature = "libsql")]
impl RebornLibSqlOpenAiCompatRefStore {
    pub fn new(filesystem: Arc<LibSqlRootFilesystem>) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::new(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<LibSqlRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl OpenAiCompatRefStore for RebornLibSqlOpenAiCompatRefStore {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        self.inner.reserve(request).await
    }

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.bind_internal_refs(request).await
    }

    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.lookup_authorized(request).await
    }
}

#[cfg(feature = "postgres")]
pub struct RebornPostgresOpenAiCompatRefStore {
    inner: FilesystemOpenAiCompatRefStore,
}

#[cfg(feature = "postgres")]
impl RebornPostgresOpenAiCompatRefStore {
    pub fn new(filesystem: Arc<PostgresRootFilesystem>) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::new(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<PostgresRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl OpenAiCompatRefStore for RebornPostgresOpenAiCompatRefStore {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        self.inner.reserve(request).await
    }

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.bind_internal_refs(request).await
    }

    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.lookup_authorized(request).await
    }
}

#[async_trait]
impl OpenAiCompatRefStore for FilesystemOpenAiCompatRefStore {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        self.reserve_with_cas(request).await
    }

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.bind_with_cas(request).await
    }

    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        let (state, _) = self.load_state().await?;
        let Some(mapping) = state.mapping_by_public_id(&request.public_id) else {
            return Ok(None);
        };
        if !mapping.is_authorized_for(&request.requester) {
            return Ok(None);
        }
        Ok(Some(mapping.clone()))
    }
}

impl FilesystemOpenAiCompatRefStore {
    async fn reserve_with_cas(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        for _ in 0..self.cas_retries {
            let (mut state, version) = self.load_state().await?;
            if let Some(key) = request.idempotency_key.as_ref()
                && let Some(mapping) =
                    state.mapping_by_idempotency(&request.owner, request.surface, key)
            {
                if mapping.request_fingerprint == request.request_fingerprint {
                    return Ok(OpenAiCompatRefReservationOutcome::Replayed(mapping.clone()));
                }
                return Ok(OpenAiCompatRefReservationOutcome::Conflict(
                    OpenAiCompatIdempotencyConflict {
                        surface: request.surface,
                    },
                ));
            }

            let mapping = new_pending_mapping(&state, &request);
            state.mappings.push(mapping.clone());
            match self.save_state(&state, version).await {
                Ok(()) => return Ok(OpenAiCompatRefReservationOutcome::Created(mapping)),
                Err(SaveStateError::CasConflict) => continue,
                Err(SaveStateError::Ref(error)) => return Err(error),
            }
        }
        Err(OpenAiCompatRefError::StoreUnavailable)
    }

    async fn bind_with_cas(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        for _ in 0..self.cas_retries {
            let (mut state, version) = self.load_state().await?;
            let Some(mapping) = state.mapping_by_public_id_mut(&request.public_id) else {
                return Ok(None);
            };
            if !mapping.is_authorized_for(&request.owner) {
                return Ok(None);
            }
            mapping.binding = OpenAiCompatResourceBinding::Bound {
                internal_refs: request.internal_refs.clone(),
            };
            let updated = mapping.clone();
            match self.save_state(&state, version).await {
                Ok(()) => return Ok(Some(updated)),
                Err(SaveStateError::CasConflict) => continue,
                Err(SaveStateError::Ref(error)) => return Err(error),
            }
        }
        Err(OpenAiCompatRefError::StoreUnavailable)
    }
}

enum SaveStateError {
    CasConflict,
    Ref(OpenAiCompatRefError),
}

impl From<OpenAiCompatRefError> for SaveStateError {
    fn from(error: OpenAiCompatRefError) -> Self {
        Self::Ref(error)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOpenAiCompatRefState {
    #[serde(default)]
    mappings: Vec<OpenAiCompatResourceMapping>,
}

impl StoredOpenAiCompatRefState {
    fn validate(&self) -> Result<(), OpenAiCompatRefError> {
        for mapping in &self.mappings {
            mapping.validate()?;
        }
        Ok(())
    }

    fn mapping_by_public_id(
        &self,
        public_id: &OpenAiCompatPublicId,
    ) -> Option<&OpenAiCompatResourceMapping> {
        self.mappings
            .iter()
            .find(|mapping| &mapping.public_id == public_id)
    }

    fn mapping_by_public_id_mut(
        &mut self,
        public_id: &OpenAiCompatPublicId,
    ) -> Option<&mut OpenAiCompatResourceMapping> {
        self.mappings
            .iter_mut()
            .find(|mapping| &mapping.public_id == public_id)
    }

    fn mapping_by_idempotency(
        &self,
        owner: &OpenAiCompatActorScope,
        surface: OpenAiCompatRouteSurface,
        key: &OpenAiCompatIdempotencyKey,
    ) -> Option<&OpenAiCompatResourceMapping> {
        self.mappings.iter().find(|mapping| {
            &mapping.owner == owner
                && mapping.surface == surface
                && mapping.idempotency_key.as_ref() == Some(key)
        })
    }
}

fn new_pending_mapping(
    state: &StoredOpenAiCompatRefState,
    request: &OpenAiCompatRefReservation,
) -> OpenAiCompatResourceMapping {
    loop {
        let public_id = OpenAiCompatPublicId::generate_for(request.surface);
        if state.mapping_by_public_id(&public_id).is_some() {
            continue;
        }
        let mapping = OpenAiCompatResourceMapping {
            public_id,
            owner: request.owner.clone(),
            surface: request.surface,
            request_fingerprint: request.request_fingerprint.clone(),
            idempotency_key: request.idempotency_key.clone(),
            binding: OpenAiCompatResourceBinding::Pending,
        };
        debug_assert!(mapping.validate().is_ok());
        return mapping;
    }
}

fn entry_for_state(state: &StoredOpenAiCompatRefState) -> Result<Entry, OpenAiCompatRefError> {
    state.validate()?;
    let payload = serde_json::to_value(state).map_err(corrupt_mapping(
        "serialize OpenAI-compatible ref state payload",
    ))?;
    let kind = RecordKind::new(REF_STATE_RECORD_KIND)
        .map_err(|_| OpenAiCompatRefError::StoreUnavailable)?;
    Entry::record(kind, &payload).map_err(corrupt_mapping(
        "serialize OpenAI-compatible ref state entry",
    ))
}

fn state_path(root: &VirtualPath) -> Result<VirtualPath, OpenAiCompatRefError> {
    VirtualPath::new(format!(
        "{}/{}",
        root.as_str().trim_end_matches('/'),
        STATE_FILE_NAME
    ))
    .map_err(|_| OpenAiCompatRefError::StoreUnavailable)
}

fn default_ref_root() -> VirtualPath {
    VirtualPath::new(DEFAULT_REF_ROOT).expect("DEFAULT_REF_ROOT is valid") // safety: hard-coded /engine virtual path literal.
}

fn filesystem_error(
    operation: &'static str,
) -> impl FnOnce(FilesystemError) -> OpenAiCompatRefError {
    move |error| {
        tracing::error!(
            operation,
            error_type = std::any::type_name_of_val(&error),
            "OpenAI-compatible ref store filesystem operation failed"
        );
        OpenAiCompatRefError::StoreUnavailable
    }
}

fn corrupt_mapping<E>(operation: &'static str) -> impl FnOnce(E) -> OpenAiCompatRefError
where
    E: std::fmt::Display,
{
    move |error| {
        tracing::error!(
            operation,
            error_type = std::any::type_name_of_val(&error),
            "OpenAI-compatible ref store mapping payload is invalid"
        );
        OpenAiCompatRefError::CorruptMapping
    }
}
