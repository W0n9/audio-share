//! Authentication primitives (pairing.md §9.1, PR-9; spec §4.4 validation).
//!
//! Pure functions: the TLV `auth_context` derivation, ECDSA-P256-SHA256
//! verification over the RAW context bytes (exactly one hash — implementations
//! MUST NOT pre-hash and pass the digest to a SHA256withECDSA-style API), the
//! §8.1 `client_id` derivation, and the capability-list validation rules.
//! Signing is never implemented server-side; the Server verifies.

use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use sha2::Digest as _;

/// The proof-input label for the §6.3.1 pairing client_proof.
pub const PAIRING_PROOF_LABEL: &[u8] = b"audio-share v2 pairing confirm";

/// A TLV element: `tag(u8) || len(u16 BE) || value` (pairing.md §6.3).
pub fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + value.len());
    out.push(tag);
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
    out
}

/// The §9.1 auth context:
/// `TLV(1,"auth") || TLV(2,server_random) || TLV(3,client_public_key)
///  || TLV(4,server_public_key) || TLV(5,protocol_major) || TLV(6,exporter)`.
///
/// `TLV(5, protocol_major)` is the fixed 4-byte big-endian integer (v2 =
/// `00 00 00 02`) — one value-encoding rule covers the pairing and the auth
/// transcripts (pairing.md §6.3, adjudication).
pub fn build_auth_context(
    server_random: &[u8],
    client_public_key: &[u8],
    server_public_key: &[u8],
    auth_exporter: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&tlv(1, b"auth"));
    out.extend_from_slice(&tlv(2, server_random));
    out.extend_from_slice(&tlv(3, client_public_key));
    out.extend_from_slice(&tlv(4, server_public_key));
    out.extend_from_slice(&tlv(5, &crate::codes::PROTOCOL_MAJOR.to_be_bytes()));
    out.extend_from_slice(&tlv(6, auth_exporter));
    out
}

/// The §6.3.1 pairing proof input: `TLV(1, label) || TLV(2, binding)`.
pub fn build_pairing_proof_input(binding: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + PAIRING_PROOF_LABEL.len() + 3 + binding.len());
    out.extend_from_slice(&tlv(1, PAIRING_PROOF_LABEL));
    out.extend_from_slice(&tlv(2, binding));
    out
}

/// The §8.1 client identity: first 8 bytes of SHA-256(client_public_key).
pub fn client_id(client_public_key: &[u8]) -> [u8; 8] {
    let digest: [u8; 32] = sha2::Sha256::digest(client_public_key).into();
    digest[..8].try_into().expect("sha256 is 32 bytes")
}

/// The verdict of one signature verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofVerdict {
    /// The signature verifies over the context with this key.
    Valid,
    /// The key, the context, or the signature bytes are unusable — treated as
    /// an invalid proof (203 territory), never as a crash.
    Invalid,
}

/// Verify one ECDSA-P256-SHA256 r||s signature (fixed 64 bytes, IEEE P1363)
/// over the RAW context bytes with this SEC1-compressed P-256 public key.
///
/// PR-9: exactly one hash — the SHA-256 inside ECDSA-SHA256 runs over the raw
/// bytes; callers pass the raw bytes, never a digest.
pub fn verify_auth_proof(
    client_public_key: &[u8],
    context: &[u8],
    signature: &[u8],
) -> Result<(), ProofVerdict> {
    let key = parse_client_public_key(client_public_key)?;
    // A non-64-byte signature is not an IEEE P1363 r||s — invalid proof.
    let signature = Signature::from_slice(signature).map_err(|_| ProofVerdict::Invalid)?;
    key.verify(context, &signature).map_err(|_| ProofVerdict::Invalid)
}

/// Verify a §6.3.1 pairing client_proof over its proof input.
pub fn verify_pairing_proof(
    client_public_key: &[u8],
    binding: &[u8],
    client_proof: &[u8],
) -> Result<(), ProofVerdict> {
    verify_auth_proof(client_public_key, &build_pairing_proof_input(binding), client_proof)
}

