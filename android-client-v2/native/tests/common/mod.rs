//! Shared deterministic test helpers for the native integration tests.
//!
//! Integration-test crates are separate binaries, so each file that needs
//! these must declare `mod common;`. Everything is intentionally tiny and
//! dependency-free.

#![allow(dead_code)]

/// splitmix64: reproducible from the seed, no external dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// `true` with probability `permille / 1000`.
    pub fn chance(&mut self, permille: u64) -> bool {
        self.below(1000) < permille
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next_u64() as u8).collect()
    }
}

/// `M5_FUZZ_SEED` when set, else `default`. Prints the seed and `label` so a
/// failing run can be replayed.
pub fn seed(default: u64, label: &str) -> u64 {
    let seed = std::env::var("M5_FUZZ_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default);
    eprintln!("{label} seed = {seed}");
    seed
}
