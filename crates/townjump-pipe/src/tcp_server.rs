// JS: @holesail/hyper-cmd-lib-net/index.js:pipeTcpServer (incoming DHT connection → new TCP to the target service).
// Related: piper.rs (join), tcp_proxy.rs (client side).
use crate::piper::pipe_connecting;
use crate::pump::info;
use crate::stats::{PipeStats, PiperOpts};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// Opens TCP to `local` (host as is) and joins with `remote`; a connection error tears down both sides.
pub async fn pipe_tcp_server<S>(
    remote: S,
    local: (String, u16),
    opts: PiperOpts,
    stats: Arc<PipeStats>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let logger = opts.logger.clone();
    let connect = async move {
        let stream = TcpStream::connect((local.0.as_str(), local.1)).await?;
        info(&logger, "Connected");
        Ok(stream)
    };
    pipe_connecting(remote, connect, opts, stats).await;
}

#[cfg(test)]
#[path = "tcp_server_tests.rs"]
mod tests;
