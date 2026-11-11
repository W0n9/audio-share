//! Control-plane frame codec (throwaway prototype shape, ticket 01).
//!
//! Wire shape fixed by the prototype brief:
//! `u32 BE frame_len | u8 msg_type | payload` with `frame_len = 1 + payload.len()`
//! and a 64 KiB cap on `frame_len`. NOT the normative v2 layout.

/// Hard cap on a whole frame (length field included) from the brief: 64 KiB.
pub const MAX_FRAME_LEN: usize = 64 * 1024;

pub const TYPE_AUTH: u8 = 0x01;
pub const TYPE_AUTH_OK: u8 = 0x02;
pub const TYPE_AUTH_ERR: u8 = 0x03;
pub const TYPE_SUBSCRIBE: u8 = 0x04;
pub const TYPE_MEDIA_START: u8 = 0x05;
pub const TYPE_UNSUBSCRIBE: u8 = 0x06;
pub const TYPE_CLOSE: u8 = 0x07;

/// One decoded control message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMsg {
    /// 0x01 AUTH: utf-8 test credential token.
    Auth(String),
    /// 0x02 AUTH_OK: u32 BE session_id.
    AuthOk(u32),
    /// 0x03 AUTH_ERR: u8 code (1=BAD_CREDENTIAL 2=NOT_AUTHENTICATED 3=MALFORMED 4=RATE_LIMITED).
    AuthErr(u8),
    /// 0x04 SUBSCRIBE: empty.
    Subscribe,
    /// 0x05 MEDIA_START: u32 stream_instance_id, u32 config_generation, u32 sample_rate,
    /// u8 channels, u8 sample_format (1=f32le) — all BE.
    MediaStart {
        stream_instance_id: u32,
        config_generation: u32,
        sample_rate: u32,
        channels: u8,
        sample_format: u8,
    },
    /// 0x06 UNSUBSCRIBE: empty.
    Unsubscribe,
    /// 0x07 CLOSE: u8 reason (0=NORMAL 1=SERVER_STOP 2=PROTOCOL_ERROR 3=AUTH_FAILED
    /// 4=IDLE_TIMEOUT 5=INTERNAL_ERROR).
    Close(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codec error: {}", self.0)
    }
}
impl std::error::Error for CodecError {}

pub type CodecResult<T> = Result<T, CodecError>;

/// Encode one control message into the wire frame.
pub fn encode_frame(msg: &ControlMsg) -> Vec<u8> {
    let mut body = Vec::with_capacity(64);
    let ty = match msg {
        ControlMsg::Auth(token) => {
            body.extend_from_slice(token.as_bytes());
            TYPE_AUTH
        }
        ControlMsg::AuthOk(id) => {
            body.extend_from_slice(&id.to_be_bytes());
            TYPE_AUTH_OK
        }
        ControlMsg::AuthErr(code) => {
            body.push(*code);
            TYPE_AUTH_ERR
        }
        ControlMsg::Subscribe => TYPE_SUBSCRIBE,
        ControlMsg::MediaStart {
            stream_instance_id,
            config_generation,
            sample_rate,
            channels,
            sample_format,
        } => {
            body.extend_from_slice(&stream_instance_id.to_be_bytes());
            body.extend_from_slice(&config_generation.to_be_bytes());
            body.extend_from_slice(&sample_rate.to_be_bytes());
            body.push(*channels);
            body.push(*sample_format);
            TYPE_MEDIA_START
        }
        ControlMsg::Unsubscribe => TYPE_UNSUBSCRIBE,
        ControlMsg::Close(reason) => {
            body.push(*reason);
            TYPE_CLOSE
        }
    };
    let frame_len = (1 + body.len()) as u32;
    debug_assert!(frame_len as usize <= MAX_FRAME_LEN);
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&frame_len.to_be_bytes());
    out.push(ty);
    out.extend_from_slice(&body);
    out
}

