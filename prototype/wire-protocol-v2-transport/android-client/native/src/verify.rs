//! SPKI-pinned server certificate verifier (prototype).
//!
//! Server identity = sha256 of the server cert's SubjectPublicKeyInfo DER.
//! The pin comes from the operator (server prints it at startup); this verifier
//! accepts exactly that key and rejects everything else. No CA chain, no name
//! checking, no expiry checking — the pin IS the identity check for the
//! prototype, matching the brief's "clients PIN that SPKI hash".
//!
//! SPKI extraction, sha256 and hex parsing are single-sourced on
//! `proto_core::spki` (the server crate's helpers). The verifier itself is
//! client-side (the server pins nothing).
//!
//! Host-testable against rcgen-generated certs without a QUIC connection.

use proto_core::spki::{extract_spki_der, parse_hex32, spki_sha256, to_hex, HexError, SpkiError};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};

/// sha256 of the SPKI DER, as raw 32 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpkiPin(pub [u8; 32]);

impl SpkiPin {
    /// Parse the 64-hex-char SPKI sha256 string printed by the server.
    pub fn from_hex(s: &str) -> Result<Self, HexError> {
        Ok(SpkiPin(parse_hex32(s)?))
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    /// Compute the pin of a DER-encoded X.509 certificate (server-print helper /
    /// test tooling / negative-case construction).
    pub fn of_certificate(cert_der: &[u8]) -> Result<Self, SpkiError> {
        let spki = extract_spki_der(cert_der)?;
        Ok(SpkiPin(spki_sha256(&spki)))
    }
}

/// The verifier: accepts exactly one SPKI sha256, delegates signature checks to
/// the ring provider's algorithm table.
#[derive(Debug)]
pub struct PinnedSpkiVerifier {
    pin: SpkiPin,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedSpkiVerifier {
    pub fn new(pinned_spki_sha256_hex: &str) -> Result<Self, HexError> {
        Ok(PinnedSpkiVerifier {
            pin: SpkiPin::from_hex(pinned_spki_sha256_hex)?,
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
        })
    }

    pub fn pin(&self) -> &SpkiPin {
        &self.pin
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
            // never log pins or key material detail beyond the mismatch fact
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

    #[test]
    fn pin_of_certificate_is_sha256_of_spki() {
        let (key, cert) = self_signed_cert();
        let pin = SpkiPin::of_certificate(&cert).unwrap();
        // independent digest over rcgen's own SPKI serialization
        let expect = spki_sha256(key.public_key_der().as_ref());
        assert_eq!(pin.0, expect);
        // roundtrip through the hex form the server prints
        let hex = pin.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(SpkiPin::from_hex(&hex).unwrap(), pin);
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
        assert_eq!(SpkiPin::from_hex("abc"), Err(HexError::BadLength(3)));
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
        };
        let res = v.verify_server_cert(
            &CertificateDer::from(cert),
            &[],
            &ServerName::try_from("audio-share.test").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(res.is_ok(), "pinned cert must verify, got {res:?}");
        // the key pair must stay alive so the cert's key isn't dropped early
        drop(key);
    }

    #[test]
    fn verifier_rejects_certificate_with_wrong_key() {
        let (_, cert_a) = self_signed_cert();
        let pin_a = SpkiPin::of_certificate(&cert_a).unwrap();
        let v = PinnedSpkiVerifier {
            pin: pin_a,
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
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
    }

    #[test]
    fn verifier_rejects_garbage_certificate() {
        let (_, cert) = self_signed_cert();
        let pin = SpkiPin::of_certificate(&cert).unwrap();
        let v = PinnedSpkiVerifier {
            pin,
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
        };
        let res = v.verify_server_cert(
            &CertificateDer::from(vec![0x30, 0x03, 0x02, 0x01, 0x00]),
            &[],
            &ServerName::try_from("audio-share.test").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(res.is_err());
    }
}
