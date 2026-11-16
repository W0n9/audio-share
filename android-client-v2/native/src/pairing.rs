//! Client-side pairing primitives (pairing.md §6.3, §7, §8).
//!
//! Pure, transport-agnostic helpers shared by the state machine, the session
//! layer, and the interop harness:
//! - the §6.3 binding-context TLV canonicalization and SHA-256;
//! - the §7.1 SAS derivation and §7.3 presentation;
//! - the §7.4 Server SPKI fingerprint;
//! - the pair of TLS-Exporter labels;
//! - the `PairingConfirm` proof construction, signed through the identity
//!   signing seam (a Keystore key never leaves Android).
//!
//! Red line: this module never logs nonces, SAS values, bindings, or keys.

use std::sync::Arc;

use crate::proto::{frame::Body, Frame, PairingConfirm, PairingRequest};
use crate::signer::IdentitySigner;

/// TLS-Exporter label for `binding` (48 bytes; pairing.md §6.3).
pub const PAIRING_EXPORTER_LABEL: &[u8] = b"EXPORTER audio-share v2 pairing";
/// TLS-Exporter label for `sas_seed` (32 bytes; pairing.md §6.3).
pub const SAS_EXPORTER_LABEL: &[u8] = b"EXPORTER audio-share v2 sas";
/// The wire protocol major carried in the pairing transcript.
pub const PROTOCOL_MAJOR: u32 = 2;
/// The binding length.
pub const BINDING_LEN: usize = 48;
/// The SAS seed length.
pub const SAS_SEED_LEN: usize = 32;
/// The pairing proof domain label (pairing.md §6.3.1).
pub const PAIRING_PROOF_LABEL: &[u8] = b"audio-share v2 pairing confirm";
/// Fixed 12-byte Ed25519 SubjectPublicKeyInfo algorithm prefix.
pub const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// The Client-side pairing mode (wire values 1/2; pairing.md §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientPairingMode {
    /// QR payload supplied an out-of-band nonce and pinned Server key.
    Qr,
    /// SAS fallback: unpinned first connection, human comparison.
    Sas,
}

impl ClientPairingMode {
    /// The protobuf enum value.
    pub fn wire_value(self) -> i32 {
        match self {
            ClientPairingMode::Qr => 1,
            ClientPairingMode::Sas => 2,
        }
    }

    /// The lowercase ASCII mode string committed into the transcript.
    pub fn as_str(self) -> &'static str {
        match self {
            ClientPairingMode::Qr => "qr",
            ClientPairingMode::Sas => "sas",
        }
    }

    /// Parse the wire enum (1/2); anything else is invalid.
    pub fn from_wire(value: i32) -> Result<Self, String> {
        match value {
            1 => Ok(ClientPairingMode::Qr),
            2 => Ok(ClientPairingMode::Sas),
            other => Err(format!("unknown pairing mode {other}")),
        }
    }
}

/// The pairing exchange phase, exposed to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingPhase {
    /// No pairing exchange started.
    Idle,
    /// `PairingRequest` sent; awaiting `PairingResponse`.
    AwaitingResponse,
    /// Response verified; SAS/fingerprint shown; awaiting the user's confirm.
    AwaitingConfirmation,
    /// `PairingConfirm` sent; awaiting `PairingResult`.
    Confirming,
    /// `PairingResult(OK)`; the Server persisted this identity.
    Complete,
    /// Pairing failed or was cancelled.
    Failed,
}

impl PairingPhase {
    /// Stable name for the JNI/UI surface.
    pub fn name(self) -> &'static str {
        match self {
            PairingPhase::Idle => "IDLE",
            PairingPhase::AwaitingResponse => "AWAITING_RESPONSE",
            PairingPhase::AwaitingConfirmation => "AWAITING_CONFIRMATION",
            PairingPhase::Confirming => "CONFIRMING",
            PairingPhase::Complete => "COMPLETE",
            PairingPhase::Failed => "FAILED",
        }
    }
}

/// Display material and outcome for one pairing exchange. No secrets beyond
/// the SAS/fingerprint that the user must see (never logged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingInfo {
    /// The pairing mode.
    pub mode: ClientPairingMode,
    /// The current phase.
    pub phase: PairingPhase,
    /// The §7.3 SAS as `123 456` once the response is verified.
    pub sas: Option<String>,
    /// The §7.4 Server SPKI fingerprint.
    pub fingerprint: Option<String>,
    /// The Server's raw Ed25519 identity key (hex).
    pub server_public_key_hex: Option<String>,
    /// The pin the Client must persist (SHA-256 SPKI, hex).
    pub server_spki_sha256_hex: Option<String>,
    /// The Server-assigned canonical client id.
    pub client_id: Option<String>,
    /// The accepted display name.
    pub display_name: Option<String>,
    /// A §6 error code when pairing failed.
    pub error_code: Option<u32>,
}

