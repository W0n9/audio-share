//! The §8.1 server-side authorization store: `pairing/clients.json` — public
//! keys and metadata only, no secrets. Test-only credential preloading hands
//! the Server its authorized-Client list (ticket 07: production pairing is a
//! later phase; pairing.md §11.3 keeps bypasses in test builds).

use std::path::PathBuf;

use v2_transport::auth_store::{AuthorizationStore, ClientState, StoreError, StoredClient};

fn temp_dir() -> PathBuf {
    tempfile::tempdir().expect("tempdir").path().to_path_buf()
}

fn client(pub_hex: &str, name: &str) -> StoredClient {
    StoredClient {
        client_id: hex8(pub_hex),
        public_key: unhex(pub_hex),
        display_name: name.to_owned(),
        state: ClientState::Active,
        created_at: String::new(),
        last_seen_at: String::new(),
    }
}

fn hex8(pub_hex: &str) -> String {
    // First 8 bytes of SHA-256(public_key), hex (pairing.md §8.1).
    use sha2::{Digest, Sha256};
    let bytes = unhex(pub_hex);
    let digest = Sha256::digest(&bytes);
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const OTHER_PUB: &str = "02dd4d011a1b03e2a9e8075b627e88b4ed87559e6c65d84f7c3978a7413bc0d5d4";

#[test]
fn unknown_identity_classifies_202() {
    let store = AuthorizationStore::in_memory();
    let verdict = store.authorize(&unhex(CLIENT_PUB));
    assert!(matches!(
        verdict,
        v2_transport::auth_store::Authorization::Unknown(_)
    ));
}

#[test]
fn active_identity_authorizes_with_record() {
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(client(CLIENT_PUB, "Test Phone"));
    match store.authorize(&unhex(CLIENT_PUB)) {
        v2_transport::auth_store::Authorization::Authorized(record) => {
            assert_eq!(record.display_name, "Test Phone");
            assert_eq!(record.state, ClientState::Active);
        }
        other => panic!("expected Authorized, got {other:?}"),
    }
}

#[test]
fn revoked_identity_classifies_204() {
    // pairing.md §9.2: a revoked identity is refused immediately; 204 has
    // requires_repairing=false (admin action) unlike 202.
    let store = AuthorizationStore::in_memory();
    let mut record = client(CLIENT_PUB, "Test Phone");
    record.state = ClientState::Revoked;
    store.insert_for_tests(record);
    match store.authorize(&unhex(CLIENT_PUB)) {
        v2_transport::auth_store::Authorization::Revoked(record) => {
            assert_eq!(record.state, ClientState::Revoked);
        }
        other => panic!("expected Revoked, got {other:?}"),
    }
}

#[test]
fn store_persists_as_clients_json_with_0600() {
    // pairing.md §10.2: <data dir>/pairing/clients.json, file 0600, dir 0700.
    let dir = temp_dir();
    {
        let store = AuthorizationStore::open(&dir).expect("open fresh store");
        store.insert_for_tests(client(CLIENT_PUB, "Test Phone"));
    }
    let path = dir.join("pairing").join("clients.json");
    assert!(path.is_file(), "store must exist at {}", path.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "clients.json must be 0600, got {:o}",
            mode
        );
        let dir_mode = std::fs::metadata(dir.join("pairing"))
            .expect("dir metadata")
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700, "pairing dir must be 0700");
    }
    // Reload sees the record.
    let store = AuthorizationStore::open(&dir).expect("reopen");
    assert!(matches!(
        store.authorize(&unhex(CLIENT_PUB)),
        v2_transport::auth_store::Authorization::Authorized(_)
    ));
}

#[test]
fn revocation_is_visible_to_new_connections_immediately() {
    // pairing.md §9.2: the authorization state is consulted at every AuthInit;
    // no cache outlives the store.
    let dir = temp_dir();
    let store = AuthorizationStore::open(&dir).expect("open");
    store.insert_for_tests(client(CLIENT_PUB, "Test Phone"));
    assert!(matches!(
        store.authorize(&unhex(CLIENT_PUB)),
        v2_transport::auth_store::Authorization::Authorized(_)
    ));
    store.revoke(&unhex(CLIENT_PUB)).expect("revoke");
    assert!(matches!(
        store.authorize(&unhex(CLIENT_PUB)),
        v2_transport::auth_store::Authorization::Revoked(_)
    ));
}

