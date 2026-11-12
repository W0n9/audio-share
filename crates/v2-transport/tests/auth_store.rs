//! The §8.1 server-side authorization store: `pairing/clients.json` — public
//! keys and metadata only, no secrets. Test-only credential preloading hands
//! the Server its authorized-Client list (ticket 07: production pairing is a
//! later phase; pairing.md §11.3 keeps bypasses in test builds).

use std::path::PathBuf;

use v2_transport::auth_store::{AuthorizationStore, ClientState, StoredClient};

fn temp_dir() -> PathBuf {
    tempfile::tempdir()
        .expect("tempdir")
        .path()
        .to_path_buf()
}

fn client(pub_hex: &str, name: &str) -> StoredClient {
    StoredClient {
        client_id: hex8(pub_hex),
        public_key: unhex(pub_hex),
        display_name: name.to_owned(),
        state: ClientState::Active,
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
const OTHER_PUB: &str = "021111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn unknown_identity_classifies_202() {
    let store = AuthorizationStore::in_memory();
    let verdict = store.authorize(&unhex(CLIENT_PUB));
    assert!(matches!(verdict, v2_transport::auth_store::Authorization::Unknown(_)));
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
        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "clients.json must be 0600, got {:o}", mode);
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
    // pairing.md §10.3: atomic write + one backup generation. The helper's
    // backup name is `<stem>.bak` (clients.bak next to clients.json).
    let dir = temp_dir();
    let store = AuthorizationStore::open(&dir).expect("open");
    store.insert_for_tests(client(CLIENT_PUB, "Test Phone"));
    store.insert_for_tests(client(OTHER_PUB, "Second"));
    let backup = dir.join("pairing").join("clients.bak");
    assert!(backup.is_file(), "one .bak generation must be kept");
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