impl PairingInfo {
    /// The pre-response state.
    pub fn awaiting(mode: ClientPairingMode) -> Self {
        PairingInfo {
            mode,
            phase: PairingPhase::AwaitingResponse,
            sas: None,
            fingerprint: None,
            server_public_key_hex: None,
            server_spki_sha256_hex: None,
            client_id: None,
            display_name: None,
            error_code: None,
        }
    }
}

/// The credential summary exposed after `PairingResult(OK)` so Kotlin can
/// persist it. Public data only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedCredential {
    /// The pin to persist (SHA-256 SPKI, lowercase hex).
    pub server_spki_sha256_hex: String,
    /// The Server's raw Ed25519 identity public key (hex).
    pub server_public_key_hex: String,
    /// The canonical `client_id`.
    pub client_id: String,
    /// The accepted display name.
    pub display_name: String,
}

/// Per-connection TLS-Exporter seam. The session implements this over
/// `quinn::Connection::export_keying_material`; tests inject fixed stand-ins.
pub trait PairingExporter: Send + Sync {
    /// Export `len` bytes for `label` bound to `context`.
    fn export(&self, label: &[u8], context: &[u8], len: usize) -> Result<Vec<u8>, String>;
}

/// A shared exporter handle.
pub type SharedExporter = Arc<dyn PairingExporter>;

/// An exporter backed by a closure (tests and adapters).
pub struct FnExporter<F>(pub F);

impl<F> std::fmt::Debug for FnExporter<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnExporter").finish()
    }
}

impl<F> PairingExporter for FnExporter<F>
where
    F: Fn(&[u8], &[u8], usize) -> Result<Vec<u8>, String> + Send + Sync,
{
    fn export(&self, label: &[u8], context: &[u8], len: usize) -> Result<Vec<u8>, String> {
        (self.0)(label, context, len)
    }
}

/// TLV(tag, value) = tag(u8) || len(u16 BE) || value.
fn tlv(tag: u8, value: &[u8]) -> Result<Vec<u8>, String> {
    crate::auth::tlv(tag, value)
}

/// Validate and canonicalize the capability list: unique, ascending, ≤ 255
/// ids, encoded as `count(u8) || id(u32 BE)…` (pairing.md §6.3).
pub fn capabilities_canonical(capabilities: &[u32]) -> Result<Vec<u8>, String> {
    if capabilities.len() > u8::MAX as usize {
        return Err("more than 255 capabilities".to_string());
    }
    let mut previous: Option<u32> = None;
    for &id in capabilities {
        if let Some(prev) = previous {
            if id <= prev {
                return Err("capabilities must be unique and ascending".to_string());
            }
        }
        previous = Some(id);
    }
    let mut out = Vec::with_capacity(1 + capabilities.len() * 4);
    out.push(capabilities.len() as u8);
    for id in capabilities {
        out.extend_from_slice(&id.to_be_bytes());
    }
    Ok(out)
}

/// The §6.3 inputs to the binding transcript.
#[derive(Debug, Clone)]
pub struct BindingInputs<'a> {
    /// `qr`/`sas`.
    pub mode: ClientPairingMode,
    /// The QR nonce (empty in SAS mode).
    pub pairing_nonce: &'a [u8],
    /// The Client's 32-byte CSPRNG value.
    pub client_random: &'a [u8],
    /// The Server's 32-byte CSPRNG value.
    pub server_random: &'a [u8],
    /// The Client's 33-byte SEC1 public key.
    pub client_public_key: &'a [u8],
    /// The Server's raw 32-byte Ed25519 key.
    pub server_public_key: &'a [u8],
    /// The offered capabilities (ascending, unique).
    pub capabilities: &'a [u32],
    /// The accepted display name.
    pub client_display_name: &'a str,
}

