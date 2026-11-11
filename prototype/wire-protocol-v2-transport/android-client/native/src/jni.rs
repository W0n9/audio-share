//! JNI surface — LOW-FREQUENCY ONLY. No per-datagram calls, no per-packet
//! callbacks: media flows through the attached direct ByteBuffer (shared
//! memory) and counters are polled via getStats.
//!
//! Kotlin side: `dev.prototype.wireproto.ProtoNative` (package is part of the
//! exported symbol names below). DEVICE ONLY — compiled by cargo test but the
//! JNI functions themselves are UNTESTED on host (no JVM fixture in this crate).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use jni::objects::{JByteBuffer, JClass, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::JNIEnv;

use crate::session::{Session, SessionConfig};

/// Prototype handle registry: Kotlin holds an opaque i64; the map keeps the
/// Arc alive. NOT production-grade (no weak refs / liveness callbacks).
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

fn jstring_or_empty(env: &mut JNIEnv, s: &str) -> jni::sys::jstring {
    env.new_string(s)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Run a fallible closure, translating a panic into an Err so nothing unwinds
/// across the FFI boundary.
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

#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_createSession(
    env: &mut JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
    pinned_spki_sha256_hex: JString,
    credential: JString,
) -> jlong {
    #[cfg(target_os = "android")]
    crate::init_android_logger();
    guard(|| {
        let host: String = env
            .get_string(&host)
            .map(|s| s.to_string_lossy().into_owned())
            .map_err(|e| format!("host: {e}"))?;
        let pin: String = env
            .get_string(&pinned_spki_sha256_hex)
            .map(|s| s.to_string_lossy().into_owned())
            .map_err(|e| format!("pin: {e}"))?;
        let credential: String = env
            .get_string(&credential)
            .map(|s| s.to_string_lossy().into_owned())
            .map_err(|e| format!("credential: {e}"))?;
        if port <= 0 || port > 65535 {
            return Err(format!("port {port} out of range"));
        }
        if credential.len() > 1024 {
            // keep the credential out of logs; only its length is ever reported
            return Err("credential too long".into());
        }
        let session = Session::new(SessionConfig {
            host,
            port: port as u16,
            pinned_spki_hex: pin,
            credential,
        })
        .map_err(|e| format!("session: {e}"))?;
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        sessions().lock().unwrap().insert(id, Arc::new(session));
        Ok(id)
    })
    .unwrap_or(0)
}

#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_connect(
    env: &mut JNIEnv,
    _class: JClass,
    handle: jlong,
    bind_local_address: JString,
) -> jboolean {
    guard(|| {
        let s = get_session(handle)?;
        let bind: Option<std::net::IpAddr> = {
            let addr: String = env
                .get_string(&bind_local_address)
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if addr.is_empty() {
                None
            } else {
                Some(addr.parse().map_err(|_| format!("bad bind address {addr:?}"))?)
            }
        };
        s.connect(bind).map(|_| true)
    })
    .map_or(0, |b| b as jboolean)
}

#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_subscribe(
    _env: &mut JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    guard(|| {
        let session = get_session(handle)?;
        session.subscribe().map(|_| true)
    })
    .map_or(0, |b| b as jboolean)
}

#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_unsubscribe(
    _env: &mut JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    guard(|| {
        let session = get_session(handle)?;
        session.unsubscribe().map(|_| true)
    })
    .map_or(0, |b| b as jboolean)
}

#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_stop(
    _env: &mut JNIEnv,
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

/// Destroy the session: drop the registry entry, runtime shuts down with it.
#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_destroy(
    _env: &mut JNIEnv,
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

/// Current state name (IDLE/AUTHENTICATING/READY/PLAYING/CLOSED). Empty string
/// on unknown handle.
#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_getState(
    env: &mut JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jni::sys::jstring {
    match guard(|| {
        let session = get_session(handle)?;
        let st = session.shared.state.lock().unwrap().clone();
        Ok(st.state_name().to_string())
    }) {
        Ok(s) => jstring_or_empty(env, &s),
        Err(_) => jstring_or_empty(env, ""),
    }
}

/// Full stats JSON (see crate::stats for the contract). Empty string on error.
#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_getStats(
    env: &mut JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jni::sys::jstring {
    match guard(|| {
        let session = get_session(handle)?;
        Ok(crate::stats::render(&session.snapshot()))
    }) {
        Ok(s) => jstring_or_empty(env, &s),
        Err(_) => jstring_or_empty(env, ""),
    }
}

/// Attach ONE direct ByteBuffer (Kotlin: ByteBuffer.allocateDirect). The native
/// side writes PCM into it; Kotlin reads via write_pos from getStats. Returns
/// false if already attached or the buffer is not direct.
#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_attachBuffer(
    env: &mut JNIEnv,
    _class: JClass,
    handle: jlong,
    buffer: JByteBuffer,
) -> jboolean {
    guard(|| {
        let session = get_session(handle)?;
        let base = env
            .get_direct_buffer_address(&buffer)
            .map_err(|e| format!("direct address: {e}"))?;
        let capacity = env
            .get_direct_buffer_capacity(&buffer)
            .map_err(|e| format!("direct capacity: {e}"))?;
        if base.is_null() || capacity == 0 {
            return Err("buffer is not direct (null address)".to_string());
        }
        // RING_CAPACITY guard: the session expects at least one datagram of headroom
        if capacity < 8192 {
            return Err(format!("buffer capacity {capacity} too small"));
        }
        unsafe { session.attach_buffer(base, capacity).map(|_| true) }
    })
    .map_or(0, |b| b as jboolean)
}

/// Network-change migration: rebind the QUIC endpoint onto a UDP socket the
/// JVM created and route-pinned to the active Android `Network`
/// (`Network.bindSocket(DatagramSocket)`), passed as the raw fd
/// (`ParcelFileDescriptor.fromDatagramSocket(...).getFd()`). Low-frequency
/// (once per network change). Returns false when the rebind failed — the
/// caller then falls back to teardown + bounded-backoff reconnect.
///
/// FD OWNERSHIP: the fd transfers to native on call — on success AND on
/// failure (the socket is dropped, closing the fd, if the rebind errors, and
/// the endpoint owns it on success). The JVM side must never close or reuse
/// the fd (or the socket/pfd) after this call.
#[no_mangle]
pub extern "system" fn Java_dev_prototype_wireproto_ProtoNative_rebind(
    _env: &mut JNIEnv,
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
