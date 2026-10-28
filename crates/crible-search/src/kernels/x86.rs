// SPDX-License-Identifier: Apache-2.0

//! AVX2 implementation of the kernels (`x86_64`).
//!
//! References: `docs/07-stack-et-deploiement.md` ("Noyaux SIMD"),
//! `docs/phases/P4-recherche-rapide.md` (`NOYAUX-SIMD`).
//!
//! The fp16 to fp32 conversion is an F16C instruction (`_mm256_cvtph_ps`), not
//! an AVX2 one. The dispatcher therefore requires both features for the dot
//! product and only AVX2 for Hamming, and each wrapper documents the exact set
//! it enables in its own `SAFETY` comment.

use core::arch::x86_64::*;

use half::f16;

/// Hamming distance over `u64` words, 256 bits per iteration.
///
/// AVX2 has no vector popcount, so the bits of every byte of the exclusive or
/// are counted through a 16-entry nibble lookup (`_mm256_shuffle_epi8`) and the
/// byte counts are summed by `_mm256_sad_epu8` into four `u64` lanes.
pub(crate) fn hamming(a: &[u64], b: &[u64]) -> u64 {
    // SAFETY: the dispatcher calls this wrapper only when
    // `is_x86_feature_detected!("avx2")` is true, and `hamming_bits` enables
    // exactly `avx2`.
    unsafe { hamming_bits(a, b) }
}

/// Dot product of an fp16 vector and an f32 vector, 8 elements per iteration.
///
/// `_mm256_cvtph_ps` widens eight fp16 in one instruction, the products are
/// `f32`, and each half of the product vector is widened to `f64` for the
/// accumulation, exactly as in the scalar reference.
pub(crate) fn dot(a: &[f16], b: &[f32]) -> f32 {
    // SAFETY: the dispatcher calls this wrapper only when
    // `is_x86_feature_detected!("avx2")` and `is_x86_feature_detected!("f16c")`
    // are both true, and `dot_bits` enables exactly those two features.
    unsafe { dot_bits(a, b) }
}

/// # Safety
///
/// The caller must ensure the CPU supports AVX2.
#[target_feature(enable = "avx2")]
unsafe fn hamming_bits(a: &[u64], b: &[u64]) -> u64 {
    let len = a.len().min(b.len());
    let low_mask = _mm256_set1_epi8(0x0f);
    // Set bits of every nibble, 0 to 4, duplicated over the two 128-bit lanes.
    let lookup = _mm256_setr_epi8(
        0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, 0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3,
        3, 4,
    );
    let mut acc = _mm256_setzero_si256();
    let mut index = 0;
    // SAFETY: `index + 4 <= len`, so the two 32-byte loads stay inside their
    // slices (`_mm256_loadu_si256` accepts any alignment), and the store
    // writes four `u64` into a four-element array.
    unsafe {
        while index + 4 <= len {
            let words_a = _mm256_loadu_si256(a.as_ptr().add(index).cast::<__m256i>());
            let words_b = _mm256_loadu_si256(b.as_ptr().add(index).cast::<__m256i>());
            let differing = _mm256_xor_si256(words_a, words_b);
            let low = _mm256_and_si256(differing, low_mask);
            let high = _mm256_and_si256(_mm256_srli_epi16(differing, 4), low_mask);
            let bits = _mm256_add_epi8(
                _mm256_shuffle_epi8(lookup, low),
                _mm256_shuffle_epi8(lookup, high),
            );
            // A byte of `bits` is at most 8, so the byte-wise sum cannot
            // overflow; `_mm256_sad_epu8` sums each 8-byte group into a `u64`.
            acc = _mm256_add_epi64(acc, _mm256_sad_epu8(bits, _mm256_setzero_si256()));
            index += 4;
        }
        let mut lanes = [0_u64; 4];
        _mm256_storeu_si256(lanes.as_mut_ptr().cast::<__m256i>(), acc);
        let mut total: u64 = lanes.iter().sum();
        for offset in index..len {
            total += u64::from((a[offset] ^ b[offset]).count_ones());
        }
        total
    }
}

/// # Safety
///
/// The caller must ensure the CPU supports AVX2 and F16C.
#[target_feature(enable = "avx2,f16c")]
unsafe fn dot_bits(a: &[f16], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut acc_low = _mm256_setzero_pd();
    let mut acc_high = _mm256_setzero_pd();
    let mut index = 0;
    // SAFETY: `index + 8 <= len` keeps the 16-byte load of `a`, the 32-byte
    // load of `b` and the four `f64` stores of the accumulator inside their
    // buffers. `f16` is `repr(transparent)` over `u16` (`half`), so the cast to
    // `*const __m128i` reinterprets the same 16 bytes.
    unsafe {
        while index + 8 <= len {
            let packed = _mm_loadu_si128(a.as_ptr().add(index).cast::<__m128i>());
            let halves = _mm256_cvtph_ps(packed);
            let query = _mm256_loadu_ps(b.as_ptr().add(index));
            let product = _mm256_mul_ps(halves, query);
            acc_low = _mm256_add_pd(acc_low, _mm256_cvtps_pd(_mm256_castps256_ps128(product)));
            acc_high = _mm256_add_pd(acc_high, _mm256_cvtps_pd(_mm256_extractf128_ps(product, 1)));
            index += 8;
        }
        let mut lanes = [0.0_f64; 4];
        _mm256_storeu_pd(lanes.as_mut_ptr(), _mm256_add_pd(acc_low, acc_high));
        let mut total: f64 = lanes.iter().sum();
        for offset in index..len {
            total += f64::from(a[offset].to_f32() * b[offset]);
        }
        total as f32
    }
}

#[cfg(test)]
mod tests {
    //! Each `unsafe` function above has a test that runs it directly, so the
    //! vector path is executed on `x86_64` even when the dispatcher would
    //! prefer another backend.

    use half::f16;

    use super::{dot_bits, hamming_bits};
    use crate::kernels::scalar;

    #[test]
    fn avx2_hamming_matches_scalar() {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return;
        }
        let a = vec![0x0123_4567_89ab_cdef_u64; 12];
        let b = vec![0xfedc_ba98_7654_3210_u64; 12];
        // SAFETY: the AVX2 check above guarantees the feature `hamming_bits`
        // enables.
        let distance = unsafe { hamming_bits(&a, &b) };
        assert_eq!(distance, scalar::hamming(&a, &b));
        assert_eq!(distance, 64 * 12);
        // SAFETY: the AVX2 check above guarantees the feature.
        assert_eq!(unsafe { hamming_bits(&[], &[]) }, 0);
    }

    #[test]
    fn avx2_dot_matches_scalar() {
        if !std::arch::is_x86_feature_detected!("avx2")
            || !std::arch::is_x86_feature_detected!("f16c")
        {
            return;
        }
        let a: Vec<f16> = (0..768)
            .map(|i| f16::from_f32((i % 13) as f32 * 0.25 - 1.5))
            .collect();
        let b: Vec<f32> = (0..768).map(|i| (i % 7) as f32 * 0.5 - 1.0).collect();
        // SAFETY: the AVX2 and F16C checks above guarantee the features
        // `dot_bits` enables.
        let simd = unsafe { dot_bits(&a, &b) };
        let reference = scalar::dot(&a, &b);
        assert!((simd - reference).abs() <= 1e-5, "{simd} vs {reference}");
        // SAFETY: the AVX2 and F16C checks above guarantee the features.
        assert_eq!(unsafe { dot_bits(&[], &[]) }, 0.0);
    }
}
