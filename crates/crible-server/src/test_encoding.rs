// SPDX-License-Identifier: Apache-2.0

//! Test double of the query encoder of the `failpoints` build.
//!
//! ADR-0110, decision 11 asks for a test that proves the encoder of the server
//! is called **once** for a request with several targets, in the Rust job of the
//! CI, without ONNX Runtime and without a new `#[ignore]`. The double lives at
//! the single encoding point of the server, [`crate::search::encode_query`]: it
//! replaces the ONNX run with a fixed vector and counts the **real calls**, not
//! the reads of the query cache (ADR-0051), which never reach this point.
//!
//! Protocol, entirely driven by one environment variable:
//!
//! - `CRIBLE_TEST_ENCODER` names a directory the test owns;
//! - each call appends one byte to `<dir>/encoded`, so the length of that file
//!   is the number of calls the server made;
//! - without the variable the function returns `None`, so a build with the
//!   feature but no test armed behaves exactly like a normal one.
//!
//! The double exists **only** with the `failpoints` feature, never in a normal
//! build and never in a release build, like the barriers of
//! [`crate::barriers`].

use std::fs::OpenOptions;
use std::io::Write;

use crible_core::error::CribleError;

use crate::error;

/// Environment variable naming the directory of the double.
pub(crate) const ENCODER_ENV: &str = "CRIBLE_TEST_ENCODER";

/// Name of the file whose length counts the real calls to the encoder.
pub(crate) const COUNT_FILE: &str = "encoded";

/// Encodes through the double, or returns `None` when no test armed it.
///
/// The returned vector is the first axis of the model space, so a fixture whose
/// passages lie on that axis ranks them in a known order. `dims` is the
/// dimensionality of the model the request resolved.
pub(crate) fn encode(dims: u32) -> Option<Result<(Vec<f32>, bool), CribleError>> {
    let directory = std::env::var(ENCODER_ENV).ok()?;
    let path = std::path::Path::new(&directory).join(COUNT_FILE);
    let counted = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(b"x"));
    if let Err(cause) = counted {
        // The test armed the double but its directory is not writable: counting
        // silently nothing would hide the fault the test wants to observe.
        return Some(Err(error::internal(format!(
            "the test encoder cannot count a call in {}: {cause}",
            path.display()
        ))));
    }
    let length = usize::try_from(dims).unwrap_or(0);
    let mut vector = vec![0.0; length];
    if let Some(first) = vector.first_mut() {
        *first = 1.0;
    }
    Some(Ok((vector, false)))
}