#[test]
fn corrupted_store_fails_closed() {
    // pairing.md §10.3: a store that fails validation disables v2 auth
    // (fail-closed) and is never regenerated or silently truncated.
    let dir = temp_dir();
    let pairing = dir.join("pairing");
    std::fs::create_dir_all(&pairing).expect("mkdir");
    std::fs::write(pairing.join("clients.json"), "this is not json {").expect("corrupt");
    let error = AuthorizationStore::open(&dir).expect_err("corruption must fail closed");
    let text = error.to_string();
    assert!(
        text.contains("fails validation") && text.contains("fail"),
        "the error should explain the fail-closed refusal: {error}"
    );
}

#[test]
fn missing_store_starts_empty_and_creates_file_on_first_write() {
    let dir = temp_dir();
    let store = AuthorizationStore::open(&dir).expect("open fresh");
    assert!(matches!(
        store.authorize(&unhex(CLIENT_PUB)),
        v2_transport::auth_store::Authorization::Unknown(_)
    ));
    // Read-only behavior never writes; the file appears on the first write.
    assert!(!dir.join("pairing").join("clients.json").exists());
    store.insert_for_tests(client(CLIENT_PUB, "Test Phone"));
    assert!(dir.join("pairing").join("clients.json").is_file());
}

#[test]
fn a_backup_generation_is_kept_on_rewrite() {
    // pairing.md §10.3: atomic write + one backup generation. The backup keeps
    // the full file name plus `.bak` (clients.json.bak next to clients.json).
    let dir = temp_dir();
    let store = AuthorizationStore::open(&dir).expect("open");
    store.insert_for_tests(client(CLIENT_PUB, "Test Phone"));
    store.insert_for_tests(client(OTHER_PUB, "Second"));
    let backup = dir.join("pairing").join("clients.json.bak");
    assert!(
        backup.is_file(),
        "one .bak generation must be kept as clients.json.bak"
    );
    // The backup holds the pre-rewrite content (only the first client).
    let text = std::fs::read_to_string(&backup).expect("backup text");
    assert!(text.contains(&hex8(CLIENT_PUB)));
    assert!(!text.contains(&hex8(OTHER_PUB)));
}

#[test]
fn multiple_records_survive_a_round_trip() {
    let dir = temp_dir();
    let store = AuthorizationStore::open(&dir).expect("open");
    store.insert_for_tests(client(CLIENT_PUB, "Test Phone"));
    store.insert_for_tests(client(OTHER_PUB, "Second"));
    drop(store);
    let store = AuthorizationStore::open(&dir).expect("reopen");
    assert!(matches!(
        store.authorize(&unhex(CLIENT_PUB)),
        v2_transport::auth_store::Authorization::Authorized(_)
    ));
    assert!(matches!(
        store.authorize(&unhex(OTHER_PUB)),
        v2_transport::auth_store::Authorization::Authorized(_)
    ));
}