/// Streaming frame decoder: feed raw bytes off the QUIC control stream,
/// pop whole frames as they complete.
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append raw bytes, then drain any complete frames out.
    /// Errors are sticky: once a malformed frame is seen the stream is unusable.
    pub fn push(&mut self, bytes: &[u8]) -> CodecResult<Vec<ControlMsg>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 4 {
                return Ok(out);
            }
            let frame_len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]])
                as usize;
            if frame_len < 1 {
                return Err(CodecError(format!("frame_len {frame_len} < 1")));
            }
            if frame_len > MAX_FRAME_LEN {
                return Err(CodecError(format!(
                    "frame_len {frame_len} exceeds cap {MAX_FRAME_LEN}"
                )));
            }
            let total = 4 + frame_len;
            if self.buf.len() < total {
                return Ok(out);
            }
            let frame = self.buf.drain(..total).collect::<Vec<u8>>();
            out.push(decode_frame_body(&frame[5..], frame[4])?);
        }
    }
}

/// Decode the type byte + payload of one frame (the u32 length prefix already consumed).
fn decode_frame_body(payload: &[u8], ty: u8) -> CodecResult<ControlMsg> {
    let need = |n: usize| -> CodecResult<()> {
        if payload.len() != n {
            Err(CodecError(format!(
                "type 0x{ty:02x}: payload len {} != {n}",
                payload.len()
            )))
        } else {
            Ok(())
        }
    };
    match ty {
        TYPE_AUTH => Ok(ControlMsg::Auth(
            String::from_utf8(payload.to_vec())
                .map_err(|_| CodecError("AUTH payload is not utf-8".into()))?,
        )),
        TYPE_AUTH_OK => {
            need(4)?;
            Ok(ControlMsg::AuthOk(u32::from_be_bytes([
                payload[0], payload[1], payload[2], payload[3],
            ])))
        }
        TYPE_AUTH_ERR => {
            need(1)?;
            Ok(ControlMsg::AuthErr(payload[0]))
        }
        TYPE_SUBSCRIBE => {
            need(0)?;
            Ok(ControlMsg::Subscribe)
        }
        TYPE_MEDIA_START => {
            need(14)?;
            Ok(ControlMsg::MediaStart {
                stream_instance_id: u32::from_be_bytes([
                    payload[0], payload[1], payload[2], payload[3],
                ]),
                config_generation: u32::from_be_bytes([
                    payload[4], payload[5], payload[6], payload[7],
                ]),
                sample_rate: u32::from_be_bytes([
                    payload[8], payload[9], payload[10], payload[11],
                ]),
                channels: payload[12],
                sample_format: payload[13],
            })
        }
        TYPE_UNSUBSCRIBE => {
            need(0)?;
            Ok(ControlMsg::Unsubscribe)
        }
        TYPE_CLOSE => {
            need(1)?;
            Ok(ControlMsg::Close(payload[0]))
        }
        other => Err(CodecError(format!("unknown msg_type 0x{other:02x}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_roundtrip() {
        let msg = ControlMsg::Auth("test-credential-do-not-use".to_string());
        let bytes = encode_frame(&msg);
        // length prefix = 1 + token len
        let expect_len = 1 + "test-credential-do-not-use".len() as u32;
        assert_eq!(&bytes[..4], &expect_len.to_be_bytes());
        assert_eq!(bytes[4], TYPE_AUTH);
        let mut dec = FrameDecoder::new();
        let out = dec.push(&bytes).unwrap();
        assert_eq!(out, vec![msg]);
    }

    #[test]
    fn all_message_types_roundtrip() {
        let msgs = vec![
            ControlMsg::Auth(String::new()),
            ControlMsg::Auth("tok".into()),
            ControlMsg::AuthOk(0xdead_beef),
            ControlMsg::AuthErr(4),
            ControlMsg::Subscribe,
            ControlMsg::MediaStart {
                stream_instance_id: 7,
                config_generation: 1,
                sample_rate: 48000,
                channels: 2,
                sample_format: 1,
            },
            ControlMsg::Unsubscribe,
            ControlMsg::Close(2),
        ];
        for msg in msgs {
            let mut dec = FrameDecoder::new();
            assert_eq!(dec.push(&encode_frame(&msg)).unwrap(), vec![msg.clone()]);
        }
    }

    #[test]
    fn pipelined_frames_decode_in_order() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_frame(&ControlMsg::Auth("abc".into())));
        bytes.extend_from_slice(&encode_frame(&ControlMsg::Subscribe));
        bytes.extend_from_slice(&encode_frame(&ControlMsg::AuthOk(9)));
        let mut dec = FrameDecoder::new();
        assert_eq!(
            dec.push(&bytes).unwrap(),
            vec![
                ControlMsg::Auth("abc".into()),
                ControlMsg::Subscribe,
                ControlMsg::AuthOk(9)
            ]
        );
    }

    #[test]
    fn split_across_pushes() {
        let bytes = encode_frame(&ControlMsg::MediaStart {
            stream_instance_id: 1,
            config_generation: 1,
            sample_rate: 48000,
            channels: 2,
            sample_format: 1,
        });
        let mut dec = FrameDecoder::new();
        // byte-by-byte: no frame may appear until the last byte lands
        for (i, b) in bytes.iter().enumerate() {
            let out = dec.push(&[*b]).unwrap();
            if i + 1 < bytes.len() {
                assert!(out.is_empty(), "frame decoded early at byte {i}");
            } else {
                assert_eq!(out.len(), 1);
            }
        }
    }

    #[test]
    fn zero_frame_len_rejected() {
        let mut dec = FrameDecoder::new();
        assert!(dec.push(&[0, 0, 0, 0, 1]).is_err());
    }

    #[test]
    fn oversized_frame_rejected() {
        // frame_len = cap + 1, valid-looking so far
        let big = (MAX_FRAME_LEN as u32 + 1).to_be_bytes();
        let mut dec = FrameDecoder::new();
        assert!(dec.push(&big).is_err());
    }

    #[test]
    fn unknown_type_rejected() {
        // frame_len = 1, type byte = 0x99
        let mut dec = FrameDecoder::new();
        let err = dec.push(&[0, 0, 0, 1, 0x99]).unwrap_err();
        assert!(err.0.contains("unknown msg_type"), "got: {err}");
    }

    #[test]
    fn malformed_payload_length_rejected() {
        // AUTH_OK needs exactly 4 payload bytes; give 3
        let mut dec = FrameDecoder::new();
        let err = dec.push(&[0, 0, 0, 4, TYPE_AUTH_OK, 1, 2, 3]).unwrap_err();
        assert!(err.0.contains("payload len 3 != 4"), "got: {err}");
        // SUBSCRIBE must be empty; give 1 byte
        let mut dec = FrameDecoder::new();
        assert!(dec.push(&[0, 0, 0, 2, TYPE_SUBSCRIBE, 0x00]).is_err());
        // MEDIA_START needs exactly 14 payload bytes
        let mut dec = FrameDecoder::new();
        assert!(dec
            .push(&[0, 0, 0, 5, TYPE_MEDIA_START, 1, 2, 3, 4, 5])
            .is_err());
    }

    #[test]
    fn max_cap_frame_accepted() {
        // exactly at the cap: frame_len = 1 + payload, huge utf-8 AUTH token
        let payload_len = MAX_FRAME_LEN - 1;
        let mut dec = FrameDecoder::new();
        let mut bytes = (MAX_FRAME_LEN as u32).to_be_bytes().to_vec();
        bytes.push(TYPE_AUTH);
        bytes.extend(std::iter::repeat(b'a').take(payload_len));
        assert_eq!(
            dec.push(&bytes).unwrap(),
            vec![ControlMsg::Auth("a".repeat(payload_len))]
        );
    }
}
