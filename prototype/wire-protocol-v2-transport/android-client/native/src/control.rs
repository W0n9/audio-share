//! Client-side control state machine (prototype-simple, mirrors the brief's
//! server states: CONNECTING→AUTHENTICATING→READY→PLAYING→CLOSING/CLOSED).
//!
//! Transport-agnostic and host-testable: the QUIC session layer feeds decoded
//! `Frame`s (single-sourced from proto-core) in and obeys the returned
//! verdicts. Never processes control before AUTH succeeds; SUBSCRIBE legal
//! only in READY.

pub use proto_core::frame::{AuthErrCode, CloseReason, MediaStartPayload};

use crate::codec::Frame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientState {
    /// Session object created; QUIC connect not yet established.
    Idle,
    /// QUIC connected, control stream open, AUTH sent, awaiting verdict.
    AwaitingAuth,
    /// AUTH_OK seen. SUBSCRIBE allowed.
    Ready,
    /// SUBSCRIBE sent and MEDIA_START received with this config.
    Playing(MediaStartPayload),
    /// Terminal. No control processing, no playback claims afterwards.
    Closed(CloseInfo),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseInfo {
    pub reason: CloseReason,
    pub origin: CloseOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseOrigin {
    /// Server sent CLOSE.
    Remote,
    /// Local stop() call or local protocol fault.
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    /// Violation worth CLOSE(PROTOCOL_ERROR) semantics; also counted in stats.
    Protocol(String),
    /// AUTH_ERR received: bad credential / rate limit etc. Terminal for the session.
    AuthRejected { code: AuthErrCode },
    /// Anything received after the session closed.
    AfterClose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlVerdict {
    /// Message processed; no outgoing action.
    Ok,
    /// Send CLOSE(PROTOCOL_ERROR) and shut down the connection.
    ProtocolError(String),
}

impl ClientState {
    pub fn initial() -> Self {
        ClientState::Idle
    }

    /// Pure transition: fold one received control message.
    pub fn on_control(&mut self, msg: &Frame) -> Result<ControlVerdict, ControlError> {
        if let ClientState::Closed(_) = self {
            return Err(ControlError::AfterClose);
        }
        match (self.clone(), msg) {
            // AUTH verdicts are only meaningful while awaiting them.
            (ClientState::AwaitingAuth, Frame::AuthOk { .. }) => {
                *self = ClientState::Ready;
                Ok(ControlVerdict::Ok)
            }
            (ClientState::AwaitingAuth, Frame::AuthErr(code)) => {
                *self = ClientState::Closed(CloseInfo {
                    reason: CloseReason::AuthFailed,
                    origin: CloseOrigin::Remote,
                });
                Err(ControlError::AuthRejected { code: *code })
            }
            (ClientState::AwaitingAuth, Frame::Close(reason)) => {
                *self = ClientState::Closed(CloseInfo {
                    reason: *reason,
                    origin: CloseOrigin::Remote,
                });
                Ok(ControlVerdict::Ok)
            }
            // AUTH must complete before anything else is processed.
            (ClientState::AwaitingAuth, other) => Err(ControlError::Protocol(format!(
                "received {} while awaiting AUTH_OK", other_type(other)
            ))),
            (ClientState::Idle, _) => Err(ControlError::Protocol(
                "control received before connect".into(),
            )),

            (ClientState::Ready, Frame::MediaStart(payload)) => {
                *self = ClientState::Playing(*payload);
                Ok(ControlVerdict::Ok)
            }
            (ClientState::Ready, Frame::Close(reason)) => {
                *self = ClientState::Closed(CloseInfo {
                    reason: *reason,
                    origin: CloseOrigin::Remote,
                });
                Ok(ControlVerdict::Ok)
            }
            (ClientState::Ready, other) => Err(ControlError::Protocol(format!(
                "received {} in READY", other_type(other)
            ))),

            (ClientState::Playing(_), Frame::MediaStart(payload)) => {
                // capture restart / config change: same shape, new instance id
                *self = ClientState::Playing(*payload);
                Ok(ControlVerdict::Ok)
            }
            (ClientState::Playing(_), Frame::Close(reason)) => {
                *self = ClientState::Closed(CloseInfo {
                    reason: *reason,
                    origin: CloseOrigin::Remote,
                });
                Ok(ControlVerdict::Ok)
            }
            (ClientState::Playing(_), other) => Err(ControlError::Protocol(format!(
                "received {} in PLAYING", other_type(other)
            ))),
            // Unreachable: the AfterClose guard at the top returns first; kept for exhaustiveness.
            (ClientState::Closed(_), _) => Err(ControlError::AfterClose),
        }
    }

    /// Client-initiated actions are gated here (JNI subscribe/unsubscribe call these).
    pub fn can_subscribe(&self) -> bool {
        matches!(self, ClientState::Ready)
    }

    pub fn can_unsubscribe(&self) -> bool {
        matches!(self, ClientState::Ready | ClientState::Playing(_))
    }

    pub fn media_config(&self) -> Option<MediaStartPayload> {
        match self {
            ClientState::Playing(cfg) => Some(*cfg),
            _ => None,
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, ClientState::Closed(_))
    }

    /// Name used by getState()/stats JSON.
    pub fn state_name(&self) -> &'static str {
        match self {
            ClientState::Idle => "IDLE",
            ClientState::AwaitingAuth => "AUTHENTICATING",
            ClientState::Ready => "READY",
            ClientState::Playing(_) => "PLAYING",
            ClientState::Closed(_) => "CLOSED",
        }
    }
}

fn other_type(msg: &Frame) -> &'static str {
    match msg {
        Frame::Auth { .. } => "AUTH",
        Frame::AuthOk { .. } => "AUTH_OK",
        Frame::AuthErr(_) => "AUTH_ERR",
        Frame::Subscribe => "SUBSCRIBE",
        Frame::MediaStart(_) => "MEDIA_START",
        Frame::Unsubscribe => "UNSUBSCRIBE",
        Frame::Close(_) => "CLOSE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn awaiting_auth() -> ClientState {
        ClientState::AwaitingAuth
    }

    fn media_start(instance: u32, gen: u32) -> Frame {
        Frame::MediaStart(MediaStartPayload {
            stream_instance_id: instance,
            config_generation: gen,
            sample_rate: 48000,
            channels: 2,
            sample_format: 1,
        })
    }

    #[test]
    fn auth_ok_in_awaiting_auth_transitions_to_ready() {
        let mut s = awaiting_auth();
        assert_eq!(s.on_control(&Frame::AuthOk { session_id: 42 }), Ok(ControlVerdict::Ok));
        assert_eq!(s, ClientState::Ready);
        assert!(s.can_subscribe());
        assert_eq!(s.state_name(), "READY");
    }

    #[test]
    fn auth_err_is_terminal_auth_failure() {
        let mut s = awaiting_auth();
        assert_eq!(
            s.on_control(&Frame::AuthErr(AuthErrCode::BadCredential)),
            Err(ControlError::AuthRejected { code: AuthErrCode::BadCredential })
        );
        assert!(s.is_closed());
        // and nothing is processed afterwards
        assert_eq!(
            s.on_control(&Frame::AuthOk { session_id: 1 }),
            Err(ControlError::AfterClose)
        );
    }

    #[test]
    fn media_before_auth_is_protocol_error() {
        let mut s = awaiting_auth();
        assert!(matches!(
            s.on_control(&media_start(1, 1)),
            Err(ControlError::Protocol(_))
        ));
    }

    #[test]
    fn control_in_idle_is_protocol_error() {
        let mut s = ClientState::initial();
        assert!(matches!(
            s.on_control(&Frame::AuthOk { session_id: 1 }),
            Err(ControlError::Protocol(_))
        ));
    }

    #[test]
    fn duplicate_auth_ok_rejected_in_ready() {
        let mut s = awaiting_auth();
        s.on_control(&Frame::AuthOk { session_id: 1 }).unwrap();
        assert!(matches!(
            s.on_control(&Frame::AuthOk { session_id: 1 }),
            Err(ControlError::Protocol(_))
        ));
    }

    #[test]
    fn media_start_in_ready_starts_playing() {
        let mut s = awaiting_auth();
        s.on_control(&Frame::AuthOk { session_id: 1 }).unwrap();
        s.on_control(&media_start(3, 1)).unwrap();
        assert_eq!(
            s,
            ClientState::Playing(MediaStartPayload {
                stream_instance_id: 3,
                config_generation: 1,
                sample_rate: 48000,
                channels: 2,
                sample_format: 1,
            })
        );
        assert_eq!(s.media_config().unwrap().stream_instance_id, 3);
        assert!(!s.can_subscribe(), "SUBSCRIBE must be legal only in READY");
        assert!(s.can_unsubscribe());
    }

    #[test]
    fn media_start_reconfigures_new_instance() {
        let mut s = awaiting_auth();
        s.on_control(&Frame::AuthOk { session_id: 1 }).unwrap();
        s.on_control(&media_start(1, 1)).unwrap();
        s.on_control(&media_start(2, 1)).unwrap();
        assert_eq!(s.media_config().unwrap().stream_instance_id, 2);
    }

    #[test]
    fn close_from_any_live_state_closes_with_remote_origin() {
        for start in [
            awaiting_auth(),
            {
                let mut s = awaiting_auth();
                s.on_control(&Frame::AuthOk { session_id: 1 }).unwrap();
                s
            },
        ] {
            let mut s = start;
            s.on_control(&Frame::Close(CloseReason::ServerStop)).unwrap();
            assert_eq!(
                s,
                ClientState::Closed(CloseInfo {
                    reason: CloseReason::ServerStop,
                    origin: CloseOrigin::Remote,
                })
            );
        }
    }

    #[test]
    fn close_reason_codes_map() {
        // the mapping lives in proto-core; the client relies on parse()
        // rejecting unknown codes before a Close ever reaches this machine
        assert_eq!(CloseReason::from_u8(0), Some(CloseReason::Normal));
        assert_eq!(CloseReason::from_u8(3), Some(CloseReason::AuthFailed));
        assert_eq!(CloseReason::from_u8(200), None);
    }

    #[test]
    fn auth_err_in_ready_is_protocol_error_not_auth_rejection() {
        let mut s = awaiting_auth();
        s.on_control(&Frame::AuthOk { session_id: 1 }).unwrap();
        assert!(matches!(
            s.on_control(&Frame::AuthErr(AuthErrCode::RateLimited)),
            Err(ControlError::Protocol(_))
        ));
        // state machine treats it as fault; caller issues CLOSE(PROTOCOL_ERROR)
    }

    #[test]
    fn unsubscribe_not_legal_in_awaiting_auth_or_closed() {
        assert!(!awaiting_auth().can_unsubscribe());
        let mut s = awaiting_auth();
        s.on_control(&Frame::Close(CloseReason::Normal)).unwrap();
        assert!(!s.can_unsubscribe());
        assert!(!s.can_subscribe());
    }
}
