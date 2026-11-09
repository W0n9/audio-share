//! TCP control framing and UDP bind helpers.
//!
//! All integers on the Wire Protocol are little-endian. The command values are
//! frozen at v0.3.4: `GET_FORMAT = 1`, `START_PLAY = 2`, `HEARTBEAT = 3`.

use prost::Message;

use super::format::AudioFormat;

/// A TCP control command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Command {
    GetFormat = 1,
    StartPlay = 2,
    Heartbeat = 3,
}

impl Command {
    /// The frozen numeric command value.
    pub const fn to_u32(self) -> u32 {
        self as u32
    }

    /// Decode a numeric command value.
    pub fn from_u32(value: u32) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::GetFormat),
            2 => Ok(Self::StartPlay),
            3 => Ok(Self::Heartbeat),
            other => Err(ProtocolError::InvalidCommand(other)),
        }
    }
}

/// Encode a command as a little-endian `u32`, the on-wire representation.
pub fn encode_command(command: Command) -> [u8; 4] {
    command.to_u32().to_le_bytes()
}

/// Decode the command at the start of a control-channel buffer.
pub fn decode_command(bytes: &[u8]) -> Result<Command, ProtocolError> {
    if bytes.len() < 4 {
        return Err(ProtocolError::UnexpectedEof {
            needed: 4,
            available: bytes.len(),
        });
    }
    let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    Command::from_u32(value)
}

/// Encode a `GET_FORMAT` reply: command, payload size, protobuf payload.
pub fn encode_get_format_response(format: &AudioFormat) -> Vec<u8> {
    let payload = format.encode_to_vec();
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&encode_command(Command::GetFormat));
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    frame
}

/// Decode a `GET_FORMAT` reply, returning the format and the bytes consumed.
pub fn decode_get_format_response(bytes: &[u8]) -> Result<(AudioFormat, usize), ProtocolError> {
    if bytes.len() < 8 {
        return Err(ProtocolError::UnexpectedEof {
            needed: 8,
            available: bytes.len(),
        });
    }

    let command = decode_command(bytes)?;
    if command != Command::GetFormat {
        return Err(ProtocolError::UnexpectedCommand {
            expected: Command::GetFormat,
            actual: command,
        });
    }

    let size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let needed = 8usize.saturating_add(size);
    if bytes.len() < needed {
        return Err(ProtocolError::UnexpectedEof {
            needed,
            available: bytes.len(),
        });
    }

    let format = AudioFormat::decode(&bytes[8..needed])?;
    Ok((format, needed))
}

/// Encode a `START_PLAY` reply: command followed by the play id.
pub fn encode_start_play_response(id: i32) -> [u8; 8] {
    let mut frame = [0u8; 8];
    frame[..4].copy_from_slice(&encode_command(Command::StartPlay));
    frame[4..].copy_from_slice(&id.to_le_bytes());
    frame
}

/// Decode a `START_PLAY` reply, returning the play id and bytes consumed.
pub fn decode_start_play_response(bytes: &[u8]) -> Result<(i32, usize), ProtocolError> {
    if bytes.len() < 8 {
        return Err(ProtocolError::UnexpectedEof {
            needed: 8,
            available: bytes.len(),
        });
    }

    let command = decode_command(bytes)?;
    if command != Command::StartPlay {
        return Err(ProtocolError::UnexpectedCommand {
            expected: Command::StartPlay,
            actual: command,
        });
    }

    let id = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    Ok((id, 8))
}

/// Encode a heartbeat command.
pub fn encode_heartbeat() -> [u8; 4] {
    encode_command(Command::Heartbeat)
}

/// Decode a heartbeat command, returning the bytes consumed.
pub fn decode_heartbeat(bytes: &[u8]) -> Result<usize, ProtocolError> {
    let command = decode_command(bytes)?;
    if command != Command::Heartbeat {
        return Err(ProtocolError::UnexpectedCommand {
            expected: Command::Heartbeat,
            actual: command,
        });
    }
    Ok(4)
}

/// Encode the UDP bind packet: the play id as a little-endian `i32`.
pub fn encode_udp_bind(id: i32) -> [u8; 4] {
    id.to_le_bytes()
}

/// Decode the UDP bind packet.
pub fn decode_udp_bind(bytes: &[u8]) -> Result<i32, ProtocolError> {
    if bytes.len() < 4 {
        return Err(ProtocolError::UnexpectedEof {
            needed: 4,
            available: bytes.len(),
        });
    }
    Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// A malformed or unsupported Wire Protocol frame.
#[derive(Debug)]
pub enum ProtocolError {
    /// The buffer ended before a complete frame could be read.
    UnexpectedEof { needed: usize, available: usize },
    /// The command value is not one of `GET_FORMAT`, `START_PLAY`, `HEARTBEAT`.
    InvalidCommand(u32),
    /// A frame carried a different command than the decoder expected.
    UnexpectedCommand { expected: Command, actual: Command },
    /// A protobuf payload could not be decoded.
    Decode(prost::DecodeError),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedEof { needed, available } => {
                write!(f, "frame needs {needed} bytes, only {available} available")
            }
            Self::InvalidCommand(value) => write!(f, "unknown command {value}"),
            Self::UnexpectedCommand { expected, actual } => {
                write!(f, "expected {expected:?}, got {actual:?}")
            }
            Self::Decode(error) => write!(f, "invalid protobuf payload: {error}"),
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}

impl From<prost::DecodeError> for ProtocolError {
    fn from(error: prost::DecodeError) -> Self {
        Self::Decode(error)
    }
}
