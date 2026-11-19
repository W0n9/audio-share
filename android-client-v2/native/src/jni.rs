//! JNI surface — LOW-FREQUENCY ONLY (connect/auth/stop/state/stats/rebind
//! per the ticket). No per-datagram calls: media stays a skeleton this
//! ticket (the direct-ByteBuffer ring is carried over but not activated).
//!
//! Kotlin side: `dev.audioshare.v2.ClientNative` (the package/class name is
//! load-bearing: the exported symbols below embed it).
//!
//! JNIEnv BY VALUE — the prototype's recorded lesson: jni 0.21 with `&mut
//! JNIEnv` failed every createSession on device; by-value is required.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use jni::objects::{JByteBuffer, JClass, JObject, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::JNIEnv;

use crate::session::{RingAttachment, Session, SessionConfig, SessionIdentity};
use crate::signer::{IdentitySigner, JniSigner, SharedSigner};

/// Handle registry: Kotlin holds an opaque i64; the map keeps the Arc alive.
static SESSIONS: std::sync::OnceLock<Mutex<HashMap<i64, Arc<Session>>>> =
    std::sync::OnceLock::new();
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn sessions() -> &'static Mutex<HashMap<i64, Arc<Session>>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_session(handle: i64) -> Result<Arc<Session>, String> {
    sessions()
        .lock()
        .unwrap()
        .get(&handle)
        .cloned()
        .ok_or_else(|| format!("unknown session handle {handle}"))
}

fn jstring_or_empty(env: JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Run a fallible closure, translating a panic into an Err so nothing
/// unwinds across the FFI boundary.
fn guard<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => Err(match p.downcast_ref::<&str>() {
            Some(s) => format!("native panic: {s}"),
            None => "native panic (non-string payload)".to_string(),
        }),
    }
}

fn get_string(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    env.get_string(s)
        .map(|x| x.to_string_lossy().into_owned())
        .map_err(|e| format!("string: {e}"))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("hex value has odd length".to_string());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for i in 0..value.len() / 2 {
        out.push(
            u8::from_str_radix(&value[2 * i..2 * i + 2], 16)
                .map_err(|_| format!("invalid hex at byte {i}"))?,
        );
    }
    Ok(out)
}

fn new_handle(session: Session) -> i64 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    sessions().lock().unwrap().insert(id, Arc::new(session));
    id
}

fn build_identity_for_test(
    pin: String,
    server_public_key: Vec<u8>,
) -> Result<SessionIdentity, String> {
    Ok(SessionIdentity::Auth {
        pinned_spki_hex: pin,
        server_public_key,
        signer: crate::session::test_support_signer()?,
    })
}

/// The Kotlin `ClientSigner` may be any implementation; this wrapper turns a
/// failed upcall into a typed session error instead of a panic.
fn kotlin_signer(env: &mut JNIEnv, signer: &JObject) -> Result<SharedSigner, String> {
    let signer = JniSigner::new(env, signer)?;
    let _ = signer.public_key_sec1()?;
    Ok(Arc::new(signer))
}

/// createSession(host, port, pinnedSpkiHex, serverPublicKeyHex, clientName)
/// → handle (>0) or 0. The pin + server key are the test-only pre-provisioned
/// Server identity material (test config; production pairing is phase 6).
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_createSession(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
    pinned_spki_sha256_hex: JString,
    server_public_key_hex: JString,
    client_name: JString,
) -> jlong {
    #[cfg(target_os = "android")]
    crate::init_android_logger();
    match guard(|| {
        let host = get_string(&mut env, &host)?;
        let pin = get_string(&mut env, &pinned_spki_sha256_hex)?;
        let server_key_hex = get_string(&mut env, &server_public_key_hex)?;
        let client_name = get_string(&mut env, &client_name)?;
        if port <= 0 || port > 65535 {
            return Err(format!("port {port} out of range"));
        }
        let server_public_key = decode_hex(&server_key_hex)?;
        let identity = build_identity_for_test(pin, server_public_key)?;
        let session = Session::new(SessionConfig {
            host,
            port: port as u16,
            client_name,
            identity,
        })
        .map_err(|e| format!("session: {e}"))?;
        Ok(new_handle(session))
    }) {
        Ok(id) => id,
        Err(e) => {
            log::error!("createSession failed: {e}");
            0
        }
    }
}

