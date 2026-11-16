//! Client identity signing seam (pairing.md §2 PR-2, §6.3.1, §9.1).
//!
//! The production Client identity private key lives in Android Keystore and is
//! non-exportable. The native crate therefore never owns the production key:
//! it only ever asks an [`IdentitySigner`] for the public key and for a single
//! ECDSA-P256-SHA256 signature over the raw transcript bytes. Exactly one hash
//! applies (the SHA-256 inside ECDSA-SHA256); callers pass the RAW context and
//! must never pre-hash it.
//!
//! Implementations:
//! - [`SoftwareSigner`] — an in-process P-256 key. Host tests, the shared
//!   interop harness, and the documented software-backed fallback
//!   (pairing.md §10.1) use it. It is never the production Android default.
//! - [`JniSigner`] — calls a Kotlin `dev.audioshare.v2.ClientSigner` back over
//!   JNI. The Kotlin object owns the Keystore key; the Rust side only holds a
//!   global reference. Calls happen on tokio worker threads, so each call
//!   attaches the current thread to the JVM first.

use std::sync::Arc;

use p256::ecdsa::{signature::Signer, Signature, SigningKey};

/// A Client identity that can expose its SEC1-compressed public key and sign
/// raw transcript bytes with ECDSA-P256-SHA256.
///
/// `sign` returns the fixed 64-byte `r || s` (IEEE P1363) encoding required on
/// the wire. Implementations MUST hash the raw bytes exactly once.
pub trait IdentitySigner: Send + Sync {
    /// The 33-byte SEC1 compressed P-256 public key (0x02/0x03 prefix).
    fn public_key_sec1(&self) -> Result<Vec<u8>, String>;

    /// Sign the RAW context bytes: one ECDSA-P256-SHA256 signature, 64-byte
    /// `r || s`.
    fn sign(&self, raw_context: &[u8]) -> Result<[u8; 64], String>;
}

/// A software P-256 identity (host tests / documented fallback).
pub struct SoftwareSigner {
    key: SigningKey,
}

impl SoftwareSigner {
    /// Wrap an existing key.
    pub fn new(key: SigningKey) -> Self {
        SoftwareSigner { key }
    }

    /// Generate a fresh random identity.
    pub fn generate() -> Self {
        SoftwareSigner {
            key: SigningKey::random(&mut rand_core::OsRng),
        }
    }

    /// The underlying signing key (host tests only; production code must not
    /// rely on exportability).
    pub fn signing_key(&self) -> &SigningKey {
        &self.key
    }
}

impl IdentitySigner for SoftwareSigner {
    fn public_key_sec1(&self) -> Result<Vec<u8>, String> {
        Ok(self
            .key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec())
    }

    fn sign(&self, raw_context: &[u8]) -> Result<[u8; 64], String> {
        let sig: Signature = self.key.sign(raw_context);
        Ok(sig.to_bytes().into())
    }
}

/// Validate the 33-byte SEC1 compressed public-key shape.
pub fn validate_public_key_sec1(key: &[u8]) -> Result<(), String> {
    if key.len() != 33 {
        return Err(format!(
            "client public key must be 33-byte SEC1, got {}",
            key.len()
        ));
    }
    if key[0] != 0x02 && key[0] != 0x03 {
        return Err("client public key must start with 0x02 or 0x03".to_string());
    }
    Ok(())
}

/// A shared signer handle.
pub type SharedSigner = Arc<dyn IdentitySigner>;

/// TEST-ONLY: the fixture identity behind `test-support` (or unit tests).
#[cfg(any(test, feature = "test-support"))]
pub fn test_signer() -> SharedSigner {
    Arc::new(SoftwareSigner::new(crate::auth::test_client_identity_key()))
}

/// Convert a DER-encoded ECDSA signature to fixed 64-byte `r || s` (P1363).
///
/// This is the exact transformation the Android Keystore path uses
/// (`SHA256withECDSA` emits DER): strip the SEQUENCE, read two INTEGERs,
/// left-pad each to 32 big-endian bytes. It rejects non-minimal or oversized
/// integers rather than silently truncating.
pub fn der_to_p1363(der: &[u8]) -> Result<[u8; 64], String> {
    if der.len() < 2 || der[0] != 0x30 {
        return Err("ECDSA DER: missing SEQUENCE".to_string());
    }
    let (seq_len, header) = der_length(&der[1..])?;
    let body = &der[1 + header..];
    if seq_len != body.len() {
        return Err(format!(
            "ECDSA DER: sequence length {seq_len} != body {}",
            body.len()
        ));
    }
    let (r, after_r) = dere_int(body)?;
    let (s, after_s) = dere_int(after_r)?;
    if !after_s.is_empty() {
        return Err("ECDSA DER: trailing bytes".to_string());
    }
    let mut out = [0u8; 64];
    copy_int_padded(r, &mut out[..32])?;
    copy_int_padded(s, &mut out[32..])?;
    Ok(out)
}

