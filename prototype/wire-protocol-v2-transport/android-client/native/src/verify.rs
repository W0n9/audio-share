//! SPKI-pinned server certificate verifier (prototype).
//!
//! Server identity = sha256 of the server cert's SubjectPublicKeyInfo DER.
//! The pin comes from the operator (server prints it at startup); this verifier
//! accepts exactly that key and rejects everything else. No CA chain, no name
//! checking, no expiry checking — the pin IS the identity check for the
//! prototype, matching the brief's "clients PIN that SPKI hash".
//!
//! Host-testable against rcgen-generated certs without a QUIC connection.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};

/// sha256 of the SPKI DER, as raw 32 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpkiPin(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinError {
    BadHexLen(usize),
    BadHex,
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinError::BadHexLen(n) => write!(f, "SPKI pin must be 64 hex chars, got {n}"),
            PinError::BadHex => write!(f, "SPKI pin contains non-hex characters"),
        }
    }
}

impl SpkiPin {
    /// Parse the 64-hex-char SPKI sha256 string printed by the server.
    pub fn from_hex(s: &str) -> Result<Self, PinError> {
        let s = s.trim().to_lowercase();
        if s.len() != 64 {
            return Err(PinError::BadHexLen(s.len()));
        }
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            let hi = hex_val(s.as_bytes()[2 * i]).ok_or(PinError::BadHex)?;
            let lo = hex_val(s.as_bytes()[2 * i + 1]).ok_or(PinError::BadHex)?;
            *b = (hi << 4) | lo;
        }
        Ok(SpkiPin(out))
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// Compute the pin of a DER-encoded X.509 certificate (server-print helper /
    /// test tooling / negative-case construction).
    pub fn of_certificate(cert_der: &[u8]) -> Result<Self, String> {
        let spki = extract_spki(cert_der)?;
        Ok(SpkiPin(sha256(spki)))
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let d = ring::digest::digest(&ring::digest::SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// Walk a DER TLV: returns (tag, content range, offset past the TLV).
fn der_tlv(buf: &[u8], at: usize) -> Result<(u8, std::ops::Range<usize>, usize), String> {
    if at >= buf.len() {
        return Err("der: truncated at tag".into());
    }
    let tag = buf[at];
    let mut i = at + 1;
    if i >= buf.len() {
        return Err("der: truncated at length".into());
    }
    let first = buf[i];
    i += 1;
    let len = if first < 0x80 {
        first as usize
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return Err(format!("der: unsupported length form {first:#x}"));
        }
        if i + n > buf.len() {
            return Err("der: truncated long length".into());
        }
        let mut v = 0usize;
        for k in 0..n {
            v = (v << 8) | buf[i + k] as usize;
        }
        i += n;
        v
    };
    let end = i + len;
    if end > buf.len() {
        return Err(format!(
            "der: TLV content {}..{end} exceeds buffer {}",
            i,
            buf.len()
        ));
    }
    Ok((tag, i..end, end))
}

/// Parse the immediate children of a constructed TLV.
fn der_children(buf: &[u8], content: std::ops::Range<usize>) -> Result<Vec<(u8, std::ops::Range<usize>)>, String> {
    let mut out = Vec::new();
    let mut i = content.start;
    while i < content.end {
        let (tag, r, next) = der_tlv(buf, i)?;
        out.push((tag, r));
        i = next;
    }
    Ok(out)
}

/// Extract the SubjectPublicKeyInfo TLV (full DER, including its tag+length)
/// from an X.509 certificate: SEQUENCE{ tbs: SEQUENCE{ [0]?, serial, sigalg,
/// issuer, validity, subject, spki, ... }, sigalg, sig }.
pub fn extract_spki(cert_der: &[u8]) -> Result<&[u8], String> {
    let (tag, cert_content, _) = der_tlv(cert_der, 0)?;
    if tag != 0x30 {
        return Err(format!("der: certificate tag {tag:#x} != SEQUENCE"));
    }
    let tbs_children = der_children(cert_der, cert_content)?;
    let (tbs_tag, tbs_content) = tbs_children
        .first()
        .ok_or("der: certificate has no children")?
        .clone();
    if tbs_tag != 0x30 {
        return Err(format!("der: tbsCertificate tag {tbs_tag:#x} != SEQUENCE"));
    }
    let children = der_children(cert_der, tbs_content)?;
    // skip optional [0] EXPLICIT version (context tag 0xa0)
    let start = if children.first().map(|c| c.0) == Some(0xa0) { 1 } else { 0 };
    // remaining: serial(0) sigalg(1) issuer(2) validity(3) subject(4) spki(5)
    let spki = children
        .get(start + 5)
        .ok_or("der: tbsCertificate has too few elements for SPKI")?;
    if spki.0 != 0x30 {
        return Err(format!("der: element {} is not the SPKI SEQUENCE", spki.0));
    }
    // include the SPKI's own tag+length: back up from content start
    // (re-derive the header: tag byte + length bytes)
    let content_start = spki.1.start;
    let header_len = header_len_of(cert_der, content_start)?;
    Ok(&cert_der[content_start - header_len..spki.1.end])
}

/// Length of the DER header (tag + length bytes) immediately before `content_start`.
fn header_len_of(buf: &[u8], content_start: usize) -> Result<usize, String> {
    // walk backwards: content_start-1 is the last length byte
    if content_start < 2 {
        return Err("der: bad header".into());
    }
    let first = buf[content_start - 1];
    let hdr = if first < 0x80 {
        2 // tag + short length
    } else {
        2 + (first & 0x7f) as usize
    };
    Ok(hdr)
}

/// The verifier: accepts exactly one SPKI sha256, delegates signature checks to
/// the ring provider's algorithm table.
#[derive(Debug)]
pub struct PinnedSpkiVerifier {
    pin: SpkiPin,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedSpkiVerifier {
    pub fn new(pinned_spki_sha256_hex: &str) -> Result<Self, PinError> {
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
            .map_err(|e| TlsError::General(e))?;
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
    fn spki_extraction_matches_rcgen_public_key_der() {
        let (key, cert) = self_signed_cert();
        let expect_der = key.public_key_der();
        let expect: &[u8] = expect_der.as_ref();
        assert_eq!(extract_spki(&cert).unwrap(), expect);
    }

    #[test]
    fn pin_of_certificate_is_sha256_of_spki() {
        let (key, cert) = self_signed_cert();
        let pin = SpkiPin::of_certificate(&cert).unwrap();
        assert_eq!(pin.0, sha256(key.public_key_der().as_ref()));
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
        assert_eq!(SpkiPin::from_hex("abc"), Err(PinError::BadHexLen(3)));
        assert_eq!(SpkiPin::from_hex(&"g".repeat(64)), Err(PinError::BadHex));
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