#[test]
fn revoke_unknown_key_is_a_distinct_error() {
    // Revoking an absent key is an operator mistake, not a corrupted store;
    // the error must say so instead of suggesting clients.json.bak recovery.
    let store = AuthorizationStore::in_memory();
    let error = store.revoke(&unhex(CLIENT_PUB)).expect_err("unknown key");
    assert!(
        matches!(error, StoreError::UnknownClient { .. }),
        "got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// Load-time validation (pairing.md §8.1 / §10.3): every malformed record is a
// fail-closed load error, never a silently accepted credential.
// ---------------------------------------------------------------------------

/// Write one raw store file and open it. `entries` is the JSON array body.
fn open_raw(entries: &str) -> Result<AuthorizationStore, v2_transport::auth_store::StoreError> {
    let dir = temp_dir();
    let pairing = dir.join("pairing");
    std::fs::create_dir_all(&pairing).expect("mkdir");
    std::fs::write(
        pairing.join("clients.json"),
        format!(r#"{{"version": 1, "clients": [{entries}]}}"#),
    )
    .expect("write store");
    AuthorizationStore::open(&dir)
}

fn record_json(client_id: &str, public_key_b64: &str, display_name: &str) -> String {
    format!(
        r#"{{"client_id": "{client_id}", "public_key": "{public_key_b64}", "display_name": "{display_name}", "state": "active"}}"#
    )
}

const CLIENT_PUB_B64: &str = "A9ClKd50dWQawFDHkrQs_weRB8pTENp5U9JZUSFHfA9t";

fn load_error(entries: &str) -> String {
    open_raw(entries)
        .expect_err("malformed store must fail closed")
        .to_string()
}

#[test]
fn public_key_must_be_a_33_byte_compressed_point() {
    // 32 bytes, not the §8.1 33-byte SEC1-compressed point.
    let short = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let error = load_error(&record_json("cf7940b474310246", short, "Test Phone"));
    assert!(
        error.contains("public_key"),
        "the refusal must name the field: {error}"
    );
    assert!(
        error.contains("33"),
        "the refusal should cite the compressed-point shape: {error}"
    );
}

#[test]
fn public_key_must_decode_to_a_valid_curve_point() {
    // 0x02 || 32 x 0xff: correct length and prefix, but x is outside the field
    // and no point decodes — the store must not hold an unusable credential.
    let off_curve = "Av__________________________________________";
    let error = load_error(&record_json("cf7940b474310246", off_curve, "Test Phone"));
    assert!(
        error.contains("public_key"),
        "the refusal must name the field: {error}"
    );
}

#[test]
fn client_id_must_match_the_derived_value() {
    // pairing.md §8.1: client_id = first 8 bytes of SHA-256(public_key), hex.
    let error = load_error(&record_json(
        "deadbeefdeadbeef",
        CLIENT_PUB_B64,
        "Test Phone",
    ));
    assert!(
        error.contains("client_id"),
        "the refusal must name the field: {error}"
    );
}

#[test]
fn client_id_case_is_canonical_lowercase() {
    // pairing.md §8.1: only the canonical lowercase hex spelling is valid.
    // Mixed/uppercase hex names the same bytes but is not the canonical id,
    // so loading it must fail closed rather than silently normalize.
    let canonical = hex8(CLIENT_PUB);
    let mixed = format!("{}{}", canonical[..2].to_ascii_uppercase(), &canonical[2..]);
    assert_ne!(mixed, canonical);
    let error = load_error(&record_json(&mixed, CLIENT_PUB_B64, "Test Phone"));
    assert!(
        error.contains("client_id"),
        "the refusal must name the field: {error}"
    );
    assert!(
        error.contains("canonical"),
        "the refusal should explain the canonical requirement: {error}"
    );
}

#[test]
fn persisted_client_id_is_always_canonical() {
    // A record may reach the store through an admin/import path with a
    // noncanonical in-memory id; persistence must still write the derived
    // lowercase value, and a reload must see that canonical id.
    let dir = temp_dir();
    let store = AuthorizationStore::open(&dir).expect("open");
    let mut record = client(CLIENT_PUB, "Test Phone");
    record.client_id = record.client_id.to_ascii_uppercase();
    store.upsert(record).expect("upsert");

    let path = dir.join("pairing").join("clients.json");
    let text = std::fs::read_to_string(&path).expect("read clients.json");
    assert!(
        text.contains(&hex8(CLIENT_PUB)),
        "canonical id must be persisted: {text}"
    );
    assert!(
        !text.contains(&hex8(CLIENT_PUB).to_ascii_uppercase()),
        "noncanonical uppercase id must never be persisted: {text}"
    );

    drop(store);
    let store = AuthorizationStore::open(&dir).expect("reopen");
    match store.authorize(&unhex(CLIENT_PUB)) {
        v2_transport::auth_store::Authorization::Authorized(record) => {
            assert_eq!(record.client_id, hex8(CLIENT_PUB));
        }
        other => panic!("expected Authorized, got {other:?}"),
    }
}

#[test]
fn display_name_is_capped_at_63_bytes() {
    let name = "x".repeat(64);
    let error = load_error(&record_json("cf7940b474310246", CLIENT_PUB_B64, &name));
    assert!(
        error.contains("display_name"),
        "the refusal must name the field: {error}"
    );
}

#[test]
fn duplicate_client_ids_are_rejected() {
    let one = record_json("cf7940b474310246", CLIENT_PUB_B64, "Test Phone");
    let two = record_json("cf7940b474310246", CLIENT_PUB_B64, "Impostor");
    let error = load_error(&format!("{one}, {two}"));
    assert!(
        error.contains("duplicate"),
        "the refusal must explain the duplicate: {error}"
    );
}

// ---------------------------------------------------------------------------
// Runtime reload (pairing.md §9.2: revocation takes effect within ≤ 5 s).
// The store watches `clients.json`; a deleted or corrupt file must not leave
// stale credentials usable in memory (§10.3 fail-closed).
// ---------------------------------------------------------------------------

#[test]
fn deleting_the_file_revokes_the_identity_on_reload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = AuthorizationStore::open(dir.path()).expect("open");
    store
        .upsert(client(CLIENT_PUB, "Test Phone"))
        .expect("upsert");
    let path = dir.path().join("pairing").join("clients.json");
    let saved = std::fs::read(&path).expect("read store");
    assert!(matches!(
        store.authorize(&unhex(CLIENT_PUB)),
        v2_transport::auth_store::Authorization::Authorized(_)
    ));

    std::fs::remove_file(&path).expect("delete store");
    assert!(store.reload_if_changed().expect("reload after delete"));
    assert!(
        matches!(
            store.authorize(&unhex(CLIENT_PUB)),
            v2_transport::auth_store::Authorization::Unknown(_)
        ),
        "a removed record must stop authorizing"
    );

    std::fs::write(&path, &saved).expect("restore store");
    assert!(store.reload_if_changed().expect("reload after re-add"));
    assert!(
        matches!(
            store.authorize(&unhex(CLIENT_PUB)),
            v2_transport::auth_store::Authorization::Authorized(_)
        ),
        "re-adding the record restores authorization"
    );
}

#[test]
fn corrupt_reload_fails_closed_and_recovers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = AuthorizationStore::open(dir.path()).expect("open");
    store
        .upsert(client(CLIENT_PUB, "Test Phone"))
        .expect("upsert");
    let path = dir.path().join("pairing").join("clients.json");
    let saved = std::fs::read(&path).expect("read store");

    std::fs::write(&path, b"{not json").expect("corrupt store");
    let error = store
        .reload_if_changed()
        .expect_err("corruption must be reported");
    assert!(
        matches!(error, StoreError::Corrupted { .. }),
        "got {error:?}"
    );
    assert!(
        matches!(
            store.authorize(&unhex(CLIENT_PUB)),
            v2_transport::auth_store::Authorization::Unavailable
        ),
        "a corrupt store must not keep authorizing from memory"
    );

    std::fs::write(&path, &saved).expect("restore store");
    store.reload_if_changed().expect("reload after repair");
    assert!(matches!(
        store.authorize(&unhex(CLIENT_PUB)),
        v2_transport::auth_store::Authorization::Authorized(_)
    ));
}

// ---------------------------------------------------------------------------
// Ticket 27: replacing `clients.json` must never expose a transient missing
// file. The old write sequence renamed the live target to `.bak` before
// writing the replacement, so a concurrent reload could see `Missing`, clear
// the in-memory table, and answer 202 AUTHORIZATION_UNKNOWN until the next
// successful reload (pairing.md §10.3: every write is atomic; a missing file
// means the records were really removed, §9.2).
// ---------------------------------------------------------------------------

/// A replacement that fails part-way must not remove the live store. The old
/// sequence moved the target to `.bak` *before* creating the temp file, so a
/// failure in the temp-file step left `clients.json` missing and the next
/// reload cleared every authorization. This is the deterministic form of the
/// same ordering defect: no concurrency or timing is needed to expose it.
#[test]
fn failed_replacement_never_hides_or_clears_live_authorizations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = AuthorizationStore::open(dir.path()).expect("open");
    store
        .upsert(client(CLIENT_PUB, "Test Phone"))
        .expect("first write");
    let pairing_dir = dir.path().join("pairing");
    let path = pairing_dir.join("clients.json");
    let before = std::fs::read(&path).expect("read store");

    // Occupy the temp path with a directory: creating the temp file fails.
    // Under the old ordering the live target has already been renamed away.
    let temp = pairing_dir.join(".clients.json.tmp");
    std::fs::create_dir(&temp).expect("block temp path");
    let result = store.upsert(client(CLIENT_PUB, "Test Phone Renamed"));
    assert!(result.is_err(), "the blocked temp path must fail the write");

    // The reader only sees a real deletion when the operator really deleted
    // the file. A failed replacement must leave both the file and the
    // in-memory authorizations intact.
    store
        .reload_if_changed()
        .expect("reload after failed write");
    assert!(
        matches!(
            store.authorize(&unhex(CLIENT_PUB)),
            v2_transport::auth_store::Authorization::Authorized(_)
        ),
        "a failed replacement was mistaken for a deleted store and cleared live authorizations"
    );
    assert_eq!(
        std::fs::read(&path).expect("live store still exists"),
        before,
        "the live store must not be removed by a failed replacement"
    );

    std::fs::remove_dir(&temp).expect("unblock temp path");
}

