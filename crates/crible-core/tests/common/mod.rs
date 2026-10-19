// SPDX-License-Identifier: Apache-2.0

//! Helpers shared by the integration tests of `crible-core`.

use proptest::strategy::Strategy;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};

/// Number of cases run by every property test of this crate.
pub const PROPTEST_CASES: u32 = 256;

/// Runs a property test with a fixed seed and a fixed number of cases.
///
/// `proptest!` seeds the generator randomly unless `PROPTEST_RNG_SEED` is set,
/// so the runner is built by hand here: the same input sequence runs on every
/// invocation, and failure persistence is disabled.
pub fn check_property<V: std::fmt::Debug>(
    strategy: impl Strategy<Value = V>,
    property: impl Fn(V) -> Result<(), TestCaseError>,
) {
    let config = Config {
        cases: PROPTEST_CASES,
        source_file: None,
        ..Config::default()
    };
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    let mut runner = TestRunner::new_with_rng(config, rng);
    if let Err(failure) = runner.run(&strategy, property) {
        panic!("property failed: {failure}");
    }
}
