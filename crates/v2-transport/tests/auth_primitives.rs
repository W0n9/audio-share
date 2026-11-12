use v2_transport::auth;

// Fixture constants (fixtures/wire-protocol-v2/test-keys.json, control/auth-happy-path.json).
const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
const AUTH_SERVER_RANDOM: &str =
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const AUTH_EXPORTER: &str =
    "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";
/// The recorded auth_context (control/auth-happy-path.json derived block).
const AUTH_CONTEXT: &str = "01000461757468020020000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f03002103d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d04002011b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c05000400000002060020c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";
/// The valid happy-path signature (r||s 64 bytes).
const VALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060c";
/// error-203-authentication-failed's corrupted signature (last byte flipped).
const INVALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060b";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn auth_context_tlv_matches_fixture_derivation() {
    // pairing.md §9.1: TLV(1,'auth') || TLV(2,server_random) || TLV(3,client_pubkey)
    // || TLV(4,server_pubkey) || TLV(5,protocol_major u32 BE) || TLV(6,exporter).
    let built = auth::build_auth_context(
        &unhex(AUTH_SERVER_RANDOM),
        &unhex(CLIENT_PUB),
        &unhex(SERVER_PUB),
        &unhex(AUTH_EXPORTER),
    );
    assert_eq!(built, unhex(AUTH_CONTEXT));
    assert_eq!(built.len(), 155); // fixture: auth_context_bytes = 155
}

#[test]
fn client_id_is_first_8_of_sha256_public_key() {
    // pairing.md §8.1; fixture pairing-happy-path-qr: client_id cf7940b474310246.
    let client_id = auth::client_id(&unhex(CLIENT_PUB));
    assert_eq!(client_id.to_vec(), unhex("cf7940b474310246"));
}

#[test]
fn valid_proof_verifies_over_raw_auth_context() {
    // PR-9: exactly one hash — the ECDSA-SHA256 over the RAW auth_context bytes.
    let verdict = auth::verify_auth_proof(
        &unhex(CLIENT_PUB),
        &unhex(AUTH_CONTEXT),
        &unhex(VALID_SIG),
    );
    assert_eq!(verdict, Ok(()));
}

#[test]
fn corrupted_proof_fails_verification() {
    // error-203: the flipped-byte signature MUST NOT verify.
    let verdict = auth::verify_auth_proof(
        &unhex(CLIENT_PUB),
        &unhex(AUTH_CONTEXT),
        &unhex(INVALID_SIG),
    );
    assert!(verdict.is_err(), "the corrupted 203 signature must not verify");
}

#[test]
fn malformed_proof_shapes_are_rejected_without_verification() {
    // A signature that is not exactly 64 bytes can never be a P1363 r||s.
    assert!(auth::verify_auth_proof(&unhex(CLIENT_PUB), &unhex(AUTH_CONTEXT), &[0u8; 63]).is_err());
    assert!(auth::verify_auth_proof(&unhex(CLIENT_PUB), &unhex(AUTH_CONTEXT), &[0u8; 65]).is_err());
}

#[test]
fn public_key_must_parse_as_sec1_compressed_p256() {
    assert!(auth::parse_client_public_key(&unhex(CLIENT_PUB)).is_ok());
    // Wrong length.
    assert!(auth::parse_client_public_key(&[0x02; 32]).is_err());
    assert!(auth::parse_client_public_key(&[0u8; 34]).is_err());
    // Compressed-point prefix with an x-coordinate that is not on the curve.
    let off_curve = {
        let mut v = vec![0x03u8];
        v.extend_from_slice(&[0xffu8; 32]);
        v
    };
    assert!(auth::parse_client_public_key(&off_curve).is_err());
}

#[test]
fn capabilities_must_be_ascending_and_unique() {
    // spec §4.4 validation table: duplicated or not ascending → 401.
    assert!(auth::validate_capability_list(&[]).is_ok());
    assert!(auth::validate_capability_list(&[7]).is_ok());
    assert!(auth::validate_capability_list(&[1, 2, 3]).is_ok());
    assert!(auth::validate_capability_list(&[2, 1]).is_err(), "descending is malformed");
    assert!(auth::validate_capability_list(&[3, 3]).is_err(), "duplicates are malformed");
}

#[test]
fn canonical_capabilities_is_count_u8_then_ids_u32_be() {
    // pairing.md §6.3: count(u8) || for each id ascending: id(u32 BE).
    assert_eq!(auth::canonical_capabilities(&[]), vec![0u8]);
    assert_eq!(
        auth::canonical_capabilities(&[7]),
        vec![1u8, 0, 0, 0, 7]
    );
    // The single-id list from control.negotiate-unknown-capability-ignored: 01 00000007.
    assert_eq!(auth::canonical_capabilities(&[1, 2]), vec![2u8, 0, 0, 0, 1, 0, 0, 0, 2]);
}

#[test]
fn pairing_capability_lists_cap_at_255_ids() {
    // pairing.md §6.3: count(u8) bounds the list at 255; 256 ids → malformed
    // (fixture error-401-pairing-capabilities-over-255). The bound protects
    // the canonical form for both pairing and Negotiate lists.
    let ids: Vec<u32> = (1..=255).collect();
    assert!(auth::validate_capability_list_bounded(&ids, 255).is_ok());
    let mut ids_256 = ids.clone();
    ids_256.push(256);
    assert!(auth::validate_capability_list_bounded(&ids_256, 255).is_err());
    assert!(auth::validate_capability_list(&ids_256).is_err());
    assert!(auth::validate_capability_list(&ids).is_ok());
}
