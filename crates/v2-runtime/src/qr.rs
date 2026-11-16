//! Offline QR rendering (no Google/GMS or web dependency).
//!
//! The payload is the §4.1 pairing URI. Rendering is best-effort: a payload
//! that does not fit at error-correction level M falls back to level L (§4.1),
//! and a failure leaves the copyable URI as the primary transfer path.

use qrcodegen::{QrCode, QrCodeEcc};

/// Render `text` as a self-contained SVG QR code with a four-module quiet zone.
pub fn render_svg(text: &str) -> Option<String> {
    let code = QrCode::encode_text(text, QrCodeEcc::Medium)
        .or_else(|_| QrCode::encode_text(text, QrCodeEcc::Low))
        .ok()?;
    const BORDER: i32 = 4;
    let size = code.size() + BORDER * 2;
    let mut path = String::new();
    for y in 0..code.size() {
        for x in 0..code.size() {
            if code.get_module(x, y) {
                let x = x + BORDER;
                let y = y + BORDER;
                path.push_str(&format!("M{x},{y}h1v1h-1z"));
            }
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" \
         shape-rendering=\"crispEdges\" role=\"img\"><rect width=\"100%\" height=\"100%\" \
         fill=\"#ffffff\"/><path d=\"{path}\" fill=\"#000000\"/></svg>"
    ))
}
