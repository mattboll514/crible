// SPDX-License-Identifier: Apache-2.0

//! NEON implementation of the kernels (`aarch64`).
//!
//! References: `docs/07-stack-et-deploiement.md` ("Noyaux SIMD"),
//! `docs/phases/P4-recherche-rapide.md` (`NOYAUX-SIMD`).
//!
//! The two entry points are safe functions that wrap one `unsafe` intrinsic
//! loop each. The dispatcher in the parent module only selects a wrapper after
//! `is_aarch64_feature_detected!` returned the features the wrapper enables,
//! which is what makes the calls sound; each wrapper repeats that argument in
//! its own `SAFETY` comment, because it is the code that is audited.

use core::arch::aarch64::*;

use half::f16;

/// Hamming distance over `u64` words, 256 bits per iteration.
///
/// `vcntq_u8` counts the set bits of every byte of the exclusive or; two
/// 128-bit halves are added per iteration (a byte of the sum is at most 16, so
/// it cannot overflow) and the pairwise widening add `vpadalq_u8` grows them
/// into `u16` lanes. Those lanes are flushed into the `u64` total every
/// [`FLUSH_ITERATIONS`] iterations, which bounds them at 32 768 and makes the
/// kernel exact for any length.
pub(crate) fn hamming(a: &[u64], b: &[u64]) -> u64 {
    // SAFETY: the dispatcher calls this wrapper only when
    // `is_aarch64_feature_detected!("neon")` is true, and `hamming_bits`
    // enables exactly `neon`.
    unsafe { hamming_bits(a, b) }
}

/// Dot product of an fp16 vector and an f32 vector, 8 elements per iteration.
///
/// The fp16 to fp32 conversion is one instruction (`vcvt_f32_f16`, `fp16`
/// extension); the products are `f32`, then widened to `f64` for the
/// accumulation, exactly as in the scalar reference.
pub(crate) fn dot(a: &[f16], b: &[f32]) -> f32 {
    // SAFETY: the dispatcher calls this wrapper only when both
    // `is_aarch64_feature_detected!("neon")` and
    // `is_aarch64_feature_detected!("fp16")` are true, and `dot_bits` enables
    // exactly those two features.
    unsafe { dot_bits(a, b) }
}

/// Number of four-word iterations between two flushes of the `u16`
/// accumulator into the `u64` total. Every lane stays at or below
/// `FLUSH_ITERATIONS * 32 = 32 768`, under the `u16` maximum of 65 535.
const FLUSH_ITERATIONS: usize = 1024;

/// # Safety
///
/// The caller must ensure the CPU supports NEON.
#[target_feature(enable = "neon")]
unsafe fn hamming_bits(a: &[u64], b: &[u64]) -> u64 {
    let len = a.len().min(b.len());
    let mut acc = vdupq_n_u16(0);
    let mut total: u64 = 0;
    let mut index = 0;
    let mut since_flush: usize = 0;
    // SAFETY: `index + 4 <= len`, so the four loads stay inside both slices
    // (`as_ptr().add(index)` is the first of four `u64`, and each
    // `vld1q_u64` reads two).
    unsafe {
        while index + 4 <= len {
            let first = veorq_u64(
                vld1q_u64(a.as_ptr().add(index)),
                vld1q_u64(b.as_ptr().add(index)),
            );
            let second = veorq_u64(
                vld1q_u64(a.as_ptr().add(index + 2)),
                vld1q_u64(b.as_ptr().add(index + 2)),
            );
            let bits = vaddq_u8(
                vcntq_u8(vreinterpretq_u8_u64(first)),
                vcntq_u8(vreinterpretq_u8_u64(second)),
            );
            acc = vpadalq_u8(acc, bits);
            index += 4;
            since_flush += 1;
            if since_flush == FLUSH_ITERATIONS {
                total += u64::from(vaddlvq_u16(acc));
                acc = vdupq_n_u16(0);
                since_flush = 0;
            }
        }
    }
    total += u64::from(vaddlvq_u16(acc));
    for offset in index..len {
        total += u64::from((a[offset] ^ b[offset]).count_ones());
    }
    total
}

