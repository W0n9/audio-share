//! SPKI-pinned server certificate verifier (ticket 08; evolved from the
//! device-validated prototype skeleton).
//!
//! Server identity = sha256 of the server certificate's SubjectPublicKeyInfo
//! DER (spec §3 D7, pairing.md PR-1). The pin comes from the test-only
//! configuration for this ticket (pre-provisioned Server public key — the
//! fixtures' Ed25519 test key derives the recorded pin); production pairing
//! is phase 6. A pin mismatch fails the TLS handshake; the Client classifies
//! it locally as 205 and MUST NOT fall back to v1 or plaintext, MUST NOT
//! retry without user action (pairing.md §8.3).
//!
//! SPKI extraction is a self-contained minimal DER walk (the prototype
//! single-sourced it on proto-core, which is a Server-side crate this crate
//! must not depend on — cross-checking against rcgen's independent
//! serialization stays as the test evidence).

use std::sync::atomic::{AtomicBool, Ordering};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};

#[derive(Debug, PartialEq, Eq)]
pub enum PinError {
    /// truncated or malformed DER
    Malformed,
    /// subjectPublicKeyInfo not found in certificate
    NotFound,
    /// pin must be 64 hex chars
    BadPin,
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinError::Malformed => write!(f, "truncated or malformed DER"),
            PinError::NotFound => write!(f, "subjectPublicKeyInfo not found in certificate"),
            PinError::BadPin => write!(f, "pin must be 64 hex chars"),
        }
    }
}

impl std::error::Error for PinError {}

/// sha256 of the SPKI DER, as raw 32 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpkiPin(pub [u8; 32]);

impl SpkiPin {
    /// Parse the 64-hex-char SPKI sha256 (test config / fingerprint display).
    pub fn from_hex(s: &str) -> Result<Self, PinError> {
        let s = s.trim();
        if s.len() != 64 {
            return Err(PinError::BadPin);
        }
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| PinError::BadPin)?;
        }
        Ok(SpkiPin(out))
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Compute the pin of a DER-encoded X.509 certificate.
    pub fn of_certificate(cert_der: &[u8]) -> Result<Self, PinError> {
        let spki = extract_spki_der(cert_der)?;
        Ok(SpkiPin(spki_sha256(&spki)))
    }
}

/// SHA-256 over the SPKI DER.
pub fn spki_sha256(spki_der: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(spki_der);
    d.into()
}

/// Parsed DER TLV header.
#[derive(Debug, Clone, Copy)]
struct TlvHeader {
    tag: u8,
    content_len: usize,
    header_len: usize,
}

impl TlvHeader {
    fn total_len(&self) -> usize {
        self.header_len + self.content_len
    }
}

/// Parses one DER TLV header, validating that the element fits within `buf`.
fn parse_tlv_header(buf: &[u8]) -> Result<TlvHeader, PinError> {
    if buf.len() < 2 {
        return Err(PinError::Malformed);
    }
    let tag = buf[0];
    let first = buf[1] as usize;
    let (content_len, header_len) = if first < 0x80 {
        (first, 2usize)
    } else {
        let count = first & 0x7f;
        if count == 0 || count > 4 || buf.len() < 2 + count {
            return Err(PinError::Malformed);
        }
        let mut len = 0usize;
        for &b in &buf[2..2 + count] {
            len = (len << 8) | b as usize;
        }
        (len, 2 + count)
    };
    let header = TlvHeader {
        tag,
        content_len,
        header_len,
    };
    if header.total_len() > buf.len() {
        return Err(PinError::Malformed);
    }
    Ok(header)
}

/// Consumes one DER TLV element, returning `(tag, full_element_bytes)`.
fn next_tlv<'a>(buf: &mut &'a [u8]) -> Result<(u8, &'a [u8]), PinError> {
    let header = parse_tlv_header(buf)?;
    let (element, rest) = buf.split_at(header.total_len());
    *buf = rest;
    Ok((header.tag, element))
}

