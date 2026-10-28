// SPDX-License-Identifier: Apache-2.0

//! NOYAUX-SIMD acceptance criterion: every SIMD kernel returns the same result
//! as the scalar reference.
//!
//! Hamming is exact on every backend, so the property asserts equality of the
//! two integers. The dot product is compared with a **relative** tolerance:
//! `1e-6` of the scalar result, plus an absolute floor of `1e-5` for a result
//! close to zero, where a relative bound would be unreachable. Both backends
//! compute the same `f32` products and differ only by the `f64` summation
//! order, but the single rounding to `f32` can put the two results one `f32`
//! ulp apart, which is `1.5e-5` on a product of about 150 and would exceed an
//! absolute bound of `1e-5`.
//!
//! The dispatched function is the one production calls, so this test exercises
//! the vector path on the machine that runs it (NEON here, AVX2 in the Linux
//! CI) and the scalar path on a CPU without the extensions. The
//! architecture-specific unit tests in `src/kernels/neon.rs` and
//! `src/kernels/x86.rs` run each `unsafe` function directly, so the vector
//! code is covered even when the dispatcher would choose otherwise.

use crible_search::kernels::{self, scalar};
use half::f16;
use proptest::prelude::*;

/// Relative tolerance of the dot product: `1e-6` of the scalar result.
const DOT_RELATIVE_TOLERANCE: f32 = 1e-6;

/// Absolute floor of that tolerance, for a result close to zero.
const DOT_ABSOLUTE_FLOOR: f32 = 1e-5;

/// Whether `simd` stays within the relative tolerance of `reference`.
fn within_tolerance(simd: f32, reference: f32) -> bool {
    (simd - reference).abs() <= DOT_RELATIVE_TOLERANCE * reference.abs() + DOT_ABSOLUTE_FLOOR
}

/// An fp16 value in `[-4, 4]`, the range a normalised component lives in.
fn half_component() -> impl Strategy<Value = f16> {
    (-4.0_f32..4.0).prop_map(f16::from_f32)
}

/// A query component in `[-4, 4]`.
fn query_component() -> impl Strategy<Value = f32> {
    -4.0_f32..4.0
}

/// Words of a binary code, from the empty code to 128 words (8 192 bits).
fn code() -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(any::<u64>(), 0..=128)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Hamming: exact equality with the scalar reference, at every length.
    #[test]
    fn hamming_simd_matches_scalar(a in code(), b in code()) {
        prop_assert_eq!(kernels::hamming(&a, &b), scalar::hamming(&a, &b));
    }

    /// Hamming: a flipped bit moves the distance to the zero code by exactly
    /// one, on the backend production uses.
    #[test]
    fn hamming_simd_counts_one_bit(mut a in code(), bit in 0_usize..8192) {
        prop_assume!(!a.is_empty());
        let zero = vec![0_u64; a.len()];
        let before: u64 = a.iter().map(|word| u64::from(word.count_ones())).sum();
        prop_assert_eq!(kernels::hamming(&a, &zero), before);
        let offset = bit % (a.len() * 64);
        let was_set = a[offset / 64] >> (offset % 64) & 1 == 1;
        a[offset / 64] ^= 1_u64 << (offset % 64);
        let expected = if was_set { before - 1 } else { before + 1 };
        prop_assert_eq!(kernels::hamming(&a, &zero), expected);
        prop_assert_eq!(kernels::hamming(&a, &a), 0);
    }

    /// Dot product: the SIMD result stays within the relative tolerance of the
    /// scalar one.
    #[test]
    fn dot_simd_matches_scalar(
        a in prop::collection::vec(half_component(), 0..=1024),
        b in prop::collection::vec(query_component(), 0..=1024),
    ) {
        let simd = kernels::dot(&a, &b);
        let reference = scalar::dot(&a, &b);
        prop_assert!(
            within_tolerance(simd, reference),
            "simd {} vs scalar {} on {} elements",
            simd,
            reference,
            a.len().min(b.len())
        );
    }

    /// Dot product: the same relative tolerance at the length production uses
    /// (768 fp16 components), where the number of accumulated terms is largest.
    #[test]
    fn dot_simd_matches_scalar_at_production_length(
        a in prop::collection::vec(half_component(), 768),
        b in prop::collection::vec(query_component(), 768),
    ) {
        let simd = kernels::dot(&a, &b);
        let reference = scalar::dot(&a, &b);
        prop_assert!(within_tolerance(simd, reference));
    }
}

/// The two kernels read the common prefix when the slices differ in length,
/// as `kernels` documents; they never read out of bounds and never panic.
#[test]
fn kernels_read_the_common_prefix() {
    let words = [0xaaaa_aaaa_aaaa_aaaa_u64; 5];
    assert_eq!(kernels::hamming(&words, &words[..2]), 0);
    assert_eq!(kernels::hamming(&words[..2], &words), 0);
    assert_eq!(kernels::hamming(&words, &[]), 0);

    let halves: Vec<f16> = (0..7).map(|i| f16::from_f32(i as f32)).collect();
    let query: Vec<f32> = (0..7).map(|i| i as f32).collect();
    assert_eq!(kernels::dot(&halves, &query), 91.0);
    assert_eq!(
        kernels::dot(&halves, &query[..3]),
        scalar::dot(&halves, &query[..3])
    );
    assert_eq!(kernels::dot(&[], &query), 0.0);
}

/// Both backends on a code of production size: 12 words, 768 bits.
#[test]
fn hamming_matches_scalar_on_a_production_code() {
    let a: Vec<u64> = (0..12_u64)
        .map(|i| i.wrapping_mul(0x0101_0101_0101_0101))
        .collect();
    let b: Vec<u64> = (0..12_u64)
        .map(|i| i.wrapping_mul(0xf0f0_f0f0_f0f0_f0f1))
        .collect();
    assert_eq!(kernels::hamming(&a, &b), scalar::hamming(&a, &b));
}

/// The selected backend is reported, so the benchmark output and the test log
/// say which implementation was measured.
#[test]
fn backend_is_reported() {
    let name = kernels::hamming_backend().as_str();
    assert!(["scalar", "neon", "avx2"].contains(&name));
    assert!(["scalar", "neon", "avx2"].contains(&kernels::dot_backend().as_str()));
}