/// While a replacement is physically in flight, a reload must never observe
/// the live target as missing. A FIFO at the temp path blocks the writer at a
/// controlled point inside `upsert` *without* any test hook in production
/// code; under the old sequence that point is after the live target was
/// renamed to `.bak`.
#[cfg(unix)]
#[test]
fn reload_during_a_replacement_never_observes_a_missing_store() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(AuthorizationStore::open(dir.path()).expect("open"));
    store
        .upsert(client(CLIENT_PUB, "Test Phone"))
        .expect("first write");
    let pairing_dir = dir.path().join("pairing");
    let path = pairing_dir.join("clients.json");
    let temp = pairing_dir.join(".clients.json.tmp");

    // A FIFO blocks `File::create(temp)` until a reader opens it.
    let temp_c = CString::new(temp.as_os_str().as_bytes()).expect("temp path");
    let created = unsafe { libc::mkfifo(temp_c.as_ptr(), 0o600) };
    assert_eq!(created, 0, "mkfifo failed");

    let writer = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || store.upsert(client(CLIENT_PUB, "Test Phone Renamed")))
    };

    // Old sequence: the target disappears before the writer blocks on the
    // FIFO, so this window is stable and observable. New sequence: the target
    // is never touched before the rename-over, so it stays present for the
    // whole wait.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut observed_missing = false;
    while Instant::now() < deadline {
        if !path.exists() {
            observed_missing = true;
            break;
        }
        if writer.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    let reload = store.reload_if_changed();
    let authorization = store.authorize(&unhex(CLIENT_PUB));

    // Unblock the writer so a failing assertion cannot leak a blocked thread.
    let reader = {
        let temp = temp.clone();
        std::thread::spawn(move || {
            let mut fifo = std::fs::File::open(temp).expect("open fifo read end");
            let mut sink = std::io::sink();
            let _ = std::io::copy(&mut fifo, &mut sink);
        })
    };
    let _ = writer.join();
    let _ = reader.join();

    assert!(
        !observed_missing,
        "the live store disappeared while a replacement was in flight; a concurrent reload would clear authorizations"
    );
    assert!(
        reload.is_ok(),
        "the in-flight replacement must not make the reload fail: {reload:?}"
    );
    assert!(
        matches!(
            authorization,
            v2_transport::auth_store::Authorization::Authorized(_)
        ),
        "the reader observed the in-flight replacement as a missing/deleted store"
    );
}
