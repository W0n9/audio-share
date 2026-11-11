//! Server identity helpers: extract the SubjectPublicKeyInfo DER from an X.509
//! certificate and hash it for SPKI pinning.
//!
//! The prototype pins `sha256(SPKI DER)` of the server's self-signed leaf
//! certificate. Extraction uses a minimal DER walk over the standard
//! `Certificate ::= SEQUENCE { tbsCertificate, ... }` shape; correctness is
//! cross-checked in tests against rcgen's independently serialized public key
//! DER for the same key pair.

use ring::digest;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SpkiError {
    #[error("truncated or malformed DER")]
    Malformed,
    #[error("subjectPublicKeyInfo not found in certificate")]
    NotFound,
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
fn parse_tlv_header(buf: &[u8]) -> Result<TlvHeader, SpkiError> {
    if buf.len() < 2 {
        return Err(SpkiError::Malformed);
    }
    let tag = buf[0];
    let first = buf[1] as usize;
    let (content_len, header_len) = if first < 0x80 {
        (first, 2usize)
    } else {
        let count = first & 0x7f;
        if count == 0 || count > 4 || buf.len() < 2 + count {
            return Err(SpkiError::Malformed);
        }
        let mut len = 0usize;
        for &b in &buf[2..2 + count] {
            len = (len << 8) | b as usize;
        }
        (len, 2 + count)
    };
    let header = TlvHeader { tag, content_len, header_len };
    if header.total_len() > buf.len() {
        return Err(SpkiError::Malformed);
    }
    Ok(header)
}

/// Consumes one DER TLV element from the front of `buf`, returning
/// `(tag, full_element_bytes_including_header)` and advancing `buf`.
fn next_tlv<'a>(buf: &mut &'a [u8]) -> Result<(u8, &'a [u8]), SpkiError> {
    let header = parse_tlv_header(buf)?;
    let (element, rest) = buf.split_at(header.total_len());
    *buf = rest;
    Ok((header.tag, element))
}

/// Strips the TLV header, returning only the element's content octets.
fn element_content(element: &[u8]) -> Result<&[u8], SpkiError> {
    let header = parse_tlv_header(element)?;
    Ok(&element[header.header_len..header.total_len()])
}

/// Extracts the full DER bytes of the subjectPublicKeyInfo element from an
/// X.509 certificate.
///
/// tbsCertificate child order (RFC 5280): optional `[0]` version, INTEGER
/// serialNumber, SEQUENCE signature, SEQUENCE issuer, SEQUENCE validity,
/// SEQUENCE subject, SEQUENCE subjectPublicKeyInfo — i.e. the fifth SEQUENCE
/// in the TBS is the SPKI.
pub fn extract_spki_der(cert_der: &[u8]) -> Result<Vec<u8>, SpkiError> {
    let mut cursor = cert_der;
    let (cert_tag, cert_el) = next_tlv(&mut cursor)?;
    if cert_tag != 0x30 {
        return Err(SpkiError::Malformed);
    }
    let mut cert_children = element_content(cert_el)?;
    // First certificate child: tbsCertificate.
    let (tbs_tag, tbs_el) = next_tlv(&mut cert_children)?;
    if tbs_tag != 0x30 {
        return Err(SpkiError::Malformed);
    }
    let mut tbs_children = element_content(tbs_el)?;
    // First TBS child: optional [0] EXPLICIT version (0xa0), else serialNumber.
    let (first_tag, _) = next_tlv(&mut tbs_children)?;
    if first_tag != 0xa0 && first_tag != 0x02 {
        return Err(SpkiError::Malformed);
    }
    // Remaining children: serialNumber (if version was present), signature
    // algorithm, issuer, validity, subject — all counting SEQUENCE tags; the
    // fifth SEQUENCE is subjectPublicKeyInfo.
    let mut sequence_count = 0usize;
    loop {
        if tbs_children.is_empty() {
            return Err(SpkiError::NotFound);
        }
        let (tag, element) = next_tlv(&mut tbs_children)?;
        if tag == 0x30 {
            sequence_count += 1;
            if sequence_count == 5 {
                return Ok(element.to_vec());
            }
        }
    }
}