/// Build `SHA-256(TLV 1..9)` (pairing.md §6.3).
pub fn binding_context(inputs: &BindingInputs<'_>) -> Result<[u8; 32], String> {
    match inputs.mode {
        ClientPairingMode::Qr => {
            if inputs.pairing_nonce.len() != 16 {
                return Err("QR pairing_nonce must be 16 bytes".to_string());
            }
        }
        ClientPairingMode::Sas => {
            if !inputs.pairing_nonce.is_empty() {
                return Err("SAS mode must omit pairing_nonce".to_string());
            }
        }
    }
    if inputs.client_random.len() != 32 {
        return Err("client_random must be 32 bytes".to_string());
    }
    if inputs.server_random.len() != 32 {
        return Err("server_random must be 32 bytes".to_string());
    }
    crate::signer::validate_public_key_sec1(inputs.client_public_key)?;
    if inputs.server_public_key.len() != 32 {
        return Err("server_public_key must be 32 bytes".to_string());
    }
    let capabilities = capabilities_canonical(inputs.capabilities)?;

    let mut transcript = Vec::new();
    transcript.extend(tlv(1, inputs.mode.as_str().as_bytes())?);
    // Fixed 4-byte big-endian major, never a varint (pairing.md §6.3).
    transcript.extend(tlv(2, &PROTOCOL_MAJOR.to_be_bytes())?);
    if inputs.mode == ClientPairingMode::Qr {
        transcript.extend(tlv(3, inputs.pairing_nonce)?);
    }
    transcript.extend(tlv(4, inputs.client_random)?);
    transcript.extend(tlv(5, inputs.server_random)?);
    transcript.extend(tlv(6, inputs.client_public_key)?);
    transcript.extend(tlv(7, inputs.server_public_key)?);
    transcript.extend(tlv(8, &capabilities)?);
    transcript.extend(tlv(9, inputs.client_display_name.as_bytes())?);

    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(&transcript).into())
}

/// Derive the binding over the exporter seam.
pub type DerivedBinding = ([u8; 32], [u8; 48], [u8; 32]);

pub fn derive_binding(
    exporter: &dyn PairingExporter,
    inputs: &BindingInputs<'_>,
) -> Result<DerivedBinding, String> {
    let context = binding_context(inputs)?;
    let binding = exporter.export(PAIRING_EXPORTER_LABEL, &context, BINDING_LEN)?;
    let sas_seed = exporter.export(SAS_EXPORTER_LABEL, &context, SAS_SEED_LEN)?;
    let binding: [u8; 48] = binding
        .try_into()
        .map_err(|v: Vec<u8>| format!("binding must be 48 bytes, got {}", v.len()))?;
    let sas_seed: [u8; 32] = sas_seed
        .try_into()
        .map_err(|v: Vec<u8>| format!("sas_seed must be 32 bytes, got {}", v.len()))?;
    Ok((context, binding, sas_seed))
}

/// `SAS = BE u32(sas_seed[0..4]) mod 1_000_000` (§7.1).
pub fn derive_sas_code(sas_seed: &[u8]) -> Result<u32, String> {
    let first = sas_seed
        .get(..4)
        .ok_or_else(|| "sas_seed must be at least 4 bytes".to_string())?;
    let value = u32::from_be_bytes(first.try_into().expect("four bytes"));
    Ok(value % 1_000_000)
}

/// Render a SAS as `123 456` (§7.3).
pub fn format_sas(code: u32) -> String {
    format!("{:03} {:03}", (code / 1000) % 1000, code % 1000)
}

/// The Server SPKI DER for a raw Ed25519 key.
pub fn ed25519_spki(server_public_key: &[u8]) -> Result<Vec<u8>, String> {
    if server_public_key.len() != 32 {
        return Err("server_public_key must be 32 bytes".to_string());
    }
    let mut spki = Vec::with_capacity(44);
    spki.extend_from_slice(&ED25519_SPKI_PREFIX);
    spki.extend_from_slice(server_public_key);
    Ok(spki)
}

