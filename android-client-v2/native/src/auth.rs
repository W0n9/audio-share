//! Authentication signing side (pairing.md §9.1 / PR-9) — host-tested.
//!
//! auth_context = TLV(1,"auth") || TLV(2,server_random) || TLV(3,client_pubkey)
//!             || TLV(4,server_pubkey) || TLV(5,protocol_major)
//!             || TLV(6,auth_exporter)
//! signature   = ECDSA-P256-SHA256(client_identity_key, auth_context),
//!               fixed 64-byte r||s (IEEE P1363)
//!
//! Exactly ONE hash (adjudication 2026-11-12): SHA-256 inside ECDSA-SHA256
//! over the RAW auth_context bytes. Never pre-hash and hand the digest to a
//! SHA256withECDSA-style API.
//!
//! TLV value encodings (pairing.md §6.3, normative, one rule for auth and
//! pairing transcripts): protocol_major = fixed 4-byte big-endian; everything
//! else = raw wire bytes. TLV(tag, value) = tag(u8) || len(u16 BE) || value.
//!
//! The Server identity key is Ed25519 (PR-1); its raw 32-byte public key is
//! what goes into TLV(4). The Client identity key is ECDSA P-256 (PR-2).

use crate::proto::{frame::Body, AuthProof, Frame};
#[cfg(test)]
use prost::Message as _;
use p256::ecdsa::{
    signature::{Signer, Verifier},
    Signature, SigningKey, VerifyingKey,
};

/// TLV(tag, value) = tag(u8) || len(u16, big-endian) || value.
///
/// The length field is a checked `u16`: a value that cannot be represented is
/// an error rather than a silently truncated transcript.
pub fn tlv(tag: u8, value: &[u8]) -> Result<Vec<u8>, String> {
    let len = u16::try_from(value.len())
        .map_err(|_| format!("TLV value is {} bytes, max 65535", value.len()))?;
    let mut out = Vec::with_capacity(3 + value.len());
    out.push(tag);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(value);
    Ok(out)
}

/// Build the raw auth_context bytes (pairing.md §9.1).
pub fn auth_context(
    server_random: &[u8],
    client_public_key: &[u8],
    server_public_key: &[u8],
    protocol_major: u32,
    auth_exporter: &[u8],
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.extend_from_slice(&tlv(1, b"auth")?);
    out.extend_from_slice(&tlv(2, server_random)?);
    out.extend_from_slice(&tlv(3, client_public_key)?);
    out.extend_from_slice(&tlv(4, server_public_key)?);
    // TLV(5, protocol_major): fixed 4-byte BE — never a varint, never 1 byte.
    out.extend_from_slice(&tlv(5, &protocol_major.to_be_bytes())?);
    out.extend_from_slice(&tlv(6, auth_exporter)?);
    Ok(out)
}

/// Sign the raw auth_context bytes with the identity key: one ECDSA-P256-SHA256
/// signature, fixed 64-byte r||s (IEEE P1363).
pub fn sign_auth_context(key: &SigningKey, auth_context: &[u8]) -> [u8; 64] {
    let sig: Signature = key.sign(auth_context);
    sig.to_bytes().into()
}

