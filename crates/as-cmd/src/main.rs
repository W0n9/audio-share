//! `as-cmd` skeleton.
//!
//! The final CLI (bind handling, endpoint listing, encoding flags, capture) is
//! implemented by a later ticket. For now the binary only proves that the
//! workspace builds without vcpkg, MSVC, or MFC.

fn main() {
    println!(
        "as-cmd {} (Server rewrite skeleton)",
        env!("CARGO_PKG_VERSION")
    );
}