/// Parse a 33-byte SEC1-compressed P-256 public key (pairing.md §2).
pub fn parse_client_public_key(bytes: &[u8]) -> Result<VerifyingKey, ProofVerdict> {
    VerifyingKey::from_sec1_bytes(bytes).map_err(|_| ProofVerdict::Invalid)
}

/// Validate a capabilities list per spec §4.4 (duplicated or not in ascending
/// numeric order → 401). Used for both `Negotiate` and `PairingRequest`.
pub fn validate_capability_list(capabilities: &[u32]) -> Result<(), CapabilityListError> {
    validate_capability_list_bounded(capabilities, u8::MAX as usize)
}

/// The bounded variant: `max_ids` caps the list length (pairing.md §6.3's
/// `count(u8)` bound of 255 for pairing lists). Both bands route through the
/// same check — a list longer than 255 ids cannot be put into canonical form
/// by either message (fixture error-401-pairing-capabilities-over-255).
pub fn validate_capability_list_bounded(
    capabilities: &[u32],
    max_ids: usize,
) -> Result<(), CapabilityListError> {
    if capabilities.len() > max_ids {
        return Err(CapabilityListError::TooMany {
            count: capabilities.len(),
            max: max_ids,
        });
    }
    for pair in capabilities.windows(2) {
        // Strictly ascending: equal = duplicate, less = out of order.
        if pair[0] >= pair[1] {
            return Err(CapabilityListError::NotCanonical {
                earlier: pair[0],
                later: pair[1],
            });
        }
    }
    Ok(())
}

/// A capability-list validation failure (spec §4.4: 401).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityListError {
    /// More ids than the count(u8) canonical form can represent.
    TooMany {
        /// The offered count.
        count: usize,
        /// The applied bound.
        max: usize,
    },
    /// Two adjacent ids are equal (duplicate) or descending.
    NotCanonical {
        /// The earlier id.
        earlier: u32,
        /// The later id.
        later: u32,
    },
}

impl std::fmt::Display for CapabilityListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany { count, max } => {
                write!(f, "{count} capability ids exceed the canonical count(u8) bound {max}")
            }
            Self::NotCanonical { earlier, later } => {
                write!(f, "capability ids {earlier} then {later}: the list must be ascending and unique")
            }
        }
    }
}

impl std::error::Error for CapabilityListError {}

