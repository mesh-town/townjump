// JS: interop/js/testnet.mjs and run-matrix.mjs (collecting process stdout/stderr into a buffer), MOCK_RANDOM_HEX from mock-dht.cjs.
// Related: quiet.rs reads `last_write` to wait for silence.
use crate::lock::lock;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use townjump_logger::SharedOut;

#[derive(Default)]
struct Buf {
    bytes: Vec<u8>,
    last_write: Option<Instant>,
}

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(data);
        self.last_write = Some(Instant::now());
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Output buffer instead of stdout/stderr: clones share the same bytes.
#[derive(Clone, Default)]
pub struct Capture(Arc<Mutex<Buf>>);

impl Write for Capture {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        lock(&self.0).write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    /// The same buffer as a shared output stream for `Logger` and the CLI.
    pub fn shared(&self) -> SharedOut {
        self.0.clone()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }

    pub fn bytes(&self) -> Vec<u8> {
        lock(&self.0).bytes.clone()
    }

    /// Moment of the last write; `None` means nothing was written yet.
    pub fn last_write(&self) -> Option<Instant> {
        lock(&self.0).last_write
    }
}

/// "Randomness" source like the mock's `MOCK_RANDOM_HEX`: each call writes `0, 1, …, 31, 0, …`.
pub fn fixed_random() -> impl FnMut(&mut [u8]) + Send + 'static {
    |buf| {
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = (i % 32) as u8;
        }
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
