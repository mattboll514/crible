// SPDX-License-Identifier: Apache-2.0

//! The two low-level kernels of the dense branch: Hamming distance over binary
//! codes and the exact fp16 by f32 dot product of the rescoring step.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`NOYAUX-SIMD`),
//! `docs/07-stack-et-deploiement.md` ("Noyaux SIMD", micro-benchmark),
//! `docs/03-recherche.md` (branche dense) and
//! `crates/crible-store/src/codes.rs` (a code is a little-endian `u64` word
//! sequence, `ceil(dims / 64)` words, no header).
//!
//! # Contract
//!
//! Both kernels are total: they read `min(a.len(), b.len())` elements, so a
//! caller that passes slices of different lengths gets the contribution of the
//! common prefix rather than a panic or a silent read out of bounds. The
//! production callers always pass equal lengths (a passage code and a query
//! code, a stored vector and a query vector), and a unit test fixes the common
//! prefix rule.
//!
//! `hamming` counts the set bits of the exclusive or, so its result is exact
//! for every input, on every backend. `dot` accumulates in `f64` on every
//! backend: the products are the `f32` products of a widened `f16` component
//! and a query `f32`, and only the summation order differs between backends.
//! The difference between the scalar and the SIMD result is therefore a
//! rounding on the last `f64` bits, far below the `1e-5` tolerance the task
//! sheet requires.
//!
//! # Backends
//!
//! The implementation is chosen once, on first use, by a runtime detection
//! stored in a [`OnceLock`]: NEON on `aarch64`, AVX2 on `x86_64` (plus `f16c`
//! for the dot product, which converts fp16 to fp32 in one instruction), and
//! the scalar reference implementation everywhere else. NEON is part of the
//! `aarch64` baseline, but the detection still runs so that one binary behaves
//! the same way on every CPU that claims the architecture.
//!
//! The choice between a dedicated crate and these hand-written intrinsics is
//! argued in `benches/kernels.rs`, which records the measured numbers.

use std::sync::OnceLock;

use half::f16;

/// Portable reference implementation, also the oracle of the tests and the
/// baseline of the benchmark.
pub mod scalar;

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(target_arch = "x86_64")]
mod x86;

/// The implementation a kernel runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The portable reference implementation.
    Scalar,
    /// `aarch64` Advanced SIMD.
    Neon,
    /// `x86_64` AVX2.
    Avx2,
}

impl Backend {
    /// Stable lowercase name, used by the benchmark output and the tests.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Neon => "neon",
            Self::Avx2 => "avx2",
        }
    }
}

type HammingFn = fn(&[u64], &[u64]) -> u64;
type DotFn = fn(&[f16], &[f32]) -> f32;

/// The selected implementation of each kernel, with the name of its backend.
///
/// The two kernels are selected independently: a CPU may have NEON without the
/// fp16 extension, in which case Hamming is vectorised and the dot product
/// falls back to the scalar loop.
struct Selection {
    hamming: HammingFn,
    dot: DotFn,
    hamming_backend: Backend,
    dot_backend: Backend,
}

/// Hamming distance between two binary codes, as the number of differing bits.
///
/// The caller passes the `u64` words of `crible-store`'s `codes.bin`, in
/// little-endian order. The result is exact, whatever the backend.
#[must_use]
pub fn hamming(a: &[u64], b: &[u64]) -> u64 {
    (selected().hamming)(a, b)
}

/// The selected Hamming implementation, to resolve the runtime dispatch once.
///
/// [`hamming`] looks the selection up through a [`OnceLock`] on every call,
/// which costs about 1.4 ns, some 40 % of a 12-word Hamming (`NOYAUX-SIMD`,
/// `benches/kernels.rs`). A scan that hashes millions of codes therefore
/// resolves the pointer **once** with this function and calls it for every
/// passage, instead of paying the dispatch per passage.
#[must_use]
pub fn hamming_function() -> fn(&[u64], &[u64]) -> u64 {
    selected().hamming
}

/// Exact dot product of a stored `f16` vector and a query `f32` vector.
///
/// `a` holds the reference vector read from a segment (fp16), `b` the
/// normalised query (f32). The vectors are normalised upstream, so this
/// product is the cosine the rescoring step ranks on.
#[must_use]
pub fn dot(a: &[f16], b: &[f32]) -> f32 {
    (selected().dot)(a, b)
}

/// Backend the Hamming kernel runs on, for the benchmark output.
#[must_use]
pub fn hamming_backend() -> Backend {
    selected().hamming_backend
}

/// Backend the dot-product kernel runs on, for the benchmark output.
#[must_use]
pub fn dot_backend() -> Backend {
    selected().dot_backend
}

fn selected() -> &'static Selection {
    static SELECTED: OnceLock<Selection> = OnceLock::new();
    SELECTED.get_or_init(choose)
}

#[cfg(target_arch = "aarch64")]
fn choose() -> Selection {
    if std::arch::is_aarch64_feature_detected!("neon") {
        // The NEON dot product converts fp16 with the `fp16` extension, so it
        // needs one more check than Hamming. Apple silicon and every ARMv8.2+
        // core have it; ARMv8.0 cores keep the scalar loop.
        if std::arch::is_aarch64_feature_detected!("fp16") {
            Selection {
                hamming: neon::hamming,
                dot: neon::dot,
                hamming_backend: Backend::Neon,
                dot_backend: Backend::Neon,
            }
        } else {
            Selection {
                hamming: neon::hamming,
                dot: scalar::dot,
                hamming_backend: Backend::Neon,
                dot_backend: Backend::Scalar,
            }
        }
    } else {
        scalar_selection()
    }
}

#[cfg(target_arch = "x86_64")]
fn choose() -> Selection {
    if !std::arch::is_x86_feature_detected!("avx2") {
        return scalar_selection();
    }
    // The fp16 to fp32 conversion is an F16C instruction, not an AVX2 one: a
    // CPU may have AVX2 without F16C, and then only Hamming is vectorised.
    let f16c = std::arch::is_x86_feature_detected!("f16c");
    Selection {
        hamming: x86::hamming,
        dot: if f16c { x86::dot } else { scalar::dot },
        hamming_backend: Backend::Avx2,
        dot_backend: if f16c { Backend::Avx2 } else { Backend::Scalar },
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
fn choose() -> Selection {
    scalar_selection()
}

fn scalar_selection() -> Selection {
    Selection {
        hamming: scalar::hamming,
        dot: scalar::dot,
        hamming_backend: Backend::Scalar,
        dot_backend: Backend::Scalar,
    }
}