/// Convert a fixed 64-byte `r || s` signature to DER (host tests and the
/// software path can use this to drive a JCA verifier).
pub fn p1363_to_der(sig: &[u8]) -> Result<Vec<u8>, String> {
    if sig.len() != 64 {
        return Err(format!("signature must be 64 bytes, got {}", sig.len()));
    }
    let mut body = Vec::with_capacity(72);
    body.push(0x02);
    encode_int(&sig[..32], &mut body);
    body.push(0x02);
    encode_int(&sig[32..], &mut body);
    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(0x30);
    encode_len(body.len(), &mut out);
    out.extend_from_slice(&body);
    Ok(out)
}

fn der_length(buf: &[u8]) -> Result<(usize, usize), String> {
    let first = *buf.first().ok_or("ECDSA DER: missing length")? as usize;
    if first < 0x80 {
        return Ok((first, 1));
    }
    let count = first & 0x7f;
    if count == 0 || count > 2 || buf.len() < 1 + count {
        return Err("ECDSA DER: unsupported length".to_string());
    }
    let mut len = 0usize;
    for &b in &buf[1..1 + count] {
        len = (len << 8) | b as usize;
    }
    Ok((len, 1 + count))
}

fn dere_int(buf: &[u8]) -> Result<(&[u8], &[u8]), String> {
    if buf.len() < 2 || buf[0] != 0x02 {
        return Err("ECDSA DER: expected INTEGER".to_string());
    }
    let (len, header) = der_length(&buf[1..])?;
    let start = 1 + header;
    let end = start + len;
    if end > buf.len() {
        return Err("ECDSA DER: truncated INTEGER".to_string());
    }
    let value = &buf[start..end];
    // DER integers are signed; a leading 0x00 is the minimal positive pad.
    let stripped = if value.len() > 1 && value[0] == 0x00 {
        &value[1..]
    } else {
        value
    };
    if stripped.len() > 32 {
        return Err("ECDSA DER: integer wider than 32 bytes".to_string());
    }
    Ok((stripped, &buf[end..]))
}

fn copy_int_padded(value: &[u8], out: &mut [u8]) -> Result<(), String> {
    if value.len() > out.len() {
        return Err("ECDSA DER: integer does not fit 32 bytes".to_string());
    }
    let offset = out.len() - value.len();
    out[offset..].copy_from_slice(value);
    Ok(())
}

fn encode_int(value: &[u8], out: &mut Vec<u8>) {
    let stripped = {
        let mut i = 0;
        while i + 1 < value.len() && value[i] == 0 {
            i += 1;
        }
        &value[i..]
    };
    let needs_pad = stripped.first().is_some_and(|b| b & 0x80 != 0);
    let len = stripped.len() + usize::from(needs_pad);
    encode_len(len, out);
    if needs_pad {
        out.push(0);
    }
    out.extend_from_slice(stripped);
}

fn encode_len(len: usize, out: &mut Vec<u8>) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        out.push(0x81);
        out.push(len as u8);
    }
}

/// A Kotlin `ClientSigner` reached over JNI.
///
/// The Kotlin object owns the non-exportable Keystore key. The public key is
/// fetched once at construction (on the JNI thread, which is already
/// attached); `sign` attaches the current tokio worker thread before calling
/// back.
pub struct JniSigner {
    vm: jni::JavaVM,
    signer: jni::objects::GlobalRef,
    public_key: Vec<u8>,
}

impl JniSigner {
    /// Build the signer from the Kotlin object and fetch its public key.
    pub fn new(
        env: &mut jni::JNIEnv<'_>,
        signer: &jni::objects::JObject<'_>,
    ) -> Result<Self, String> {
        let vm = env.get_java_vm().map_err(|e| format!("JavaVM: {e}"))?;
        if signer.is_null() {
            return Err("ClientSigner must not be null".to_string());
        }
        let global = env
            .new_global_ref(signer)
            .map_err(|e| format!("ClientSigner global ref: {e}"))?;
        let public_key = call_public_key(env, &global)?;
        validate_public_key_sec1(&public_key)?;
        Ok(JniSigner {
            vm,
            signer: global,
            public_key,
        })
    }
}

impl std::fmt::Debug for JniSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the key or the JVM reference.
        f.debug_struct("JniSigner")
            .field("public_key", &"<redacted>")
            .finish()
    }
}

impl IdentitySigner for JniSigner {
    fn public_key_sec1(&self) -> Result<Vec<u8>, String> {
        Ok(self.public_key.clone())
    }