/// createAuthSession(host, port, pinnedSpkiHex, serverPublicKeyHex,
/// clientName, signer) → handle (>0) or 0.
///
/// Production paired reconnection: the private key stays in Android Keystore;
/// `signer` is the Kotlin `dev.audioshare.v2.ClientSigner`.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_createAuthSession(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
    pinned_spki_sha256_hex: JString,
    server_public_key_hex: JString,
    client_name: JString,
    signer: JObject,
) -> jlong {
    #[cfg(target_os = "android")]
    crate::init_android_logger();
    match guard(|| {
        let host = get_string(&mut env, &host)?;
        let pin = get_string(&mut env, &pinned_spki_sha256_hex)?;
        let server_key_hex = get_string(&mut env, &server_public_key_hex)?;
        let client_name = get_string(&mut env, &client_name)?;
        if port <= 0 || port > 65535 {
            return Err(format!("port {port} out of range"));
        }
        let server_public_key = decode_hex(&server_key_hex)?;
        if server_public_key.len() != 32 {
            return Err("server public key must be 32 bytes".to_string());
        }
        let signer = kotlin_signer(&mut env, &signer)?;
        let session = Session::new(SessionConfig {
            host,
            port: port as u16,
            client_name,
            identity: SessionIdentity::Auth {
                pinned_spki_hex: pin,
                server_public_key,
                signer,
            },
        })
        .map_err(|e| format!("session: {e}"))?;
        Ok(new_handle(session))
    }) {
        Ok(id) => id,
        Err(e) => {
            log::error!("createAuthSession failed: {e}");
            0
        }
    }
}

/// createPairingSession(host, port, mode, pinnedSpkiHex, serverPublicKeyHex,
/// nonceHex, displayName, signer) → handle (>0) or 0.
///
/// `mode`: 1 = QR, 2 = SAS. QR requires the pin + Server key + 16-byte nonce;
/// SAS passes empty pin/key/nonce. The session sends `PairingRequest` instead
/// of `AuthInit`.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_createPairingSession(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
    mode: jint,
    pinned_spki_sha256_hex: JString,
    server_public_key_hex: JString,
    pairing_nonce_hex: JString,
    display_name: JString,
    signer: JObject,
) -> jlong {
    #[cfg(target_os = "android")]
    crate::init_android_logger();
    match guard(|| {
        let host = get_string(&mut env, &host)?;
        let pin = get_string(&mut env, &pinned_spki_sha256_hex)?;
        let server_key_hex = get_string(&mut env, &server_public_key_hex)?;
        let nonce_hex = get_string(&mut env, &pairing_nonce_hex)?;
        let display_name = get_string(&mut env, &display_name)?;
        if port <= 0 || port > 65535 {
            return Err(format!("port {port} out of range"));
        }
        if display_name.is_empty() || display_name.len() > 63 {
            return Err("display name must be 1..=63 bytes".to_string());
        }
        let signer = kotlin_signer(&mut env, &signer)?;
        let identity = match mode {
            1 => {
                let server_public_key = decode_hex(&server_key_hex)?;
                if server_public_key.len() != 32 {
                    return Err("server public key must be 32 bytes".to_string());
                }
                let nonce = decode_hex(&nonce_hex)?;
                if nonce.len() != 16 {
                    return Err("QR pairing nonce must be 16 bytes".to_string());
                }
                if pin.len() != 64 {
                    return Err("QR pin must be 64 hex chars".to_string());
                }
                SessionIdentity::PairQr {
                    pinned_spki_hex: pin,
                    server_public_key,
                    nonce,
                    display_name,
                    signer,
                }
            }
            2 => {
                if !pin.is_empty() || !server_key_hex.is_empty() || !nonce_hex.is_empty() {
                    return Err("SAS pairing takes no pin, server key, or nonce".to_string());
                }
                SessionIdentity::PairSas {
                    display_name,
                    signer,
                }
            }
            other => return Err(format!("unknown pairing mode {other}")),
        };
        let session = Session::new(SessionConfig {
            host,
            port: port as u16,
            client_name: display_name_for_config(&identity),
            identity,
        })
        .map_err(|e| format!("session: {e}"))?;
        Ok(new_handle(session))
    }) {
        Ok(id) => id,
        Err(e) => {
            log::error!("createPairingSession failed: {e}");
            0
        }
    }
}

fn display_name_for_config(identity: &SessionIdentity) -> String {
    match identity {
        SessionIdentity::Auth { .. } => "paired-client".to_string(),
        SessionIdentity::PairQr { display_name, .. }
        | SessionIdentity::PairSas { display_name, .. } => display_name.clone(),
    }
}

/// confirmPairing(handle): the user confirmed the displayed SAS/fingerprint.
/// Queues `PairingConfirm` on the control loop. False when there is no
/// verified pairing awaiting confirmation.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_confirmPairing(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    guard(|| {
        let session = get_session(handle)?;
        session.confirm_pairing().map(|_| true)
    })
    .map_or(0, |b| b as jboolean)
}

