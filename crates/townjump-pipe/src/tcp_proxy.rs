// JS: @holesail/hyper-cmd-lib-net/index.js:createTcpProxy (client's local TCP server, `allowHalfOpen`).
// Related: piper.rs (a join per accepted connection), tcp_server.rs (server side).
use crate::piper::pipe_connecting;
use crate::stats::{PipeStats, PiperOpts};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// Listening local port of the client; `task.abort()` closes accepting (already accepted connections keep running).
#[derive(Debug)]
pub struct TcpProxy {
    pub local_addr: SocketAddr,
    pub task: JoinHandle<()>,
}

/// Listens on `listen`; for each accepted connection calls `connect()` at once and joins. `Ok` — after bind.
pub async fn create_tcp_proxy<S, C, Fut>(
    listen: (String, u16),
    connect: C,
    opts: PiperOpts,
    stats: Arc<PipeStats>,
) -> io::Result<TcpProxy>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = io::Result<S>> + Send + 'static,
{
    let listener = TcpListener::bind((listen.0.as_str(), listen.1)).await?;
    let local_addr = listener.local_addr()?;
    let task = tokio::spawn(accept_loop(listener, connect, opts, stats));
    Ok(TcpProxy { local_addr, task })
}

/// Accepted connections run in their own tasks: closing accept (`abort`) does not touch them.
async fn accept_loop<S, C, Fut>(
    listener: TcpListener,
    connect: C,
    opts: PiperOpts,
    stats: Arc<PipeStats>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = io::Result<S>> + Send + 'static,
{
    while let Ok((accepted, _)) = listener.accept().await {
        tokio::spawn(pipe_accepted(
            accepted,
            connect(),
            opts.clone(),
            stats.clone(),
        ));
    }
}

async fn pipe_accepted<S, Fut>(
    accepted: TcpStream,
    dst: Fut,
    opts: PiperOpts,
    stats: Arc<PipeStats>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Fut: Future<Output = io::Result<S>>,
{
    pipe_connecting(accepted, dst, opts, stats).await;
}

#[cfg(test)]
#[path = "tcp_proxy_tests.rs"]
mod tests;
