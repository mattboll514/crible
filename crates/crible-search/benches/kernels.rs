// SPDX-License-Identifier: Apache-2.0

//! NOYAUX-SIMD micro-benchmark: the scalar reference against the dispatched
//! SIMD kernel, for the sizes that matter.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`NOYAUX-SIMD`),
//! `docs/07-stack-et-deploiement.md` ("Noyaux SIMD" — "à trancher (crate dédiée
//! ou intrinsèques NEON/AVX2 avec repli scalaire)", micro-benchmark in this
//! task), `docs/03-recherche.md` (branche dense: the scan and the rescoring).
//!
//! # What is measured
//!
//! - `hamming`: one call of the kernel over two binary codes. Sizes are the
//!   production one (768 bits, 12 words) and two neighbours (128 bits and
//!   4 096 bits) so that the fixed cost of the call is visible.
//! - `dot`: one call of the kernel over a stored fp16 vector and a query f32
//!   vector of 128, 768 and 4 096 components.
//!
//! `scalar` is `kernels::scalar`, the oracle; `simd` is `kernels::hamming` and
//! `kernels::dot`, which run the backend the runtime detection selected. The
//! backend name is printed before the run, because a "SIMD" number measured on
//! a scalar backend is meaningless.
//!
//! The benchmark is a fixed input built before the timed loop: the kernels are
//! branch-free over their input, so the data does not change the instruction
//! mix, only the bits.
//!
//! # Protocol
//!
//! `cargo bench -p crible-search --bench kernels` in `--release`. Criterion
//! default settings (3 s warm-up, 5 s measurement, 100 samples), inputs fully
//! in cache, single thread. The numbers recorded below come from one run on
//! the development machine; they justify the crate/code choice, they are not a
//! performance target (`docs/08-invariants-et-tests.md`: no MES for the
//! kernels).
//!
//! Machine: Apple Silicon (aarch64, NEON + fp16), `rustc 1.99.1`, macOS, one
//! bench at a time, no other load. The `simd` column includes the runtime
//! dispatch (a `OnceLock` read and an indirect call) because that is what
//! production pays; the `dispatch_only` case measures that fixed cost alone.
//!
//! | kernel | size | scalar | simd (dispatch included) | `wide` crate |
//! |---|---|---|---|---|
//! | hamming | 128 bits | 1.36 ns | 1.86 ns | — |
//! | hamming | 768 bits | 3.60 ns | 3.55 ns | — |
//! | hamming | 4 096 bits | 9.15 ns | 9.29 ns | — |
//! | dot | 128 f16 | 58.2 ns | 15.2 ns | 31.4 ns |
//! | dot | 768 f16 | 491.6 ns | 119.3 ns | 183.7 ns |
//! | dot | 4 096 f16 | 2 814 ns | 758 ns | 971 ns |
//! | dispatch only | — | — | 1.33 ns (hamming), 1.59 ns (dot) | — |
//!
//! # Why no crate
//!
//! The `wide` column comes from a throwaway crate outside the repository (same
//! machine, same data, `wide` 1.x): fp16 is widened by `half` before the
//! `f32x8` product, since `wide` has no fp16 type, and that scalar conversion
//! pass is what costs it the difference with the hand-written NEON kernel,
//! which converts with `vcvt_f32_f16` inside the vector loop.
//!
//! On `aarch64` the Hamming kernel is at parity with the reference, because
//! LLVM already vectorises the `count_ones` loop into `cnt` on a 128-bit
//! register: the instruction is the same, only the loop that feeds it differs.
//! Its body is 1.6x faster than the reference at 768 bits (3.55 ns against
//! 3.60 ns once the 1.33 ns of dispatch is removed on both sides); at
//! 4 096 bits the two are equal within noise. The vector path is kept for
//! `x86_64`, where a scalar `popcnt` handles 64 bits per cycle while AVX2
//! counts 256.
//!
//! Conclusion recorded for the ADR: no dedicated crate. `half` (already listed
//! in `docs/07-stack-et-deploiement.md`) and the `std::arch` intrinsics cover
//! both kernels, and the hand-written dot product is the fastest measured
//! option.

use std::hint::black_box;

use crible_search::kernels::{self, scalar};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use half::f16;

/// Sizes of the Hamming benchmark, in bits (production is 768).
const HAMMING_BITS: [usize; 3] = [128, 768, 4096];

/// Lengths of the dot-product benchmark, in components (production is 768).
const DOT_LENGTHS: [usize; 3] = [128, 768, 4096];

fn codes(bits: usize, seed: u64) -> (Vec<u64>, Vec<u64>) {
    let words = bits / 64;
    let a = (0..words)
        .map(|index| 0x0123_4567_89ab_cdef_u64.wrapping_mul(index as u64 + seed))
        .collect();
    let b = (0..words)
        .map(|index| 0xfedc_ba98_7654_3210_u64.wrapping_mul(index as u64 + 2 * seed))
        .collect();
    (a, b)
}

fn vectors(length: usize) -> (Vec<f16>, Vec<f32>) {
    let stored = (0..length)
        .map(|index| f16::from_f32((index % 13) as f32 * 0.25 - 1.5))
        .collect();
    let query = (0..length)
        .map(|index| (index % 7) as f32 * 0.5 - 1.0)
        .collect();
    (stored, query)
}

fn hamming(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("hamming");
    // Fixed cost of the dispatched call (OnceLock read, indirect call, tail
    // loop): measured on empty codes, so the kernel body does no work.
    group.bench_function("dispatch_only", |bencher| {
        bencher.iter(|| kernels::hamming(black_box(&[]), black_box(&[])));
    });
    for bits in HAMMING_BITS {
        let (a, b) = codes(bits, 3);
        group.throughput(Throughput::Elements((bits / 64) as u64));
        group.bench_with_input(BenchmarkId::new("scalar", bits), &bits, |bencher, _| {
            bencher.iter(|| scalar::hamming(black_box(&a), black_box(&b)));
        });
        group.bench_with_input(BenchmarkId::new("simd", bits), &bits, |bencher, _| {
            bencher.iter(|| kernels::hamming(black_box(&a), black_box(&b)));
        });
    }
    group.finish();
}

fn dot(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("dot");
    // Fixed cost of the dispatched call, measured on empty vectors.
    group.bench_function("dispatch_only", |bencher| {
        bencher.iter(|| kernels::dot(black_box(&[]), black_box(&[])));
    });
    for length in DOT_LENGTHS {
        let (stored, query) = vectors(length);
        group.throughput(Throughput::Elements(length as u64));
        group.bench_with_input(BenchmarkId::new("scalar", length), &length, |bencher, _| {
            bencher.iter(|| scalar::dot(black_box(&stored), black_box(&query)));
        });
        group.bench_with_input(BenchmarkId::new("simd", length), &length, |bencher, _| {
            bencher.iter(|| kernels::dot(black_box(&stored), black_box(&query)));
        });
    }
    group.finish();
}

fn kernels_benchmark(criterion: &mut Criterion) {
    eprintln!(
        "kernels backends: hamming={} dot={}",
        kernels::hamming_backend().as_str(),
        kernels::dot_backend().as_str()
    );
    hamming(criterion);
    dot(criterion);
}

criterion_group!(benches, kernels_benchmark);
criterion_main!(benches);
