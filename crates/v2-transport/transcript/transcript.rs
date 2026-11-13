//! Language-neutral transcript encoding shared by the Rust Server and the
//! Android Client's separately built native crate. Wire bytes remain governed
//! by the independent golden fixtures, not by cross-crate encoder agreement.

/// A TLV element exceeds its normative u16 length field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LengthOverflow;

impl std::fmt::Display for LengthOverflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("transcript TLV value exceeds 65535 bytes")
    }
}

impl std::error::Error for LengthOverflow {}

/// Append `tag(u8) || length(u16 BE) || value` without truncating its length.
pub fn push_tlv(output: &mut Vec<u8>, tag: u8, value: &[u8]) -> Result<(), LengthOverflow> {
    let length = u16::try_from(value.len()).map_err(|_| LengthOverflow)?;
    output.push(tag);
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

/// pairing.md section 9.1: append the exactly six auth TLV elements.
pub fn append_auth_context(
    output: &mut Vec<u8>,
    server_random: &[u8],
    client_public_key: &[u8],
    server_public_key: &[u8],
    protocol_major: u32,
    auth_exporter: &[u8],
) -> Result<(), LengthOverflow> {
    for (tag, value) in [
        (1, b"auth".as_slice()),
        (2, server_random),
        (3, client_public_key),
        (4, server_public_key),
        (5, &protocol_major.to_be_bytes()),
        (6, auth_exporter),
    ] {
        push_tlv(output, tag, value)?;
    }
    Ok(())
}
