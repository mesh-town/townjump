// JS: encrypted duplex stream `c` from `dht.connect` and `server.on('connection')` (streamx Duplex).
// Related: types.rs (`DhtConnection.stream`); stream implementations live in the adapter.
use tokio::io::{AsyncRead, AsyncWrite};

/// Byte duplex over a DHT connection. `shutdown` = END, drop before the end = DESTROY, error ≠ EOF.
pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

#[cfg(test)]
#[path = "duplex_tests.rs"]
mod tests;
