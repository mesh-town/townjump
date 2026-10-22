//! `AsyncRead`/`AsyncWrite` adapter over a UDX stream: read, write without waiting for ACK, close.
// JS: udx-native/lib/stream.js

mod adapter;
mod close;
mod read;
mod write;

pub use adapter::UdxAsyncStream;