    fn sign(&self, raw_context: &[u8]) -> Result<[u8; 64], String> {
        let mut env = self
            .vm
            .attach_current_thread()
            .map_err(|e| format!("attach JVM thread: {e}"))?;
        let input = env
            .byte_array_from_slice(raw_context)
            .map_err(|e| format!("sign input: {e}"))?;
        let result = env
            .call_method(self.signer.as_obj(), "sign", "([B)[B", &[(&input).into()])
            .map_err(|e| format!("ClientSigner.sign: {e}"))?;
        let obj = result
            .l()
            .map_err(|e| format!("ClientSigner.sign result: {e}"))?;
        let array = jni::objects::JByteArray::from(obj);
        let bytes = env
            .convert_byte_array(&array)
            .map_err(|e| format!("ClientSigner.sign bytes: {e}"))?;
        if bytes.len() != 64 {
            return Err(format!(
                "ClientSigner.sign must return 64-byte r||s, got {}",
                bytes.len()
            ));
        }
        let mut out = [0u8; 64];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

fn call_public_key(
    env: &mut jni::JNIEnv<'_>,
    signer: &jni::objects::GlobalRef,
) -> Result<Vec<u8>, String> {
    let result = env
        .call_method(signer.as_obj(), "publicKeySec1", "()[B", &[])
        .map_err(|e| format!("ClientSigner.publicKeySec1: {e}"))?;
    let obj = result
        .l()
        .map_err(|e| format!("ClientSigner.publicKeySec1 result: {e}"))?;
    let array = jni::objects::JByteArray::from(obj);
    env.convert_byte_array(&array)
        .map_err(|e| format!("ClientSigner.publicKeySec1 bytes: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;

    fn fixture_signer() -> SoftwareSigner {
        SoftwareSigner::new(crate::auth::test_client_identity_key())
    }

    #[test]
    fn software_signer_public_key_is_sec1_compressed() {
        let signer = fixture_signer();
        let key = signer.public_key_sec1().unwrap();
        assert_eq!(key.len(), 33);
        assert!(key[0] == 0x02 || key[0] == 0x03);
        // Fixture scalar's public key (test-keys.json).
        assert_eq!(
            key.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d"
        );
    }

    #[test]
    fn software_signer_signs_raw_context_with_one_hash() {
        let signer = fixture_signer();
        let context = b"raw pairing context";
        let sig = signer.sign(context).unwrap();
        let vk = *signer.signing_key().verifying_key();
        // ECDSA-SHA256 verify hashes the RAW bytes once.
        let parsed = p256::ecdsa::Signature::from_slice(&sig).unwrap();
        vk.verify(context, &parsed).expect("signature must verify");
        // A pre-hashed variant would fail.
        let digest = {
            use sha2::{Digest, Sha256};
            Sha256::digest(context)
        };
        assert!(vk.verify(&digest, &parsed).is_err());
    }

    #[test]
    fn symbolic_der_round_trip_matches_p1363() {
        let signer = fixture_signer();
        let sig = signer.sign(b"round trip").unwrap();
        let der = p1363_to_der(&sig).unwrap();
        assert_eq!(der_to_p1363(&der).unwrap(), sig);
    }

    #[test]
    fn der_conversion_accepts_leading_zero_padding_and_short_integers() {
        // r = 0x00..01 (leading pad), s = 0x01 (short).
        let der = vec![
            0x30, 0x09, 0x02, 0x01, 0x01, 0x02, 0x04, 0x00, 0x00, 0x00, 0x02,
        ];
        let out = der_to_p1363(&der).unwrap();
        assert_eq!(out[31], 1, "r is right-aligned");
        assert_eq!(out[63], 2, "s is right-aligned");
    }

    #[test]
    fn der_conversion_rejects_oversized_integers() {
        let mut der = vec![0x30, 0x25];
        der.extend_from_slice(&[0x02, 0x21]);
        der.extend_from_slice(&[0x01; 33]);
        der.extend_from_slice(&[0x02, 0x00]);
        assert!(der_to_p1363(&der).is_err());
    }

    #[test]
    fn public_key_validation_rejects_wrong_prefix_and_length() {
        assert!(validate_public_key_sec1(&[0x04; 33]).is_err());
        assert!(validate_public_key_sec1(&[0x02; 32]).is_err());
        assert!(validate_public_key_sec1(&[0x02; 33]).is_ok());
    }

    #[test]
    fn verifying_key_from_sec1_round_trips() {
        let signer = fixture_signer();
        let sec1 = signer.public_key_sec1().unwrap();
        let point = p256::EncodedPoint::from_bytes(&sec1).unwrap();
        let vk = VerifyingKey::from_encoded_point(&point).unwrap();
        let sig = signer.sign(b"context").unwrap();
        let parsed = p256::ecdsa::Signature::from_slice(&sig).unwrap();
        vk.verify(b"context", &parsed).unwrap();
    }
}
