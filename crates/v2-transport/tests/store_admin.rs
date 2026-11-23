//! Authorization-store administration (ticket 19): `created_at`/
//! `last_seen_at`, legacy version-1 compatibility, clear/revoke by id, the
//! headless advisory lock, and the fail-closed identity-reset order.

use std::sync::Arc;

use v2_transport::auth_store::{
    Authorization, AuthorizationStore, ClientState, StoreError, StoredClient,
};
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::pairing::{self, PairingError};

const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn client(name: &str) -> StoredClient {
    StoredClient {
        client_id: String::new(), // derived on write
        public_key: unhex(CLIENT_PUB),
        display_name: name.to_owned(),
        state: ClientState::Active,
        created_at: "2026-11-16T10:00:00Z".to_owned(),
        last_seen_at: "2026-11-16T10:00:00Z".to_owned(),
    }
}

fn second_client(name: &str) -> StoredClient {
    StoredClient {
        public_key: unhex("02ea68d7b6fedf0b71878938d51d71f8729e0acb8c2c6df8b3d79e8a4b90949ee0"),
        ..client(name)
    }
}

#[test]
fn timestamps_round_trip_and_client_id_is_canonical_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let store = AuthorizationStore::open(dir.path()).unwrap();
    store.upsert(client("Test Phone")).unwrap();

    let text = std::fs::read_to_string(dir.path().join("pairing").join("clients.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["clients"][0]["client_id"], "cf7940b474310246");
    assert_eq!(json["clients"][0]["created_at"], "2026-11-16T10:00:00Z");
    assert_eq!(json["clients"][0]["last_seen_at"], "2026-11-16T10:00:00Z");

    let reopened = AuthorizationStore::open(dir.path()).unwrap();
    let record = reopened.record_for(&unhex(CLIENT_PUB)).expect("record");
    assert_eq!(record.created_at, "2026-11-16T10:00:00Z");
    assert_eq!(record.last_seen_at, "2026-11-16T10:00:00Z");
}

#[test]
fn legacy_version_one_store_without_timestamps_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let pairing_dir = dir.path().join("pairing");
    std::fs::create_dir_all(&pairing_dir).unwrap();
    std::fs::write(
        pairing_dir.join("clients.json"),
        r#"{
  "version": 1,
  "clients": [{
    "client_id": "cf7940b474310246",
    "public_key": "A9ClKd50dWQawFDHkrQs_weRB8pTENp5U9JZUSFHfA9t",
    "display_name": "Test Phone",
    "state": "active"
  }]
}"#,
    )
    .unwrap();
    let store = AuthorizationStore::open(dir.path()).expect("legacy store loads");
    let record = store.record_for(&unhex(CLIENT_PUB)).expect("record");
    assert!(record.created_at.is_empty());
    assert!(record.last_seen_at.is_empty());
}

#[test]
fn clear_all_invalidates_every_record_and_persists_an_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = AuthorizationStore::open(dir.path()).unwrap();
    store.upsert(client("One")).unwrap();
    store.upsert(second_client("Two")).unwrap();
    assert_eq!(store.clients().len(), 2);
    store.clear_all().unwrap();
    assert!(store.clients().is_empty());
    let reopened = AuthorizationStore::open(dir.path()).unwrap();
    assert!(reopened.clients().is_empty());
}

#[test]
fn revoke_by_client_id_keeps_the_record_for_audit() {
    let dir = tempfile::tempdir().unwrap();
    let store = AuthorizationStore::open(dir.path()).unwrap();
    store.upsert(client("Test Phone")).unwrap();
    let revoked = store.revoke_client("cf7940b474310246").unwrap();
    assert_eq!(revoked.state, ClientState::Revoked);
    assert_eq!(store.clients().len(), 1);
    assert!(matches!(
        store.revoke_client("deadbeefdeadbeef"),
        Err(v2_transport::auth_store::StoreError::UnknownClientId { .. })
    ));
}