/// Verify a recorded 64-byte r||s signature over the raw auth_context bytes.
/// Verification is deterministic — this is how the golden vectors are replayed
/// (implementations never reproduce signatures, fixtures README contract item 5).
pub fn verify_auth_signature(
    client_public_key: &VerifyingKey,
    auth_context: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    if signature.len() != 64 {
        return Err(format!("signature must be 64-byte r||s, got {}", signature.len()));
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(signature);
    let sig = Signature::from_slice(&sig).map_err(|e| format!("malformed signature: {e}"))?;
    // ECDSA-SHA256 hashes the raw context exactly once inside `verify`.
    client_public_key
        .verify(auth_context, &sig)
        .map_err(|_| "signature verification failed".to_string())
}

/// TEST-ONLY client identity key (fixtures test-keys.json private scalar).
///
/// Gated behind `cfg(any(test, feature = "test-support"))`: production builds
/// have no way to reach it. The E2E APK is built with `--features
/// test-support`; the v2 pairing flow (phase 6) supplies the real key.
#[cfg(any(test, feature = "test-support"))]
pub fn test_client_identity_key() -> SigningKey {
    // scalar from fixtures/wire-protocol-v2/test-keys.json
    let d_hex = "87fb33cdf19dc9ec6324570648e8f354d26d5262e4930182382220bec2fdfac7";
    let mut d = [0u8; 32];
    for i in 0..32 {
        d[i] = u8::from_str_radix(&d_hex[2 * i..2 * i + 2], 16).unwrap();
    }
    SigningKey::from_bytes(&d.into()).expect("fixture test key is a valid scalar")
}

/// SEC1 compressed 33-byte encoding of a P-256 public key (the wire form).
pub fn sec1_compressed(key: &SigningKey) -> Vec<u8> {
    key.verifying_key().to_encoded_point(true).as_bytes().to_vec()
}

/// Build the AuthProof frame for a received AuthChallenge. The identity key
/// material and the exporter value are session state, supplied by the caller.
pub fn proof_for_challenge(
    challenge: &crate::proto::AuthChallenge,
) -> Result<Frame, String> {
    // The session layer supplies key + exporter; the machine-level helper
    // cannot sign without them. This shim exists so control.rs compiles and
    // is replaced by the session's real signing path.
    let _ = challenge;
    Err("not wired".to_string())
}

/// Build a complete AuthProof frame from a challenge + session context.
pub fn build_auth_proof(
    request_id: u64,
    server_random: &[u8],
    server_public_key: &[u8],
    protocol_major: u32,
    auth_exporter: &[u8],
    key: &SigningKey,
) -> Result<Frame, String> {
    let client_public_key = sec1_compressed(key);
    let ctx = auth_context(
        server_random,
        &client_public_key,
        server_public_key,
        protocol_major,
        auth_exporter,
    )?;
    let sig = sign_auth_context(key, &ctx);
    Ok(Frame {
        body: Some(Body::AuthProof(AuthProof {
            request_id,
            signature: sig.to_vec(),
        })),
    })
}

// Fixture constants (fixtures/wire-protocol-v2/test-keys.json).
#[cfg(test)]
const CLIENT_PUB_HEX: &str =
    "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
#[cfg(test)]
const SERVER_PUB_HEX: &str =
    "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
#[cfg(test)]
const SERVER_RANDOM_HEX: &str =
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
#[cfg(test)]
const AUTH_EXPORTER_HEX: &str =
    "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";
/// Golden vector control.auth-happy-path, derived.auth_context_hex.
#[cfg(test)]
const AUTH_CONTEXT_HEX: &str = "01000461757468020020000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f03002103d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d04002011b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c05000400000002060020c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";
/// The recorded AuthProof signature (control.auth-happy-path step 3).
#[cfg(test)]
const RECORDED_SIG_HEX: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060c";

// ---- test-support constants (cfg(test) only; never shipped) ----------------

#[cfg(test)]
pub(crate) fn tests_const_server_pub() -> Vec<u8> {
    unhex_const(SERVER_PUB_HEX)
}

#[cfg(test)]
pub(crate) fn tests_const_exporter() -> Vec<u8> {
    unhex_const(AUTH_EXPORTER_HEX)
}

#[cfg(test)]
fn unhex_const(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// Pairing proof input (pairing.md §6.3.1):
/// TLV(1, "audio-share v2 pairing confirm") || TLV(2, binding).
pub fn pairing_tlv(domain_label: &[u8], binding: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = tlv(1, domain_label)?;
    out.extend_from_slice(&tlv(2, binding)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::AuthInit;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn tlv_layout_is_tag_len_value() {
        assert_eq!(hex(&tlv(1, b"auth").unwrap()), "01000461757468");
        // 4-byte BE protocol major (pairing.md §6.3 normative encoding)
        assert_eq!(hex(&tlv(5, &2u32.to_be_bytes()).unwrap()), "05000400000002");
    }

    #[test]
    fn tlv_rejects_values_that_do_not_fit_the_length_field() {
        let too_long = vec![0u8; 65_536];
        let err = tlv(1, &too_long).unwrap_err();
        assert!(err.contains("max 65535"), "got: {err}");
    }

    /// The rebuilt auth_context must equal the golden vector's recorded
    /// derivation byte for byte (control.auth-happy-path derived.auth_context_hex).
    #[test]
    fn auth_context_matches_golden_vector() {
        let ctx = auth_context(
            &unhex(SERVER_RANDOM_HEX),
            &unhex(CLIENT_PUB_HEX),
            &unhex(SERVER_PUB_HEX),
            2,
            &unhex(AUTH_EXPORTER_HEX),
        )
        .unwrap();
        assert_eq!(hex(&ctx), AUTH_CONTEXT_HEX);
        assert_eq!(ctx.len(), 155);
    }

    /// The fixture key's SEC1 compressed public key matches test-keys.json.
    #[test]
    fn fixture_key_public_matches_test_keys() {
        let key = test_client_identity_key();
        assert_eq!(hex(&sec1_compressed(&key)), CLIENT_PUB_HEX);
    }

    /// Contract item 5 (fixtures README): implementations VERIFY the recorded
    /// signature against the recorded inputs — never reproduce it. The
    /// verification must pass against the fixture-fixed identity key.
    #[test]
    fn recorded_golden_signature_verifies() {
        let key = test_client_identity_key();
        let vk = *key.verifying_key();
        let ctx = unhex(AUTH_CONTEXT_HEX);
        verify_auth_signature(&vk, &ctx, &unhex(RECORDED_SIG_HEX))
            .expect("golden auth signature must verify");
    }

    /// A tampered context (one bit flipped in server_random) must fail.
    #[test]
    fn tampered_auth_context_fails_verification() {
        let key = test_client_identity_key();
        let vk = *key.verifying_key();
        let mut ctx = unhex(AUTH_CONTEXT_HEX);
        ctx[10] ^= 0x01;
        assert!(verify_auth_signature(&vk, &ctx, &unhex(RECORDED_SIG_HEX)).is_err());
    }

    /// Wrong-length signature rejected with a clear message.
    #[test]
    fn short_signature_is_rejected() {
        let key = test_client_identity_key();
        let vk = *key.verifying_key();
        let err = verify_auth_signature(&vk, &unhex(AUTH_CONTEXT_HEX), &[0u8; 63]).unwrap_err();
        assert!(err.contains("64-byte"), "got: {err}");
    }

    /// Signing side end-to-end: build_auth_proof produces an AuthProof whose
    /// signature verifies over the rebuilt context (sign/verify symmetry).
    #[test]
    fn build_auth_proof_round_trips() {
        let key = test_client_identity_key();
        let proof = build_auth_proof(
            1,
            &unhex(SERVER_RANDOM_HEX),
            &unhex(SERVER_PUB_HEX),
            2,
            &unhex(AUTH_EXPORTER_HEX),
            &key,
        )
        .unwrap();
        let Some(Body::AuthProof(p)) = proof.body else {
            panic!("expected auth_proof body");
        };
        assert_eq!(p.request_id, 1);
        assert_eq!(p.signature.len(), 64);
        let ctx = auth_context(
            &unhex(SERVER_RANDOM_HEX),
            &unhex(CLIENT_PUB_HEX),
            &unhex(SERVER_PUB_HEX),
            2,
            &unhex(AUTH_EXPORTER_HEX),
        )
        .unwrap();
        verify_auth_signature(key.verifying_key(), &ctx, &p.signature)
            .expect("own signature must verify");
    }

    /// The AuthInit frame built from the fixture key matches the golden
    /// vector's body_hex exactly (byte-level, fixtures README item 2).
    #[test]
    fn auth_init_encoding_matches_golden_vector() {
        let key = test_client_identity_key();
        let init = Frame {
            body: Some(Body::AuthInit(AuthInit {
                request_id: 1,
                client_public_key: sec1_compressed(&key),
            })),
        };
        let body = init.encode_to_vec();
        assert_eq!(
            hex(&body),
            "0a2c090100000000000000122103d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d"
        );
    }
}
