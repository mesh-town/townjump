// JS: @holesail/hyper-cmd-lib-net/index.js (connPiper, createTcpProxy, pipeTcpServer, UDP frames).
// Stream gluing; knows nothing about the DHT network: the remote side is any `AsyncRead + AsyncWrite`.
pub mod piper;
mod pump;
pub mod stats;
pub mod tcp_proxy;
pub mod tcp_server;
mod udp_client;
pub mod udp_frame;
pub mod udp_proxy;
mod udp_recv;
pub mod udp_server;

pub use piper::conn_piper;
pub use stats::{PipeStats, PiperOpts};
pub use tcp_proxy::{TcpProxy, create_tcp_proxy};
pub use tcp_server::pipe_tcp_server;
pub use udp_frame::{FrameDecoder, encode_frame};
pub use udp_proxy::{UdpProxy, create_udp_framed_proxy};
pub use udp_server::pipe_udp_framed_server;

// Shared test helpers for all modules of the crate.
#[cfg(test)]
#[path = "support_tests.rs"]
mod support;