#[test]
fn delete_removes_only_revoked_records_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let store = AuthorizationStore::open(dir.path()).unwrap();
    store.upsert(client("Test Phone")).unwrap();
    let client_id = "cf7940b474310246";

    let error = store
        .delete_client(client_id)
        .expect_err("an active record cannot be deleted");
    assert!(
        matches!(error, StoreError::ClientActive { .. }),
        "unexpected error {error}"
    );
    assert_eq!(store.clients().len(), 1, "the active record is unchanged");
    assert_eq!(store.clients()[0].state, ClientState::Active);

    store.revoke_client(client_id).unwrap();
    let deleted = store
        .delete_client(client_id)
        .expect("a revoked record deletes");
    assert_eq!(deleted.client_id, client_id);
    assert_eq!(deleted.state, ClientState::Revoked);
    assert!(
        store.clients().is_empty(),
        "the record is gone from the list"
    );
    assert!(
        matches!(
            store.authorize(&unhex(CLIENT_PUB)),
            Authorization::Unknown(_)
        ),
        "a deleted identity is refused at authentication"
    );

    let text = std::fs::read_to_string(dir.path().join("pairing").join("clients.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        json["clients"].as_array().unwrap().len(),
        0,
        "the record is gone from clients.json"
    );
    assert!(
        matches!(
            store.delete_client(client_id),
            Err(StoreError::UnknownClientId { .. })
        ),
        "deleting an unknown id is an error"
    );
}

#[cfg(unix)]
#[test]
fn headless_lock_is_exclusive_and_0600() {
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::mpsc;
    use std::time::Duration;

    use v2_transport::auth_store::StoreLock;

    let dir = tempfile::tempdir().unwrap();
    let lock = StoreLock::acquire(dir.path()).expect("first lock");
    let mode = std::fs::metadata(lock.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let path = dir.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _second = StoreLock::acquire(&path).expect("second lock after release");
        tx.send(()).unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(250)).is_err(),
        "a second admin operation blocks on the held lock"
    );
    drop(lock);
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the second writer proceeds after release");
    worker.join().unwrap();
}

#[cfg(unix)]
#[test]
fn try_acquire_reports_a_held_lock_without_blocking() {
    use v2_transport::auth_store::StoreLock;

    let dir = tempfile::tempdir().unwrap();
    let held = StoreLock::acquire(dir.path()).expect("first lock");
    assert!(
        StoreLock::try_acquire(dir.path())
            .expect("try acquire")
            .is_none(),
        "a held lock is reported, not waited on"
    );
    drop(held);
    let acquired = StoreLock::try_acquire(dir.path())
        .expect("try acquire after release")
        .expect("the released lock is available");
    drop(acquired);
}

#[test]
fn identity_reset_clears_pairings_before_rotating_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let config = IdentityConfig {
        data_dir: dir.path().to_path_buf(),
        allow_loose_permissions: false,
    };
    let original = ServerIdentity::open(&config).unwrap();
    let store = AuthorizationStore::open(dir.path()).unwrap();
    store.upsert(client("Test Phone")).unwrap();
    let old_pin = original.spki_sha256().to_vec();
    let old_public = original.public_key_raw().to_vec();

    let reset = pairing::reset_identity(&config, &store).expect("reset");
    assert!(store.clients().is_empty(), "old pairings invalidated");
    assert_ne!(reset.public_key_raw(), old_public.as_slice());
    assert_ne!(reset.spki_sha256(), old_pin.as_slice());
    // One backup generation of the previous identity key exists.
    assert!(dir
        .path()
        .join("identity")
        .join("server-identity.pem.bak")
        .is_file());

    // The rotated key is what a fresh process loads.
    let reloaded = ServerIdentity::open(&config).unwrap();
    assert_eq!(reloaded.public_key_raw(), reset.public_key_raw());
}

#[test]
fn pairing_core_persist_sets_timestamps_and_notifies_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(AuthorizationStore::open(dir.path()).unwrap());
    let core = pairing::PairingCore::new(
        unhex("11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c"),
        Arc::clone(&store),
    );
    let mut paired = core.subscribe_paired();
    let record = core
        .persist_client(&unhex(CLIENT_PUB), "Test Phone", pairing::PersistMode::New)
        .unwrap();
    assert_eq!(record.client_id, "cf7940b474310246");
    assert_eq!(record.created_at.len(), 20);
    assert!(record.created_at.ends_with('Z'));
    assert_eq!(record.last_seen_at.len(), 20);
    assert!(
        paired.try_recv().is_err(),
        "persistence alone must not notify; the driver notifies after PairingResult"
    );
    core.notify_paired(record.clone());
    let notified = paired.try_recv().expect("paired notification");
    assert_eq!(notified.client_id, record.client_id);
    // Re-pairing keeps the original created_at while updating last_seen_at.
    let again = core
        .persist_client(&unhex(CLIENT_PUB), "Renamed", pairing::PersistMode::New)
        .unwrap();
    assert_eq!(again.created_at, record.created_at);
    assert_eq!(again.display_name, "Renamed");
    assert_eq!(store.clients().len(), 1);
}

