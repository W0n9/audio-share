//! Media plane (spec §7): the 24-byte big-endian header and the per-Client
//! datagram packetizer.
//!
//! The packetizer is the whole send-side media contract behind one small
//! interface: instance/generation/sequence/timestamp bookkeeping, per-send
//! datagram sizing from the *current* transport limit, and whole-frame
//! chunking. It performs no I/O and never sleeps, so pacing and queueing stay
//! in the driver (spec §7.4) and the capture path never blocks.

/// Fixed media header size (spec §7.1, D4).
pub const HEADER_SIZE: usize = 24;

/// §7.1 flag bits. Bits 2–31 are reserved: senders MUST zero them.
pub const FLAG_STREAM_START: u32 = 0x1;
/// First datagram carrying a greater `config_generation` (§7.1).
pub const FLAG_CONFIG_CHANGE: u32 = 0x2;

/// The 24-byte media header (§7.1). Every field is big-endian on the wire;
/// PCM sample bytes keep their Transfer Format (little-endian) encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaHeader {
    /// ∈ [1, 2^32−1]; 0 invalid.
    pub stream_instance_id: u32,
    /// ∈ [1, 2^32−1]; +1 per decode-affecting change within the instance.
    pub config_generation: u32,
    /// Per instance: 0, +1 per datagram, mod 2^32.
    pub sequence: u32,
    /// Capture-clock sample-frame position of the first payload frame.
    pub timestamp: u64,
    /// STREAM_START / CONFIG_CHANGE; reserved bits always zero.
    pub flags: u32,
}

/// Why a datagram could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketError {
    /// The capture payload is not a whole number of sample frames.
    MisalignedPcm,
    /// The current transport limit cannot carry the header plus one frame;
    /// the session answers this with Close(602) (§7.3).
    DatagramTooSmall {
        /// The send-time `max_datagram_size()` that could not fit a frame.
        max_datagram_size: usize,
    },
    /// Sample-frame geometry unknown (zero bytes per frame).
    UnknownFrameGeometry,
    /// `set_generation` was called with a generation that is not newer.
    GenerationNotNewer,
    /// Instance ids and generations are nonzero on the wire (§7.1).
    ZeroField,
}

impl MediaHeader {
    /// Parse the fixed header, returning it with the payload slice.
    pub fn parse(datagram: &[u8]) -> Result<(Self, &[u8]), PacketError> {
        if datagram.len() < HEADER_SIZE {
            return Err(PacketError::MisalignedPcm);
        }
        let b = &datagram[..HEADER_SIZE];
        let header = MediaHeader {
            stream_instance_id: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            config_generation: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            sequence: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            timestamp: u64::from_be_bytes(b[12..20].try_into().expect("8 bytes")),
            flags: u32::from_be_bytes([b[20], b[21], b[22], b[23]]),
        };
        if header.stream_instance_id == 0 || header.config_generation == 0 {
            return Err(PacketError::ZeroField);
        }
        Ok((header, &datagram[HEADER_SIZE..]))
    }

    /// Serialize the header (big-endian, §7.1) into `out`.
    pub fn write_be(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.stream_instance_id.to_be_bytes());
        out.extend_from_slice(&self.config_generation.to_be_bytes());
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.flags.to_be_bytes());
    }
}

/// One session's media packetizer: all per-instance header state, nothing
/// else. Callers feed capture payloads and the send-time datagram limit.
#[derive(Debug)]
pub struct MediaPacketizer {
    stream_instance_id: u32,
    config_generation: u32,
    sequence: u32,
    timestamp: u64,
    frame_bytes: usize,
    stream_start_pending: bool,
    config_change_pending: bool,
}

impl MediaPacketizer {
    /// Fresh instance: sequence 0, timestamp 0, STREAM_START on the first
    /// datagram. `frame_bytes` = bytes_per_sample × channels.
    pub fn new(stream_instance_id: u32, config_generation: u32, frame_bytes: usize) -> Self {
        Self {
            stream_instance_id,
            config_generation,
            sequence: 0,
            timestamp: 0,
            frame_bytes,
            stream_start_pending: true,
            config_change_pending: false,
        }
    }

    /// The instance id carried by subsequent datagrams.
    pub const fn stream_instance_id(&self) -> u32 {
        self.stream_instance_id
    }

    /// The generation carried by subsequent datagrams.
    pub const fn config_generation(&self) -> u32 {
        self.config_generation
    }

    /// The capture-clock position of the next payload frame (what a late
    /// Subscriber is anchored at in `StreamStarted`).
    pub const fn next_timestamp(&self) -> u64 {
        self.timestamp
    }

    /// The next datagram's sequence number.
    pub const fn next_sequence(&self) -> u32 {
        self.sequence
    }

    /// Advance the sequence/clock position (instance change or late join).
    pub fn seek(&mut self, sequence: u32, timestamp: u64) -> Result<(), PacketError> {
        if self.frame_bytes == 0 {
            return Err(PacketError::UnknownFrameGeometry);
        }
        self.sequence = sequence;
        self.timestamp = timestamp;
        Ok(())
    }

    /// Record a decode-affecting configuration change: the generation must
    /// increase and the next datagram carries CONFIG_CHANGE (§7.1).
    pub fn set_generation(&mut self, generation: u32) -> Result<(), PacketError> {
        if generation == 0 || generation <= self.config_generation {
            return Err(PacketError::GenerationNotNewer);
        }
        self.config_generation = generation;
        self.config_change_pending = true;
        Ok(())
    }

    /// Chunk `pcm` into datagrams sized for `max_datagram_size`, queried by
    /// the caller at send time (§7.3, D2). Every payload is a whole number of
    /// sample frames; the clock advances by the frames actually sent.
    pub fn packetize(
        &mut self,
        pcm: &[u8],
        max_datagram_size: usize,
    ) -> Result<Vec<Vec<u8>>, PacketError> {
        if self.frame_bytes == 0 {
            return Err(PacketError::UnknownFrameGeometry);
        }
        if !pcm.len().is_multiple_of(self.frame_bytes) {
            return Err(PacketError::MisalignedPcm);
        }
        let Some(payload_max) = max_datagram_size.checked_sub(HEADER_SIZE) else {
            return Err(PacketError::DatagramTooSmall { max_datagram_size });
        };
        let frames_per_datagram = payload_max / self.frame_bytes;
        if frames_per_datagram == 0 {
            return Err(PacketError::DatagramTooSmall { max_datagram_size });
        }
        if pcm.is_empty() {
            return Ok(Vec::new());
        }

        let payload_max = frames_per_datagram * self.frame_bytes;
        let mut out = Vec::with_capacity(pcm.len().div_ceil(payload_max));
        for chunk in pcm.chunks(payload_max) {
            let frames = chunk.len() / self.frame_bytes;
            let mut flags = 0u32;
            if self.stream_start_pending {
                flags |= FLAG_STREAM_START;
                self.stream_start_pending = false;
            }
            if self.config_change_pending {
                flags |= FLAG_CONFIG_CHANGE;
                self.config_change_pending = false;
            }
            let header = MediaHeader {
                stream_instance_id: self.stream_instance_id,
                config_generation: self.config_generation,
                sequence: self.sequence,
                timestamp: self.timestamp,
                flags,
            };
            let mut datagram = Vec::with_capacity(HEADER_SIZE + chunk.len());
            header.write_be(&mut datagram);
            datagram.extend_from_slice(chunk);
            out.push(datagram);

            self.sequence = self.sequence.wrapping_add(1);
            self.timestamp += frames as u64;
        }
        Ok(out)
    }
}