/// The §6.3 canonical form: `count(u8) || for each id in ascending numeric
/// order: id(u32, big-endian)`. Callers validate first — the canonical form
/// exists only for a list that is already unique and ascending.
pub fn canonical_capabilities(capabilities: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 * capabilities.len());
    out.push(capabilities.len() as u8);
    for id in capabilities {
        out.extend_from_slice(&id.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
    const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
    const SERVER_RANDOM: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const EXPORTER: &str = "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";
    /// control.auth-happy-path derived.auth_context_hex.
    const AUTH_CONTEXT: &str = "01000461757468020020000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f03002103d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d04002011b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c05000400000002060020c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";
    /// control.auth-happy-path step 3's recorded signature.
    const VALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060c";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn auth_context_matches_the_fixture_derivation() {
        let built = build_auth_context(
            &unhex(SERVER_RANDOM),
            &unhex(CLIENT_PUB),
            &unhex(SERVER_PUB),
            &unhex(EXPORTER),
        );
        assert_eq!(hex(&built), AUTH_CONTEXT);
        assert_eq!(built.len(), 155);
    }

    #[test]
    fn protocol_major_tlv_is_fixed_four_byte_be() {
        // TLV(5, protocol_major) = 05 00 04 00 00 00 02 (pairing.md §6.3):
        // tag 05, u16 BE length 0004, value 00000002.
        let built = build_auth_context(&[], &[], &[], &[]);
        let needle = [0x05u8, 0x00, 0x04, 0x00, 0x00, 0x00, crate::codes::PROTOCOL_MAJOR as u8];
        assert_eq!(
            built.windows(needle.len()).find(|w| *w == needle).map(|w| w.to_vec()),
            Some(needle.to_vec()),
            "the fixed 4-byte BE major must appear as TLV(5) — never a varint"
        );
    }

    #[test]
    fn client_id_is_first_8_sha256_bytes() {
        // pairing-happy-path-qr's client_id for the fixture client key.
        assert_eq!(hex(&client_id(&unhex(CLIENT_PUB))), "cf7940b474310246");
    }

    #[test]
    fn fixture_signature_verifies_over_raw_context() {
        assert_eq!(
            verify_auth_proof(&unhex(CLIENT_PUB), &unhex(AUTH_CONTEXT), &unhex(VALID_SIG)),
            Ok(())
        );
    }

    #[test]
    fn flipped_signature_byte_fails() {
        let mut bad = unhex(VALID_SIG);
        let last = bad.len() - 1;
        bad[last] ^= 0x01;
        assert_eq!(
            verify_auth_proof(&unhex(CLIENT_PUB), &unhex(AUTH_CONTEXT), &bad),
            Err(ProofVerdict::Invalid)
        );
    }

    #[test]
    fn wrong_length_signatures_fail_without_panic() {
        for len in [0usize, 63, 65, 128] {
            assert_eq!(
                verify_auth_proof(&unhex(CLIENT_PUB), &unhex(AUTH_CONTEXT), &vec![0u8; len]),
                Err(ProofVerdict::Invalid)
            );
        }
    }

    #[test]
    fn sec1_keys_must_be_33_byte_compressed() {
        assert!(parse_client_public_key(&unhex(CLIENT_PUB)).is_ok());
        // Wrong length (32/34 bytes) and a valid-length key whose point is not
        // on the curve are all rejected.
        assert!(parse_client_public_key(&[0x02; 32]).is_err());
        assert!(parse_client_public_key(&[0u8; 34]).is_err());
        let off_curve = {
            let mut v = vec![0x03u8];
            v.extend_from_slice(&[0xffu8; 32]);
            v
        };
        assert!(parse_client_public_key(&off_curve).is_err());
    }

    #[test]
    fn capability_lists_must_be_ascending_and_unique() {
        assert!(validate_capability_list(&[]).is_ok());
        assert!(validate_capability_list(&[7]).is_ok());
        assert!(validate_capability_list(&[1, 2, 3]).is_ok());
        assert_eq!(
            validate_capability_list(&[2, 1]),
            Err(CapabilityListError::NotCanonical { earlier: 2, later: 1 })
        );
        assert_eq!(
            validate_capability_list(&[3, 3]),
            Err(CapabilityListError::NotCanonical { earlier: 3, later: 3 })
        );
    }

    #[test]
    fn pairing_lists_cap_at_255_ids() {
        let ids: Vec<u32> = (1..=255).collect();
        assert!(validate_capability_list_bounded(&ids, 255).is_ok());
        let mut ids_256 = ids.clone();
        ids_256.push(256);
        assert!(validate_capability_list_bounded(&ids_256, 255).is_err());
        // validate_capability_list (u8::MAX bound) also rejects 256 — the
        // >255 rule protects the count(u8) canonical form for both bands.
        assert!(validate_capability_list(&ids_256).is_err());
        assert!(validate_capability_list(&ids).is_ok());
    }

    #[test]
    fn canonical_form_is_count_u8_then_ids_u32_be() {
        assert_eq!(canonical_capabilities(&[]), vec![0u8]);
        assert_eq!(canonical_capabilities(&[7]), vec![1u8, 0, 0, 0, 7]);
        assert_eq!(
            canonical_capabilities(&[1, 2]),
            vec![2u8, 0, 0, 0, 1, 0, 0, 0, 2]
        );
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