/// sha256 over the SPKI DER — the pin value clients compare against.
pub fn spki_sha256(spki_der: &[u8]) -> [u8; 32] {
    digest::digest(&digest::SHA256, spki_der).as_ref().try_into().expect("sha256 is 32 bytes")
}

/// Lowercase hex encoding without separators (pin string format).
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses a 64-hex-character string (any case, optional `sha256/` prefix or
/// `:` separators are NOT accepted — plain hex only) into a 256-bit pin.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HexError {
    #[error("expected 64 hex characters, got {0}")]
    BadLength(usize),
    #[error("invalid hex character")]
    BadChar,
}

pub fn parse_hex32(s: &str) -> Result<[u8; 32], HexError> {
    let s = s.trim();
    if s.len() != 64 {
        return Err(HexError::BadLength(s.len()));
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let hi = s.as_bytes()[2 * i]
            .to_ascii_digit()
            .ok_or(HexError::BadChar)?;
        let lo = s.as_bytes()[2 * i + 1]
            .to_ascii_digit()
            .ok_or(HexError::BadChar)?;
        *byte = (hi << 4) | lo;
    }
    Ok(out)
}

trait HexDigit {
    fn to_ascii_digit(&self) -> Option<u8>;
}

impl HexDigit for u8 {
    fn to_ascii_digit(&self) -> Option<u8> {
        match self {
            b'0'..=b'9' => Some(self - b'0'),
            b'a'..=b'f' => Some(self - b'a' + 10),
            b'A'..=b'F' => Some(self - b'A' + 10),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_self_signed() -> (rcgen::Certificate, rcgen::KeyPair) {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params =
            rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "audio-share proto test server");
        let cert = params.self_signed(&key_pair).expect("self-signed");
        (cert, key_pair)
    }

    #[test]
    fn extracted_spki_matches_rcgen_serialized_public_key() {
        let (cert, key_pair) = generate_self_signed();
        let cert_der = cert.der().as_ref();
        let extracted = extract_spki_der(cert_der).expect("extract");
        let expected = key_pair.public_key_der();
        let expected: &[u8] = expected.as_ref();
        assert_eq!(
            extracted, expected,
            "minimal DER walk must find exactly the SPKI rcgen serialized"
        );
    }

    #[test]
    fn sha256_matches_known_nist_vector() {
        // Independent source of truth: NIST FIPS 180 test vector for "abc".
        assert_eq!(
            to_hex(&spki_sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pin_hex_roundtrips_case_insensitively() {
        let (cert, _) = generate_self_signed();
        let spki = extract_spki_der(cert.der().as_ref()).expect("extract");
        let pin = spki_sha256(&spki);
        let lower = to_hex(&pin);
        assert_eq!(lower.len(), 64);
        assert_eq!(parse_hex32(&lower).expect("parse"), pin);
        let upper = lower.to_uppercase();
        assert_eq!(parse_hex32(&upper).expect("parse"), pin);
    }

    #[test]
    fn pin_parse_rejects_bad_input() {
        assert_eq!(parse_hex32("abcd"), Err(HexError::BadLength(4)));
        assert!(parse_hex32(&"g".repeat(64)).is_err());
    }

    #[test]
    fn malformed_der_is_rejected_without_panicking() {
        for junk in [
            &[][..],
            &[0x30u8],
            &[0x30u8, 0x00],
            &[0x30u8, 0x81, 0xff, 0x01],
            &[0x30u8, 0x82, 0xff, 0xff, 0x01, 0x02],
        ] {
            assert!(extract_spki_der(junk).is_err(), "junk {:?} must not parse", junk);
        }
        // A well-formed certificate whose TBS holds only a serial number —
        // fewer than five SEQUENCEs, so the SPKI is absent.
        let short_tbs = [0x30u8, 0x05, 0x30, 0x03, 0x02, 0x01, 0x00];
        assert_eq!(extract_spki_der(&short_tbs), Err(SpkiError::NotFound));
    }
}
