// SPDX-License-Identifier: Apache-2.0

//! In-memory journal sink shared by the unit tests of the crate.
//!
//! A test that reads a `tracing` line installs [`Sink`] through
//! [`crate::logging::log_subscriber`] and reads what was captured. It lives in
//! one module so the tests of `trim` and of `jobs` do not each carry their own
//! copy; the integration binaries keep the sink of `tests/common`, which serves
//! a whole test binary.
//!
//! Test only: the crate root declares the module under `#[cfg(test)]`, so a
//! production build has none of it.

use std::io;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

/// In-memory writer that captures the journal of a test.
#[derive(Clone, Default)]
pub(crate) struct Sink(Arc<Mutex<Vec<u8>>>);

impl Sink {
    /// Contents captured so far, lossily decoded.
    pub(crate) fn contents(&self) -> String {
        self.0
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }
}

/// One write handle of the [`Sink`].
pub(crate) struct SinkWriter(Sink);

impl io::Write for SinkWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let shared = &self.0.0;
        let mut bytes = shared
            .lock()
            .map_err(|_| io::Error::other("the journal sink is poisoned"))?;
        bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Sink {
    type Writer = SinkWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SinkWriter(self.clone())
    }
}
