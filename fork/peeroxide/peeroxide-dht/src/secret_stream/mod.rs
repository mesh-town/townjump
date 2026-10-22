//! Encrypted transport: Noise XX handshake + secretstream over `AsyncRead + AsyncWrite`.
//!
//! Wire protocol (matching `@hyperswarm/secret-stream`):
//!
//! ```text
//! FRAMING: Every message is prefixed with a 3-byte little-endian uint24 length.
//!
//! NOISE XX HANDSHAKE:
//!   [3B len=32 ][32B Noise M1]                    initiator → responder
//!   [3B len=96 ][96B Noise M2]                    responder → initiator
//!   [3B len=64 ][64B Noise M3]                    initiator → responder
//!
//! ID HEADER EXCHANGE (both sides, immediately after handshake):
//!   [3B len=56 ][32B stream_id][24B secretstream_header]
//!
//! APPLICATION MESSAGES:
//!   [3B len=L+17][1B enc_tag][LB ciphertext][16B MAC]
//! ```
// JS: @hyperswarm/secret-stream/index.js

mod error;
mod frame;
mod framed;
mod io;
#[cfg(test)]
mod keepalive_tests;
#[cfg(test)]
mod lazy_header_tests;
mod session;
mod split;
#[cfg(test)]
mod test_support;
mod timers;

pub use error::SecretStreamError;
pub use session::SecretStream;
pub use split::{SecretStreamReadHalf, SecretStreamWriteHalf};
pub use timers::DEFAULT_KEEP_ALIVE;