#[test]
fn revoked_record_restores_in_place_with_the_saved_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(AuthorizationStore::open(dir.path()).unwrap());
    let core = pairing::PairingCore::new(
        unhex("11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c"),
        Arc::clone(&store),
    );
    let first = core
        .persist_client(&unhex(CLIENT_PUB), "Test Phone", pairing::PersistMode::New)
        .unwrap();
    let client_id = first.client_id.clone();
    let created_at = first.created_at.clone();
    assert!(
        core.restore_candidate(&unhex(CLIENT_PUB)).is_none(),
        "an active record is not a restore"
    );

    store.revoke_client(&client_id).unwrap();
    assert_eq!(
        core.restore_candidate(&unhex(CLIENT_PUB)).as_deref(),
        Some("Test Phone"),
        "a revoked record would be restored with its saved name"
    );

    let restored = core
        .persist_client(&unhex(CLIENT_PUB), "Renamed", pairing::PersistMode::Restore)
        .expect("explicit restore");
    assert_eq!(restored.client_id, client_id, "the same client id");
    assert_eq!(restored.created_at, created_at, "created_at is preserved");
    assert_eq!(restored.state, ClientState::Active);
    assert_eq!(
        restored.display_name, "Test Phone",
        "the saved record is restored, not replaced by the new name"
    );
    assert_eq!(store.clients().len(), 1, "exactly one record");
    assert_eq!(store.clients()[0].client_id, client_id);
    assert_eq!(store.clients()[0].state, ClientState::Active);
    assert!(core.restore_candidate(&unhex(CLIENT_PUB)).is_none());
}

#[test]
fn revoked_record_is_not_reactivated_without_restore_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(AuthorizationStore::open(dir.path()).unwrap());
    let core = pairing::PairingCore::new(
        unhex("11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c"),
        Arc::clone(&store),
    );
    let first = core
        .persist_client(&unhex(CLIENT_PUB), "Test Phone", pairing::PersistMode::New)
        .unwrap();
    store.revoke_client(&first.client_id).unwrap();

    let error = core
        .persist_client(&unhex(CLIENT_PUB), "Test Phone", pairing::PersistMode::New)
        .expect_err("a plain pairing must not restore a revoked record");
    assert!(
        matches!(error, PairingError::RestoreNotConfirmed),
        "unexpected error {error}"
    );
    assert_eq!(store.clients().len(), 1, "no record is added");
    assert_eq!(store.clients()[0].client_id, first.client_id);
    assert_eq!(store.clients()[0].state, ClientState::Revoked);
}

#[test]
fn an_explicit_restore_of_a_non_revoked_identity_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(AuthorizationStore::open(dir.path()).unwrap());
    let core = pairing::PairingCore::new(
        unhex("11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c"),
        Arc::clone(&store),
    );

    // No record at all: Restore must never insert a brand-new identity.
    let error = core
        .persist_client(
            &unhex(CLIENT_PUB),
            "Test Phone",
            pairing::PersistMode::Restore,
        )
        .expect_err("restore requires a revoked record");
    assert!(
        matches!(error, PairingError::RestoreUnavailable),
        "unexpected error {error}"
    );
    assert!(store.clients().is_empty(), "no record is added");

    // An active record: Restore is equally invalid and must not rewrite it.
    let active = core
        .persist_client(&unhex(CLIENT_PUB), "Test Phone", pairing::PersistMode::New)
        .unwrap();
    let error = core
        .persist_client(
            &unhex(CLIENT_PUB),
            "Test Phone",
            pairing::PersistMode::Restore,
        )
        .expect_err("restore requires a revoked record");
    assert!(
        matches!(error, PairingError::RestoreUnavailable),
        "unexpected error {error}"
    );
    assert_eq!(store.clients().len(), 1);
    assert_eq!(store.clients()[0].client_id, active.client_id);
    assert_eq!(store.clients()[0].state, ClientState::Active);
}
