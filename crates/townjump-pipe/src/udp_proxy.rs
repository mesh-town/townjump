// JS: @holesail/hyper-cmd-lib-net/index.js:createUdpFramedProxy (one udp4 socket, a DHT connection per sender).
// Related: udp_frame.rs (frames), udp_server.rs (server side).
use crate::udp_client::{Sender, run_sender};
use crate::udp_recv::{RECV_BUF, recv_next};
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{UdpSocket, lookup_host};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio::task::JoinSet;
use townjump_logger::Logger;

/// Client's local UDP port; `task.abort()` closes the socket and the sender streams.
#[derive(Debug)]
pub struct UdpProxy {
    pub local_addr: SocketAddr,
    pub task: JoinHandle<()>,
}

/// Binds `listen`; the first datagram of a new sender (`address:port`) creates `connect()`, replies go to the same sender.
pub async fn create_udp_framed_proxy<S, C, Fut>(
    listen: (String, u16),
    connect: C,
    logger: Logger,
) -> io::Result<UdpProxy>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = io::Result<S>> + Send + 'static,
{
    let socket = Arc::new(UdpSocket::bind(resolve_v4(&listen).await?).await?);
    let local_addr = socket.local_addr()?;
    let task = tokio::spawn(receive_loop(socket, connect, logger));
    Ok(UdpProxy { local_addr, task })
}

/// `udp4` socket: IPv4 is taken from the host addresses (`localhost` does not go to `::1`).
async fn resolve_v4(listen: &(String, u16)) -> io::Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = lookup_host((listen.0.as_str(), listen.1)).await?.collect();
    let v4 = addrs.iter().find(|a| a.is_ipv4()).or(addrs.first());
    v4.copied()
        .ok_or_else(|| io::Error::from(io::ErrorKind::AddrNotAvailable))
}

/// Sender map → channel into its stream's task; the tasks live in a `JoinSet` and are dropped with the proxy.
async fn receive_loop<S, C, Fut>(socket: Arc<UdpSocket>, connect: C, logger: Logger)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = io::Result<S>> + Send + 'static,
{
    let (mut clients, mut tasks): (HashMap<SocketAddr, UnboundedSender<Vec<u8>>>, JoinSet<()>) =
        Default::default();
    let mut buf = vec![0u8; RECV_BUF];
    loop {
        while tasks.try_join_next().is_some() {}
        let (n, from) = match recv_next(&socket, &mut buf, &logger).await {
            Ok(got) => got,
            Err(e) => return logger.error(&format!("Proxy socket error: {e}")),
        };
        let id = from.to_string();
        logger.debug(&format!("UDP message from {id}: {n} bytes"));
        let Err(lost) = forward(&clients, from, buf[..n].to_vec()) else {
            continue;
        };
        let (tx, rx) = unbounded_channel();
        let _ = tx.send(lost);
        clients.insert(from, tx);
        let sender = Sender {
            id,
            addr: from,
            socket: socket.clone(),
            logger: logger.clone(),
        };
        tasks.spawn(run_sender(sender, connect(), rx));
    }
}

/// Hands a datagram to the sender's running task; no task (or it finished) — the datagram is returned.
fn forward(
    clients: &HashMap<SocketAddr, UnboundedSender<Vec<u8>>>,
    from: SocketAddr,
    datagram: Vec<u8>,
) -> Result<(), Vec<u8>> {
    let Some(tx) = clients.get(&from) else {
        return Err(datagram);
    };
    tx.send(datagram).map_err(|e| e.0)
}

#[cfg(test)]
#[path = "udp_proxy_tests.rs"]
mod tests;
