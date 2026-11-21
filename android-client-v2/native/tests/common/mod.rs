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

/// Deterministic 32-bit LCG used by the M5-12 noise fixtures.
pub struct NoiseRng(u32);

impl NoiseRng {
    pub fn new(seed: u32) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }

    /// Uniform integer in `[-bound, bound]`.
    pub fn centred(&mut self, bound: i64) -> i64 {
        if bound == 0 {
            return 0;
        }
        let span = 2 * bound + 1;
        i64::from(self.next() >> 16) % span - bound
    }
}

/// M5-12 integer-Hz playback executor, independently implemented.
///
/// This is deliberately an **oracle**, not a call into the production mapping,
/// so a production regression cannot make the test agree with itself.
///
/// `AudioTrack.setPlaybackRate` accepts whole Hz only. Round the ppm command to
/// the nearest Hz, clamp that Hz delta to the largest whole-Hz value that stays
/// at or below `max_correction_ppm`, and report the actual ppm. The production
/// controller stores integer ppm, so the actual is truncated (`4 Hz` at 44.1 kHz
/// is 90 ppm, not 90.7).
pub fn actuator_ppm_oracle(commanded_ppm: i32, max_correction_ppm: i32, sample_rate: u32) -> i32 {
    if sample_rate == 0 {
        return 0;
    }
    let rate = i64::from(sample_rate);
    let max_delta_hz = (i64::from(max_correction_ppm.max(0)) * rate / 1_000_000).max(0);
    let requested =
        (f64::from(commanded_ppm) * f64::from(sample_rate) / 1_000_000.0).round() as i64;
    let delta_hz = requested.clamp(-max_delta_hz, max_delta_hz);
    (delta_hz * 1_000_000 / rate) as i32
}
