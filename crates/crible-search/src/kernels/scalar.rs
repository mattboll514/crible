// SPDX-License-Identifier: Apache-2.0

//! Portable reference implementation of the kernels.
//!
//! It is the definition of the two kernels: every SIMD implementation must
//! return the same result (exactly for [`hamming`], within `1e-5` for
//! [`dot`]), and it is also the fallback a CPU without the vector extensions
//! uses. The proptests, the benchmarks and the architecture-specific unit
//! tests call it as the oracle, which is why the module is public.
//!
//! Accumulation is in `f64`. The products are `f32`, as in the vectorised
//! versions, so the only difference with them is the summation order.

use half::f16;

/// Hamming distance, as the number of differing bits of the exclusive or.
///
/// Reads `min(a.len(), b.len())` words.
#[must_use]
pub fn hamming(a: &[u64], b: &[u64]) -> u64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| u64::from((x ^ y).count_ones()))
        .sum()
}

/// Dot product of a stored fp16 vector and a query f32 vector.
///
/// Reads `min(a.len(), b.len())` elements. Each product is computed in `f32`
/// (a widened `f16` times an `f32`) and accumulated in `f64`; the result is
/// rounded once, to `f32`, at the end.
#[must_use]
pub fn dot(a: &[f16], b: &[f32]) -> f32 {
    let mut acc = 0.0_f64;
    for (&x, &y) in a.iter().zip(b) {
        acc += f64::from(x.to_f32() * y);
    }
    acc as f32
}