/// getPairingState(handle): JSON pairing display material (mode, phase, SAS,
/// fingerprint, client id, error). Empty for a non-pairing session.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_getPairingState(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    match guard(|| {
        let session = get_session(handle)?;
        Ok(session.pairing_json())
    }) {
        Ok(s) => jstring_or_empty(env, &s),
        Err(_) => jstring_or_empty(env, ""),
    }
}

/// getCredentialSummary(handle): JSON public credential after
/// PairingResult(OK), for Kotlin persistence. Empty before success.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_getCredentialSummary(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    match guard(|| {
        let session = get_session(handle)?;
        Ok(session.credential_json())
    }) {
        Ok(s) => jstring_or_empty(env, &s),
        Err(_) => jstring_or_empty(env, ""),
    }
}

/// connect(handle, bindLocalAddress) — non-blocking; "" for system default
/// routing. The bind hint must be OFF (noBindHint) when the server is
/// reached through a VPN overlay (prototype device finding).
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_connect(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    bind_local_address: JString,
) -> jboolean {
    guard(|| {
        let s = get_session(handle)?;
        let bind: Option<std::net::IpAddr> = {
            let addr = get_string(&mut env, &bind_local_address)?;
            if addr.is_empty() {
                None
            } else {
                Some(
                    addr.parse()
                        .map_err(|_| format!("bad bind address {addr:?}"))?,
                )
            }
        };
        s.connect(bind).map(|_| true)
    })
    .map_or(0, |b| b as jboolean)
}

/// stop(handle): Close(701), transport close, no reconnect (§6.3).
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_stop(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    let _ = guard(|| {
        if let Ok(session) = get_session(handle) {
            session.stop();
        }
        Ok::<(), String>(())
    });
}

/// destroy(handle): stop + drop the registry entry (runtime shuts down).
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_destroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    let _ = guard(|| {
        if let Ok(session) = get_session(handle) {
            session.stop();
        }
        sessions().lock().unwrap().remove(&handle);
        Ok::<(), String>(())
    });
}

/// getState(handle): IDLE/AUTHENTICATING/NEGOTIATING/READY/PLAYING/CLOSING/
/// CLOSED; "" on unknown handle.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_getState(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    match guard(|| {
        let session = get_session(handle)?;
        Ok(session.state_name().to_string())
    }) {
        Ok(s) => jstring_or_empty(env, &s),
        Err(_) => jstring_or_empty(env, ""),
    }
}

/// getStats(handle): counters JSON (stable shape, no secrets). "" on error.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_getStats(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    match guard(|| {
        let session = get_session(handle)?;
        Ok(session.stats_json())
    }) {
        Ok(s) => jstring_or_empty(env, &s),
        Err(_) => jstring_or_empty(env, ""),
    }
}

/// rebind(handle, fd): network-change migration. FD OWNERSHIP: the fd
/// transfers to native on call — success AND failure. Returns false on
/// failure (caller falls back to teardown + bounded-backoff reconnect).
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_rebind(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    fd: jint,
) -> jboolean {
    guard(|| {
        let session = get_session(handle)?;
        session.rebind(fd).map(|_| true)
    })
    .map_or(0, |b| b as jboolean)
}

/// attachMediaRing(handle, directBuffer): attach the JVM's direct ByteBuffer
/// as the media ring, ONCE per session (media never crosses JNI per datagram).
/// Returns false when the buffer is not direct/empty or a ring is attached.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_attachMediaRing(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    buffer: JByteBuffer,
) -> jboolean {
    guard(|| {
        let session = get_session(handle)?;
        let base = env
            .get_direct_buffer_address(&buffer)
            .map_err(|e| format!("direct buffer: {e}"))?;
        let capacity = env
            .get_direct_buffer_capacity(&buffer)
            .map_err(|e| format!("direct buffer capacity: {e}"))?;
        if base.is_null() || capacity == 0 {
            return Err("media ring must be a non-empty direct ByteBuffer".to_owned());
        }
        session.attach_ring(RingAttachment { base, capacity })
    })
    .map_or(0, |b| b as jboolean)
}

/// ringPosition(handle): bytes written to the ring so far (-1 without a ring).
/// Kotlin reads from this monotonic cursor; it resyncs when it falls more
/// than one buffer behind.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_ringPosition(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jlong {
    guard(|| {
        let session = get_session(handle)?;
        Ok(session.ring_position())
    })
    .unwrap_or(-1)
}