/// # Safety
///
/// The caller must ensure the CPU supports NEON and the `fp16` extension.
#[target_feature(enable = "neon,fp16")]
unsafe fn dot_bits(a: &[f16], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut acc_low = vdupq_n_f64(0.0);
    let mut acc_high = vdupq_n_f64(0.0);
    let mut index = 0;
    // SAFETY: `index + 8 <= len` keeps the two 4-element loads and the two
    // 4-element loads of `b` inside their slices. `f16` is `repr(transparent)`
    // over `u16` (`half`), so the cast to `*const u16` is a reinterpretation of
    // the same 16 bytes.
    unsafe {
        while index + 8 <= len {
            let packed = vld1q_u16(a.as_ptr().add(index).cast::<u16>());
            let halves_low = vcvt_f32_f16(vreinterpret_f16_u16(vget_low_u16(packed)));
            let halves_high = vcvt_f32_f16(vreinterpret_f16_u16(vget_high_u16(packed)));
            let query_low = vld1q_f32(b.as_ptr().add(index));
            let query_high = vld1q_f32(b.as_ptr().add(index + 4));
            let product_low = vmulq_f32(halves_low, query_low);
            let product_high = vmulq_f32(halves_high, query_high);
            acc_low = vaddq_f64(acc_low, vcvt_f64_f32(vget_low_f32(product_low)));
            acc_high = vaddq_f64(acc_high, vcvt_f64_f32(vget_high_f32(product_low)));
            acc_low = vaddq_f64(acc_low, vcvt_f64_f32(vget_low_f32(product_high)));
            acc_high = vaddq_f64(acc_high, vcvt_f64_f32(vget_high_f32(product_high)));
            index += 8;
        }
    }
    let mut total = vaddvq_f64(vaddq_f64(acc_low, acc_high));
    for offset in index..len {
        total += f64::from(a[offset].to_f32() * b[offset]);
    }
    total as f32
}

#[cfg(test)]
mod tests {
    //! Each `unsafe` function above has a test that runs it directly, so the
    //! vector path is executed on `aarch64` even when the dispatcher would
    //! prefer another backend.

    use half::f16;

    use super::{dot_bits, hamming_bits};
    use crate::kernels::scalar;

    fn words(value: u64, count: usize) -> Vec<u64> {
        vec![value; count]
    }

    #[test]
    fn neon_hamming_matches_scalar() {
        let a = words(0x0123_4567_89ab_cdef, 12);
        let b = words(0xfedc_ba98_7654_3210, 12);
        // SAFETY: the test machine is `aarch64`, where NEON is part of the
        // baseline; this module is only compiled for `aarch64`.
        let distance = unsafe { hamming_bits(&a, &b) };
        assert_eq!(distance, scalar::hamming(&a, &b));
        assert_eq!(distance, 64 * 12);
    }

    #[test]
    fn neon_hamming_handles_a_partial_last_vector() {
        let a = vec![0_u64, 0, 0];
        let b = vec![u64::MAX, 1, 0];
        // SAFETY: NEON is part of the `aarch64` baseline.
        let distance = unsafe { hamming_bits(&a, &b) };
        assert_eq!(distance, 65);
        // SAFETY: NEON is part of the `aarch64` baseline.
        assert_eq!(unsafe { hamming_bits(&[], &[]) }, 0);
    }

    #[test]
    fn neon_dot_matches_scalar() {
        if !std::arch::is_aarch64_feature_detected!("fp16") {
            // A pre-ARMv8.2 core: the dispatcher keeps the scalar loop, and
            // `dot_bits` must not run there.
            return;
        }
        let a: Vec<f16> = (0..768)
            .map(|i| f16::from_f32((i % 13) as f32 * 0.25 - 1.5))
            .collect();
        let b: Vec<f32> = (0..768).map(|i| (i % 7) as f32 * 0.5 - 1.0).collect();
        // SAFETY: the `fp16` and NEON checks above and the `aarch64` build of
        // this module guarantee the features `dot_bits` enables.
        let simd = unsafe { dot_bits(&a, &b) };
        let reference = scalar::dot(&a, &b);
        assert!((simd - reference).abs() <= 1e-5, "{simd} vs {reference}");
        // SAFETY: NEON is part of the `aarch64` baseline.
        assert_eq!(unsafe { dot_bits(&[], &[]) }, 0.0);
    }
}
