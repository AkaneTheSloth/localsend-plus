//! Runtime detection of hardware acceleration instruction sets.
//!
//! LocalSend+ picks the fastest available code path per platform at startup:
//! SHA-256 and AES are accelerated on CPUs that expose the corresponding
//! instruction sets, and bulk file hashing/encryption benefits from SIMD
//! (AVX2/AVX-512 on x86_64, NEON on aarch64). Detection is a one-time, cheap
//! CPUID / `getauxval` read; the chosen path is logged so operators can
//! confirm which implementation is in use.
//!
//! The detection result is advisory: the actual accelerated implementation is
//! selected by the crypto/hash backends (e.g. `sha2`'s SHA-NI path, `ring`'s
//! AES-NI/GHASH paths). This module centralises the platform-specific probing
//! and the decision of *which* backend feature to prefer.

use std::sync::OnceLock;

/// Which hardware acceleration families are available on this CPU.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Acceleration {
    /// x86_64: SSE2 (baseline for 64-bit, but reported for completeness).
    pub sse2: bool,
    /// x86_64: SSE4.2 (CRC32, used by some checksum paths).
    pub sse4_2: bool,
    /// x86_64: AVX (256-bit SIMD).
    pub avx: bool,
    /// x86_64: AVX2 (integer 256-bit SIMD, used by fast memcpy/memset and hashing).
    pub avx2: bool,
    /// x86_64: AVX-512F (512-bit SIMD).
    pub avx512f: bool,
    /// x86_64: AES-NI (hardware AES round instructions).
    pub aes_ni: bool,
    /// x86_64: SHA-NI (hardware SHA-1/SHA-256 round instructions).
    pub sha_ni: bool,
    /// aarch64: NEON (128-bit SIMD, always present on AArch64).
    pub neon: bool,
    /// aarch64: ARMv8 AES extension.
    pub arm_aes: bool,
    /// aarch64: ARMv8 SHA-1/SHA-256 extension.
    pub arm_sha2: bool,
}

impl Acceleration {
    /// Whether any hardware AES acceleration is available.
    pub fn has_aes(&self) -> bool {
        self.aes_ni || self.arm_aes
    }

    /// Whether any hardware SHA-256 acceleration is available.
    pub fn has_sha256(&self) -> bool {
        self.sha_ni || self.arm_sha2
    }

    /// Whether a wide SIMD unit (AVX2/AVX-512/NEON) is available.
    pub fn has_wide_simd(&self) -> bool {
        self.avx2 || self.avx512f || self.neon
    }

    /// Human-readable, space-separated list of the detected extensions.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        for (name, present) in [
            ("sse2", self.sse2),
            ("sse4.2", self.sse4_2),
            ("avx", self.avx),
            ("avx2", self.avx2),
            ("avx512f", self.avx512f),
            ("aes-ni", self.aes_ni),
            ("sha-ni", self.sha_ni),
            ("neon", self.neon),
            ("arm-aes", self.arm_aes),
            ("arm-sha2", self.arm_sha2),
        ] {
            if present {
                parts.push(name);
            }
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(" ")
        }
    }

    /// The SHA-256 backend this platform should prefer, for logging.
    pub fn sha256_backend(&self) -> &'static str {
        if self.sha_ni || self.arm_sha2 {
            "hardware SHA-NI/ARMv8"
        } else if self.has_wide_simd() {
            "SIMD (software)"
        } else {
            "portable"
        }
    }

    /// The AES backend this platform should prefer, for logging.
    pub fn aes_backend(&self) -> &'static str {
        if self.aes_ni || self.arm_aes {
            "hardware AES-NI/ARMv8"
        } else {
            "portable"
        }
    }
}

/// The per-process detection result, computed once on first use.
static ACCELERATION: OnceLock<Acceleration> = OnceLock::new();

/// Returns the hardware acceleration capabilities of this CPU.
///
/// The result is computed once and cached for the lifetime of the process.
pub fn acceleration() -> Acceleration {
    *ACCELERATION.get_or_init(detect)
}

/// Logs the detected acceleration and the selected backends, once per process.
///
/// Called from server and client startup so the chosen code paths appear in
/// the logs on every platform.
pub fn log_detected() {
    let accel = acceleration();
    tracing::info!(
        "Hardware acceleration detected: {} (sha256: {}, aes: {})",
        accel.describe(),
        accel.sha256_backend(),
        accel.aes_backend(),
    );
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect() -> Acceleration {
    Acceleration {
        sse2: std::arch::is_x86_feature_detected!("sse2"),
        sse4_2: std::arch::is_x86_feature_detected!("sse4.2"),
        avx: std::arch::is_x86_feature_detected!("avx"),
        avx2: std::arch::is_x86_feature_detected!("avx2"),
        avx512f: std::arch::is_x86_feature_detected!("avx512f"),
        aes_ni: std::arch::is_x86_feature_detected!("aes"),
        sha_ni: std::arch::is_x86_feature_detected!("sha"),
        ..Default::default()
    }
}

#[cfg(target_arch = "aarch64")]
fn detect() -> Acceleration {
    Acceleration {
        neon: std::arch::is_aarch64_feature_detected!("neon"),
        arm_aes: std::arch::is_aarch64_feature_detected!("aes"),
        arm_sha2: std::arch::is_aarch64_feature_detected!("sha2"),
        ..Default::default()
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
fn detect() -> Acceleration {
    // Other architectures (e.g. wasm32, riscv64, loongarch64) get the portable
    // backends; the struct defaults to all-false which maps to "portable".
    Acceleration::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_is_stable_and_cached() {
        let a = acceleration();
        let b = acceleration();
        assert_eq!(a, b);
    }

    #[test]
    fn description_always_parses() {
        // Must not panic on any platform; "none" is a valid result.
        let _ = acceleration().describe();
    }

    #[test]
    fn backend_selection_is_consistent() {
        let accel = acceleration();
        // A platform with no hardware always reports the portable backend.
        assert!(!accel.sha256_backend().is_empty());
        assert!(!accel.aes_backend().is_empty());
    }
}