/// The pin: SHA-256 of the Server SPKI DER, lowercase hex.
pub fn server_pin_hex(server_public_key: &[u8]) -> Result<String, String> {
    let spki = ed25519_spki(server_public_key)?;
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(&spki).into();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// The §7.4 fingerprint: 8 groups of 8 uppercase hex, space-separated.
pub fn server_fingerprint(server_public_key: &[u8]) -> Result<String, String> {
    let spki = ed25519_spki(server_public_key)?;
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(&spki).into();
    let hex: String = digest.iter().map(|b| format!("{b:02X}")).collect();
    Ok(hex
        .as_bytes()
        .chunks(8)
        .map(|chunk| std::str::from_utf8(chunk).expect("hex is ASCII"))
        .collect::<Vec<_>>()
        .join(" "))
}

/// The §6.3.1 proof input: TLV(1, label) || TLV(2, binding).
pub fn pairing_proof_input(binding: &[u8]) -> Result<Vec<u8>, String> {
    crate::auth::pairing_tlv(PAIRING_PROOF_LABEL, binding)
}

/// Build a `PairingRequest` frame (request_id assigned by the machine).
pub fn pairing_request_frame(
    request_id: u64,
    mode: ClientPairingMode,
    nonce: Vec<u8>,
    client_random: [u8; 32],
    client_public_key: Vec<u8>,
    display_name: String,
) -> Result<Frame, String> {
    match mode {
        ClientPairingMode::Qr if nonce.len() != 16 => {
            return Err("QR pairing_nonce must be 16 bytes".to_string());
        }
        ClientPairingMode::Sas if !nonce.is_empty() => {
            return Err("SAS mode must omit pairing_nonce".to_string());
        }
        _ => {}
    }
    crate::signer::validate_public_key_sec1(&client_public_key)?;
    if display_name.is_empty() || display_name.len() > 63 {
        return Err("display name must be 1..=63 bytes".to_string());
    }
    Ok(Frame {
        body: Some(Body::PairingRequest(PairingRequest {
            request_id,
            pairing_mode: mode.wire_value(),
            pairing_nonce: nonce,
            client_random: client_random.to_vec(),
            client_public_key,
            protocol_major: PROTOCOL_MAJOR,
            capabilities: Vec::new(),
            client_name: display_name,
        })),
    })
}

/// Build a `PairingConfirm` frame with the §6.3.1 proof.
pub fn pairing_confirm_frame(
    request_id: u64,
    binding: &[u8],
    signer: &dyn IdentitySigner,
) -> Result<Frame, String> {
    if binding.len() != BINDING_LEN {
        return Err(format!("binding must be 48 bytes, got {}", binding.len()));
    }
    let input = pairing_proof_input(binding)?;
    // Exactly one ECDSA-SHA256 hash over the raw proof input; the signer may
    // be a Keystore upcall.
    let proof = signer.sign(&input)?;
    Ok(Frame {
        body: Some(Body::PairingConfirm(PairingConfirm {
            request_id,
            binding_echo: binding.to_vec(),
            client_proof: proof.to_vec(),
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // Fixture stand-ins (fixtures/wire-protocol-v2/control/pairing-*.json).
    const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
    const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
    const NONCE: &str = "808182838485868788898a8b8c8d8e8f";
    const CLIENT_RANDOM: &str = "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f";
    const SERVER_RANDOM: &str = "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f";
    const BINDING: &str = "909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf";

    fn inputs<'a>(
        mode: ClientPairingMode,
        nonce: &'a [u8],
        client_random: &'a [u8],
        server_random: &'a [u8],
        client_public_key: &'a [u8],
        server_public_key: &'a [u8],
    ) -> BindingInputs<'a> {
        BindingInputs {
            mode,
            pairing_nonce: nonce,
            client_random,
            server_random,
            client_public_key,
            server_public_key,
            capabilities: &[],
            client_display_name: "Test Phone",
        }
    }

    #[test]
    fn binding_context_matches_the_qr_golden_vector() {
        let (nonce, client_random, server_random, client_pub, server_pub) = (
            unhex(NONCE),
            unhex(CLIENT_RANDOM),
            unhex(SERVER_RANDOM),
            unhex(CLIENT_PUB),
            unhex(SERVER_PUB),
        );
        let hash = binding_context(&inputs(
            ClientPairingMode::Qr,
            &nonce,
            &client_random,
            &server_random,
            &client_pub,
            &server_pub,
        ))
        .unwrap();
        assert_eq!(
            hex(&hash),
            "9006ad9b963641483800cb15d74767762150fa3630ea17dfb4e600d16a32438b"
        );
    }

    #[test]
    fn binding_context_sas_matches_the_golden_vector() {
        let (client_random, server_random, client_pub, server_pub) = (
            unhex(CLIENT_RANDOM),
            unhex(SERVER_RANDOM),
            unhex(CLIENT_PUB),
            unhex(SERVER_PUB),
        );
        let hash = binding_context(&inputs(
            ClientPairingMode::Sas,
            &[],
            &client_random,
            &server_random,
            &client_pub,
            &server_pub,
        ))
        .unwrap();
        assert_eq!(
            hex(&hash),
            "c49ef431c6e6fd122f984f55db2cb06003cb08e07f68a38640ef483e3ad9fadd"
        );
    }

    #[test]
    fn sas_derivation_and_format_match_the_fixture() {
        let seed = unhex("e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
        assert_eq!(format_sas(derive_sas_code(&seed).unwrap()), "900 067");
    }

    #[test]
    fn sas_is_be_u32_mod_one_million() {
        let seed = |value: u32| {
            let mut v = [0u8; 32];
            v[0..4].copy_from_slice(&value.to_be_bytes());
            v
        };
        assert_eq!(derive_sas_code(&seed(1_000_000)).unwrap(), 0);
        assert_eq!(
            format_sas(derive_sas_code(&seed(1_000_001)).unwrap()),
            "000 001"
        );
        assert_eq!(
            format_sas(derive_sas_code(&seed(900_000)).unwrap()),
            "900 000"
        );
        // 0x0DBBA0 = 900_000.
        assert_eq!(
            format_sas(derive_sas_code(&seed(0x0DBBA0)).unwrap()),
            "900 000"
        );
    }

    #[test]
    fn caps_canonical_is_count_then_ascending_be_ids() {
        assert_eq!(hex(&capabilities_canonical(&[]).unwrap()), "00");
        assert_eq!(
            hex(&capabilities_canonical(&[1, 0x0102]).unwrap()),
            "020000000100000102"
        );
        assert!(capabilities_canonical(&[2, 1]).is_err());
        assert!(capabilities_canonical(&[1, 1]).is_err());
        assert!(capabilities_canonical(&vec![1u32; 256]).is_err());
    }

    #[test]
    fn proof_input_matches_the_golden_vector() {
        let binding = unhex(BINDING);
        assert_eq!(
            hex(&pairing_proof_input(&binding).unwrap()),
            "01001e617564696f2d73686172652076322070616972696e6720636f6e6669726d020030909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf"
        );
    }

    #[test]
    fn server_pin_and_fingerprint_match_the_fixture_identity() {
        let key = unhex(SERVER_PUB);
        assert_eq!(
            server_pin_hex(&key).unwrap(),
            "f75d188f6cedd011bc363699ffc247b7c98c9baa3500ce48507225582b881847"
        );
        assert_eq!(
            server_fingerprint(&key).unwrap(),
            "F75D188F 6CEDD011 BC363699 FFC247B7 C98C9BAA 3500CE48 50722558 2B881847"
        );
    }

    #[test]
    fn confirm_signature_verifies_against_the_client_key() {
        use p256::ecdsa::signature::Verifier;
        let signer = crate::signer::SoftwareSigner::new(crate::auth::test_client_identity_key());
        let binding = unhex(BINDING);
        let frame = pairing_confirm_frame(1, &binding, &signer).unwrap();
        let Some(Body::PairingConfirm(confirm)) = frame.body else {
            panic!("expected PairingConfirm");
        };
        assert_eq!(confirm.binding_echo, binding);
        assert_eq!(confirm.client_proof.len(), 64);
        let input = pairing_proof_input(&binding).unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&confirm.client_proof).unwrap();
        signer
            .signing_key()
            .verifying_key()
            .verify(&input, &sig)
            .expect("the proof must verify");
    }

    #[test]
    fn request_frame_rejects_bad_lengths_and_names() {
        let key = unhex(CLIENT_PUB);
        let random = [0u8; 32];
        assert!(pairing_request_frame(
            1,
            ClientPairingMode::Qr,
            vec![0; 15],
            random,
            key.clone(),
            "Phone".into()
        )
        .is_err());
        assert!(pairing_request_frame(
            1,
            ClientPairingMode::Sas,
            vec![0; 16],
            random,
            key.clone(),
            "Phone".into()
        )
        .is_err());
        assert!(pairing_request_frame(
            1,
            ClientPairingMode::Qr,
            vec![0; 16],
            random,
            key.clone(),
            "x".repeat(64)
        )
        .is_err());
        assert!(pairing_request_frame(
            1,
            ClientPairingMode::Qr,
            vec![0; 16],
            random,
            key,
            "Phone".into()
        )
        .is_ok());
    }

    #[test]
    fn request_frame_encoding_matches_the_fixture_body() {
        let mut random = [0u8; 32];
        random.copy_from_slice(&unhex(CLIENT_RANDOM));
        let frame = pairing_request_frame(
            1,
            ClientPairingMode::Qr,
            unhex(NONCE),
            random,
            unhex(CLIENT_PUB),
            "Test Phone".into(),
        )
        .unwrap();
        use prost::Message as _;
        let body = frame.encode_to_vec();
        assert_eq!(
            hex(&body),
            "82017009010000000000000010011a10808182838485868788898a8b8c8d8e8f2220606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f2a2103d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d3002420a546573742050686f6e65"
        );
    }
}