/// mediaFormatJson(handle): the active instance/generation/geometry as JSON,
/// or null ("") before PLAYING. Kotlin rebuilds the AudioTrack only when the
/// instance or geometry changes.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_mediaFormatJson(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    match guard(|| {
        let session = get_session(handle)?;
        Ok(session.media_format_json())
    }) {
        Ok(s) => jstring_or_empty(env, &s),
        Err(_) => jstring_or_empty(env, ""),
    }
}

/// recordPlayback(handle, submittedBytes, underruns, bufferCurrentMs): the
/// bytes the AudioTrack actually accepted (and its underruns/fill). This is
/// the only playback evidence the Client reports (§9).
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_recordPlayback(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    submitted_bytes: jlong,
    underruns: jlong,
    buffer_current_ms: jint,
) {
    let _ = guard(|| {
        let session = get_session(handle)?;
        session.record_playback(
            submitted_bytes.max(0) as u64,
            underruns.max(0) as u64,
            buffer_current_ms.max(0) as u32,
        );
        Ok::<(), String>(())
    });
}

/// recordTrim(handle, frames): frames dropped by the Kotlin reader's §8
/// hard-limit re-anchor. M5-04 counts the deletion as `ReAnchor` concealment
/// and advances the submitted/playout position over the skipped media.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_recordTrim(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    frames: jlong,
) {
    let _ = guard(|| {
        let session = get_session(handle)?;
        session.record_trim(frames.max(0) as u64);
        Ok::<(), String>(())
    });
}

/// recordSoftTrim(handle, frames): frames dropped by the Kotlin reader's soft
/// target trim (backlog above the §8 target). Counted separately from
/// concealment — normal buffer steering, never `ReAnchor`.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_recordSoftTrim(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    frames: jlong,
) {
    let _ = guard(|| {
        let session = get_session(handle)?;
        session.record_soft_trim(frames.max(0) as u64);
        Ok::<(), String>(())
    });
}

/// recordUnderrun(handle, frames): the playback loop had no data for one
/// playback period and fed `frames` of silence. Counts the §8 underrun-fill
/// concealment; the platform `underruns` counter stays a separate §9 field.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_recordUnderrun(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    frames: jlong,
) {
    let _ = guard(|| {
        let session = get_session(handle)?;
        session.record_underrun(frames.max(0) as u64);
        Ok::<(), String>(())
    });
}

/// jitterTargetMs(handle): the active §8 jitter-buffer target (60 ms by
/// default). Exposed so Kotlin does not duplicate the native constant.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_jitterTargetMs(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    guard(|| {
        let session = get_session(handle)?;
        Ok(session.jitter_target_ms() as jint)
    })
    .unwrap_or(0)
}

/// jitterHardLimitMs(handle): the active §8 jitter-buffer hard limit (200 ms
/// by default).
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_jitterHardLimitMs(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    guard(|| {
        let session = get_session(handle)?;
        Ok(session.jitter_hard_limit_ms() as jint)
    })
    .unwrap_or(0)
}

/// driftPpm(handle): the §8 capture-vs-playback drift estimate in ppm, or 0
/// until a full 10 s window exists. The accepted §9 `PlaybackDiagnostics` has
/// no drift field; this accessor plus the `playback.drift` log line is the
/// non-protocol L1 channel.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_driftPpm(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    guard(|| {
        let session = get_session(handle)?;
        Ok(session.drift_ppm().unwrap_or(0))
    })
    .unwrap_or(0)
}

/// driftCorrectionPpm(handle): the smooth, ±100 ppm-clamped playback-rate
/// correction the player should apply with `AudioTrack.setPlaybackRate`.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_driftCorrectionPpm(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    guard(|| {
        let session = get_session(handle)?;
        Ok(session.drift_correction_ppm())
    })
    .unwrap_or(0)
}

/// takeDriftIntervention(handle): whole-frame adjustment the player must apply
/// now (+ insert silence, − delete frames). Taking it clears the pending value;
/// the §8 concealment is counted only when the player acknowledges it.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_takeDriftIntervention(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    guard(|| {
        let session = get_session(handle)?;
        Ok(session.take_drift_intervention())
    })
    .unwrap_or(0)
}

/// recordDriftIntervention(handle, frames): the player applied the adjustment
/// (+ inserted, − deleted). Counts one §8 drift concealment with the signed
/// frame delta.
#[no_mangle]
pub extern "system" fn Java_dev_audioshare_v2_ClientNative_recordDriftIntervention(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    frames: jint,
) {
    let _ = guard(|| {
        let session = get_session(handle)?;
        session.record_drift_intervention(frames);
        Ok::<(), String>(())
    });
}