/// Strips the TLV header, returning the element's content octets.
fn element_content(element: &[u8]) -> Result<&[u8], PinError> {
    let header = parse_tlv_header(element)?;
    Ok(&element[header.header_len..header.total_len()])
}

/// Extracts the full DER bytes of the subjectPublicKeyInfo element from an
/// X.509 certificate.
///
/// tbsCertificate child order (RFC 5280): optional [0] version, INTEGER
/// serialNumber, SEQUENCE signature, SEQUENCE issuer, SEQUENCE validity,
/// SEQUENCE subject, SEQUENCE subjectPublicKeyInfo, then optional extensions.
fn extract_spki_der(cert_der: &[u8]) -> Result<Vec<u8>, PinError> {
    let cert_content = {
        let buf = &mut &cert_der[..];
        let (tag, cert) = next_tlv(buf)?; // Certificate ::= SEQUENCE
        if tag != 0x30 {
            return Err(PinError::Malformed);
        }
        element_content(cert)?
    };
    let tbs_content = {
        let buf = &mut &cert_content[..];
        let (tag, tbs) = next_tlv(buf)?; // tbsCertificate ::= SEQUENCE
        if tag != 0x30 {
            return Err(PinError::Malformed);
        }
        element_content(tbs)?
    };
    let buf = &mut &tbs_content[..];
    // optional [0] EXPLICIT version
    if !buf.is_empty() && buf[0] == 0xa0 {
        let _ = next_tlv(buf)?;
    }
    // serialNumber (INTEGER), signature (SEQUENCE), issuer (SEQUENCE),
    // validity (SEQUENCE), subject (SEQUENCE)
    for _ in 0..5 {
        let _ = next_tlv(buf)?;
    }
    let (tag, spki) = next_tlv(buf)?;
    if tag != 0x30 {
        return Err(PinError::NotFound);
    }
    Ok(spki.to_vec())
}

/// The verifier: accepts exactly one SPKI sha256, delegates signature checks
/// to the ring provider's algorithm table.
#[derive(Debug)]
pub struct PinnedSpkiVerifier {
    pin: SpkiPin,
    algorithms: WebPkiSupportedAlgorithms,
    /// Set once a presented certificate's SPKI differs from the pin, so the
    /// session can classify 205 by type instead of matching an error string.
    mismatch: AtomicBool,
}

impl PinnedSpkiVerifier {
    pub fn new(pinned_spki_sha256_hex: &str) -> Result<Self, PinError> {
        Ok(PinnedSpkiVerifier {
            pin: SpkiPin::from_hex(pinned_spki_sha256_hex)?,
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
            mismatch: AtomicBool::new(false),
        })
    }

    pub fn pin(&self) -> &SpkiPin {
        &self.pin
    }

    /// Whether the last observed certificate failed the SPKI pin.
    pub fn pin_mismatch(&self) -> bool {
        self.mismatch.load(Ordering::SeqCst)
    }
}

