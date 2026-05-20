#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw_host_runtime::{CapabilitySurfaceVersion, ProductionWiringConfig};
use ironclaw_reborn_composition::{
    LibSqlProductionSubstrateConfig, RebornCompositionError,
    build_libsql_production_host_runtime_services,
};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError};
use secrecy::SecretString;
use tempfile::tempdir;

#[tokio::test]
async fn libsql_substrate_builder_wires_production_components_without_local_only_seams() {
    let dir = tempdir().unwrap();
    let state_db_path = dir.path().join("state.db");
    let events_db_path = dir.path().join("events.db");
    let database = Arc::new(
        libsql::Builder::new_local(state_db_path.display().to_string())
            .build()
            .await
            .unwrap(),
    );

    let services = build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database: Arc::clone(&database),
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: events_db_path.display().to_string(),
            auth_token: None,
        },
        secret_master_key: Some(SecretString::from("01234567890123456789012345678901")),
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
    })
    .await
    .unwrap();

    let production_config = ProductionWiringConfig::new([]).require_runtime_http_egress();
    services
        .validate_production_wiring(&production_config)
        .expect("substrate-only production wiring should not use fake seams");

    let conn = database.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT 1 FROM root_filesystem_entries WHERE path = ?1",
            libsql::params!["/secrets/key-check/active.json"],
        )
        .await
        .unwrap();
    assert!(
        rows.next().await.unwrap().is_some(),
        "secret readiness sentinel should be stored through root filesystem entries"
    );
}

#[tokio::test]
async fn libsql_substrate_builder_rejects_missing_secret_master_key() {
    let dir = tempdir().unwrap();
    let state_db_path = dir.path().join("state.db");
    let events_db_path = dir.path().join("events.db");
    let database = Arc::new(
        libsql::Builder::new_local(state_db_path.display().to_string())
            .build()
            .await
            .unwrap(),
    );

    let result = build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database,
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: events_db_path.display().to_string(),
            auth_token: None,
        },
        secret_master_key: None,
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
    })
    .await;

    assert!(matches!(
        result,
        Err(RebornCompositionError::MissingSecretMasterKey)
    ));
}

#[tokio::test]
async fn libsql_substrate_builder_rejects_wrong_secret_master_key_from_filesystem_store() {
    let dir = tempdir().unwrap();
    let state_db_path = dir.path().join("state.db");
    let first_events_db_path = dir.path().join("events-a.db");
    let second_events_db_path = dir.path().join("events-b.db");
    let database = Arc::new(
        libsql::Builder::new_local(state_db_path.display().to_string())
            .build()
            .await
            .unwrap(),
    );

    build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database: Arc::clone(&database),
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: first_events_db_path.display().to_string(),
            auth_token: None,
        },
        secret_master_key: Some(SecretString::from("01234567890123456789012345678901")),
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
    })
    .await
    .unwrap();

    let result = build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database,
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: second_events_db_path.display().to_string(),
            auth_token: None,
        },
        secret_master_key: Some(SecretString::from("abcdefghijklmnopqrstuvwxyzABCDEF")),
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
    })
    .await;

    assert!(matches!(result, Err(RebornCompositionError::Secret(_))));
}

#[derive(Debug)]
struct RecordingSchedulerWakeNotifier;

impl TurnRunWakeNotifier for RecordingSchedulerWakeNotifier {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        Ok(())
    }
}
