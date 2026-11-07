#![deny(clippy::all)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use libudx::{UdxRuntime, set_bind_hook};

static CALLS: AtomicUsize = AtomicUsize::new(0);
static LAST: std::sync::Mutex<Option<SocketAddr>> = std::sync::Mutex::new(None);

fn hook(addr: SocketAddr) -> std::io::Result<std::net::UdpSocket> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    *LAST.lock().expect("lock") = Some(addr);
    std::net::UdpSocket::bind(addr)
}

#[tokio::test]
async fn bind_hook_used_when_set() {
    assert!(set_bind_hook(hook));
    let rt = UdxRuntime::new().expect("runtime");
    let socket = rt.create_socket().await.expect("create socket");
    let addr: SocketAddr = "127.0.0.1:0".parse().expect("parse");
    socket.bind(addr).await.expect("bind");

    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(*LAST.lock().expect("lock"), Some(addr));
    let local = socket.local_addr().await.expect("local addr");
    assert_eq!(local.ip(), addr.ip());
    assert_ne!(local.port(), 0);
}