impl ServerCertVerifier for PinnedSpkiVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let got = SpkiPin::of_certificate(end_entity.as_ref())
            .map_err(|e| TlsError::General(e.to_string()))?;
        if got == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            // never log pins or key material detail beyond the mismatch fact;
            // the typed flag lets the session classify this locally as 205
            // (no fallback, no retry without user action)
            self.mismatch.store(true, Ordering::SeqCst);
            Err(TlsError::General(
                "server SPKI does not match pinned identity".to_string(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn self_signed_cert() -> (rcgen::KeyPair, Vec<u8>) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["audio-share.test".into()]).unwrap();
        params.distinguished_name = rcgen::DistinguishedName::new();
        let cert = params.self_signed(&key).unwrap();
        (key, cert.der().as_ref().to_vec())
    }

    /// Fixture-derived pin: SHA-256 of the Ed25519 test key's 44-byte SPKI
    /// (test-keys.json spki_pin_sha256_hex). The Ed25519 SPKI DER is fixed:
    /// 12-byte algorithm prefix + raw key.
    #[test]
    fn fixture_test_key_pin_matches_recorded_value() {
        let spki = unhex("302a300506032b657003210011b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c");
        assert_eq!(spki.len(), 44);
        let pin = spki_sha256(&spki);
        assert_eq!(
            SpkiPin(pin).to_hex(),
            "f75d188f6cedd011bc363699ffc247b7c98c9baa3500ce48507225582b881847"
        );
        // and it parses back through the hex path
        assert_eq!(
            SpkiPin::from_hex("f75d188f6cedd011bc363699ffc247b7c98c9baa3500ce48507225582b881847")
                .unwrap(),
            SpkiPin(pin)
        );
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// Pin of a certificate is sha256 of its SPKI, cross-checked against
    /// rcgen's independently serialized public key DER.
    #[test]
    fn pin_of_certificate_is_sha256_of_spki() {
        let (key, cert) = self_signed_cert();
        let pin = SpkiPin::of_certificate(&cert).unwrap();
        let expect = spki_sha256(key.public_key_der().as_ref());
        assert_eq!(pin.0, expect);
        let hex = pin.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(SpkiPin::from_hex(&hex).unwrap(), pin);
        drop(key);
    }

    #[test]
    fn pin_from_hex_is_case_insensitive_and_trimmed() {
        let lower = "a".repeat(64);
        let upper = "A".repeat(64);
        assert_eq!(
            SpkiPin::from_hex(&lower).unwrap(),
            SpkiPin::from_hex(&upper).unwrap()
        );
        assert_eq!(
            SpkiPin::from_hex(format!("  {lower}  ").as_str()).unwrap(),
            SpkiPin::from_hex(&lower).unwrap()
        );
    }

    #[test]
    fn pin_from_hex_rejects_bad_input() {
        assert_eq!(SpkiPin::from_hex("abc"), Err(PinError::BadPin));
        assert!(SpkiPin::from_hex(&"g".repeat(64)).is_err());
    }

    #[test]
    fn verifier_accepts_pinned_server_cert() {
        let (key, cert) = self_signed_cert();
        let pin = SpkiPin::of_certificate(&cert).unwrap();
        let v = PinnedSpkiVerifier {
            pin,
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
            mismatch: AtomicBool::new(false),
        };
        let res = v.verify_server_cert(
            &CertificateDer::from(cert),
            &[],
            &ServerName::try_from("audio-share.test").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(res.is_ok(), "pinned cert must verify, got {res:?}");
        assert!(!v.pin_mismatch());
        drop(key);
    }

    /// Spec §3: a pin mismatch fails the handshake (the session classifies 205).
    #[test]
    fn verifier_rejects_certificate_with_wrong_key() {
        let (_, cert_a) = self_signed_cert();
        let pin_a = SpkiPin::of_certificate(&cert_a).unwrap();
        let v = PinnedSpkiVerifier {
            pin: pin_a,
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
            mismatch: AtomicBool::new(false),
        };
        let (_, cert_b) = self_signed_cert();
        let res = v.verify_server_cert(
            &CertificateDer::from(cert_b),
            &[],
            &ServerName::try_from("audio-share.test").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(res.is_err(), "different server key must be rejected");
        assert!(v.pin_mismatch(), "the typed 205 marker must be set");
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("pinned identity"), "got: {msg}");
    }

    #[test]
    fn verifier_rejects_garbage_certificate() {
        let (_, cert) = self_signed_cert();
        let pin = SpkiPin::of_certificate(&cert).unwrap();
        let v = PinnedSpkiVerifier {
            pin,
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
            mismatch: AtomicBool::new(false),
        };
        let res = v.verify_server_cert(
            &CertificateDer::from(vec![0x30, 0x03, 0x02, 0x01, 0x00]),
            &[],
            &ServerName::try_from("audio-share.test").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(res.is_err());
        assert!(!v.pin_mismatch(), "a malformed cert is not a pin mismatch");
    }
}
