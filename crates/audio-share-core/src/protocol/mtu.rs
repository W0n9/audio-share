//! UDP MTU segmentation.
//!
//! v0.3.4 uses an MTU of 1492 bytes, minus a 20-byte IP header and an 8-byte
//! UDP header, leaving 1464 bytes of PCM. Segments are then aligned down to
//! `block_align` so a sample frame is never split across datagrams.

/// Link MTU assumed by v0.3.4.
pub const MTU: usize = 1492;

/// IPv4 header length subtracted from the MTU.
pub const IP_HEADER_BYTES: usize = 20;

/// UDP header length subtracted from the MTU.
pub const UDP_HEADER_BYTES: usize = 8;

/// Maximum UDP payload before `block_align` alignment: `1492 - 20 - 8 = 1464`.
pub const MAX_PAYLOAD_BYTES: usize = MTU - IP_HEADER_BYTES - UDP_HEADER_BYTES;

/// Largest segment payload that fits the MTU and is a multiple of `block_align`.
///
/// A `block_align` of `0` (unknown or invalid format) or larger than
/// [`MAX_PAYLOAD_BYTES`] yields `0`: no valid segment fits a sample frame, so
/// the caller must not send PCM rather than panic or emit oversized datagrams.
pub fn max_segment_payload(block_align: usize) -> usize {
    if block_align == 0 || block_align > MAX_PAYLOAD_BYTES {
        return 0;
    }
    MAX_PAYLOAD_BYTES - (MAX_PAYLOAD_BYTES % block_align)
}

/// Split PCM into UDP segment payloads, mirroring v0.3.4.
///
/// Every segment except the last is exactly [`max_segment_payload`] bytes; the
/// last segment carries the remainder, which may be shorter and (when the
/// capture buffer itself is not aligned) not a multiple of `block_align`. An
/// empty buffer, or an invalid `block_align`, produces no segments.
pub fn segment_audio(data: &[u8], block_align: usize) -> Vec<&[u8]> {
    let max = max_segment_payload(block_align);
    if max == 0 || data.is_empty() {
        return Vec::new();
    }
    data.chunks(max).collect()
}
