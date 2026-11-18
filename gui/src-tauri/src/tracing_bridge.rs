//! Bridge the shared Server crates' `tracing` events into `server.log`.
//!
//! The v2 transport and runtime log structured, secret-free session events at
//! INFO (ticket 33). The GUI predates them and owns its own logger, so without
//! a subscriber those events were silently dropped: the Windows acceptance run
//! could not see pairing, session, `ClientClosed(701)`, or revoke lines in
//! `server.log` even though `as-cmd` emitted them.
//!
//! Only the Server's own targets are forwarded (`v2_transport`, `v2_runtime`);
//! QUIC/TLS internals stay out of the operator log.

use std::sync::Arc;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry;

/// Where a formatted line goes. Production writes to [`crate::logging`];
/// tests capture into memory.
type Sink = Arc<dyn Fn(Level, &str) + Send + Sync>;

/// A `tracing` layer that renders one `server.log` line per event.
pub struct BridgeLayer {
    sink: Sink,
}

impl BridgeLayer {
    /// Build a layer that hands each rendered line to `sink`.
    pub fn new(sink: Sink) -> Self {
        Self { sink }
    }

    /// Only the Server's own crates are operator-relevant; third-party QUIC
    /// and TLS events would flood `server.log`.
    fn target_allowed(target: &str) -> bool {
        target.starts_with("v2_transport") || target.starts_with("v2_runtime")
    }
}

impl<S: Subscriber> Layer<S> for BridgeLayer {
    fn enabled(&self, metadata: &Metadata<'_>, _context: Context<'_, S>) -> bool {
        Self::target_allowed(metadata.target()) && *metadata.level() <= Level::INFO
    }

    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = LineVisitor::default();
        event.record(&mut visitor);
        let mut line = visitor.message;
        if !visitor.fields.is_empty() {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&visitor.fields.join(" "));
        }
        (self.sink)(*event.metadata().level(), &line);
    }
}

#[derive(Default)]
struct LineVisitor {
    message: String,
    fields: Vec<String>,
}

impl LineVisitor {
    fn push(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}

impl Visit for LineVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        // `%value` fields arrive here as `format_args!`, which Debug renders
        // exactly as Display (no quotes); literal string fields keep quotes.
        self.push(field, format!("{value:?}"));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, value.to_string());
    }
}

/// Install the bridge as the process-wide subscriber.
///
/// Call once, after [`crate::logging::init`]. A second call is a no-op: an
/// already-installed global subscriber is left alone.
pub fn install() {
    let layer = BridgeLayer::new(Arc::new(|level, line| match level {
        Level::ERROR => crate::logging::error(line),
        _ => crate::logging::info(line),
    }));
    let _ = tracing::subscriber::set_global_default(registry().with(layer));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn capture() -> (Arc<Mutex<Vec<String>>>, BridgeLayer) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink_lines = Arc::clone(&lines);
        let layer = BridgeLayer::new(Arc::new(move |level, line| {
            sink_lines
                .lock()
                .expect("capture lock")
                .push(format!("{level} {line}"));
        }));
        (lines, layer)
    }

    fn with_capture(run: impl FnOnce()) -> Vec<String> {
        let (lines, layer) = capture();
        let subscriber = registry().with(layer);
        tracing::subscriber::with_default(subscriber, run);
        let captured = lines.lock().expect("capture lock").clone();
        captured
    }

    #[test]
    fn server_events_become_one_line_without_quotes() {
        let lines = with_capture(|| {
            tracing::info!(
                target: "v2_transport::session_log",
                event = %"session.closed",
                client_id = %"cf7940b474310246",
                code = 701u32,
                "v2 session closed"
            );
        });
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert!(line.starts_with("INFO "), "{line}");
        assert!(line.contains("v2 session closed"), "{line}");
        assert!(line.contains("event=session.closed"), "{line}");
        assert!(line.contains("client_id=cf7940b474310246"), "{line}");
        assert!(line.contains("code=701"), "{line}");
    }

    #[test]
    fn third_party_targets_are_not_forwarded() {
        let lines = with_capture(|| {
            tracing::info!(target: "quinn::connection", "noise");
        });
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn debug_events_are_not_forwarded() {
        let lines = with_capture(|| {
            tracing::debug!(target: "v2_transport::driver", "detail");
        });
        assert!(lines.is_empty(), "{lines:?}");
    }
}
