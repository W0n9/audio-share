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

use jni::objects::{JByteBuffer, JClass, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::JNIEnv;

use crate::session::{RingAttachment, Session, SessionConfig};

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
        let mut server_public_key = Vec::with_capacity(32);
        for i in 0..server_key_hex.len() / 2 {
            server_public_key.push(
                u8::from_str_radix(&server_key_hex[2 * i..2 * i + 2], 16)
                    .map_err(|e| e.to_string())?,
            );
        }
        let session = Session::new(SessionConfig {
            host,
            port: port as u16,
            pinned_spki_hex: pin,
            server_public_key,
            client_name,
        })
        .map_err(|e| format!("session: {e}"))?;
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        sessions().lock().unwrap().insert(id, Arc::new(session));
        Ok(id)
    }) {
        Ok(id) => id,
        Err(e) => {
            log::error!("createSession failed: {e}");
            0
        }
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
        session.record_playback(submitted_bytes.max(0) as u64, underruns.max(0) as u64, buffer_current_ms.max(0) as u32);
        Ok::<(), String>(())
    });
}
