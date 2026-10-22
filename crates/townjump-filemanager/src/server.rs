// JS: livefiles/index.js:constructor/_open/_close (listen, bind error handling).
use crate::conn::serve_conn;
use crate::opts::{Config, FilemanagerError, FilemanagerOpts, config};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::task::{AbortHandle, JoinHandle};
use townjump_jsval::{JsValue, to_port};

const MAX_REQUEST_SIZE: u64 = 10 * 1024 * 1024;
const STREAM_BUFFER_SIZE: u64 = 64 * 1024;

pub struct Filemanager {
    pub(crate) cfg: Arc<Config>,
    pub(crate) local_addr: Option<SocketAddr>,
    pub(crate) accept: Option<JoinHandle<()>>,
    pub(crate) conns: Arc<Mutex<Vec<AbortHandle>>>,
}

impl Drop for Filemanager {
    fn drop(&mut self) {
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
        if let Ok(mut list) = self.conns.lock() {
            for h in list.drain(..) {
                h.abort();
            }
        }
    }
}

pub struct FilemanagerInfo {
    pub kind: &'static str,
    pub host: JsValue,
    pub port: JsValue,
    pub role: String,
    pub username: JsValue,
    pub password: JsValue,
    pub max_request_size: u64,
    pub stream_buffer_size: u64,
}

impl Filemanager {
    pub fn new(opts: FilemanagerOpts) -> Result<Self, FilemanagerError> {
        Ok(Self {
            cfg: Arc::new(config(opts)?),
            local_addr: None,
            accept: None,
            conns: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub async fn ready(&mut self) -> Result<(), FilemanagerError> {
        if self.accept.is_some() {
            return Ok(());
        }
        let port = to_port(&self.cfg.port).map_err(FilemanagerError::Port)?;
        let host = self.cfg.host.to_js_string();
        match bind(&host, port).await {
            Ok(listener) => self.start(listener),
            Err(msg) => self.fatal(msg),
        }
        Ok(())
    }

    fn fatal(&self, msg: String) {
        if let Some(hook) = &self.cfg.on_fatal {
            hook(msg);
        }
    }

    fn start(&mut self, listener: TcpListener) {
        self.local_addr = listener.local_addr().ok();
        let cfg = self.cfg.clone();
        let conns = self.conns.clone();
        self.accept = Some(tokio::spawn(accept_loop(listener, cfg, conns)));
    }

    pub fn info(&self) -> FilemanagerInfo {
        FilemanagerInfo {
            kind: "filemanager",
            host: self.cfg.host.clone(),
            port: self.cfg.port.clone(),
            role: if self.cfg.admin { "admin" } else { "user" }.to_string(),
            username: self.cfg.username.clone(),
            password: self.cfg.password.clone(),
            max_request_size: MAX_REQUEST_SIZE,
            stream_buffer_size: STREAM_BUFFER_SIZE,
        }
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    pub async fn close(&mut self) {
        if let Some(accept) = self.accept.take() {
            accept.abort();
            let _ = accept.await;
        }
        let handles: Vec<AbortHandle> = std::mem::take(&mut *self.conns.lock().unwrap());
        for h in handles {
            h.abort();
        }
    }
}

async fn bind(host: &str, port: u16) -> Result<TcpListener, String> {
    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| format!("Error: getaddrinfo ENOTFOUND {host}"))?;
    let Some(addr) = addrs.next() else {
        return Err(format!("Error: getaddrinfo ENOTFOUND {host}"));
    };
    TcpListener::bind(addr)
        .await
        .map_err(|e| listen_error(&e, host, port))
}

async fn accept_loop(listener: TcpListener, cfg: Arc<Config>, conns: Arc<Mutex<Vec<AbortHandle>>>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            continue;
        };
        let handle = tokio::spawn(serve_conn(stream, cfg.clone()));
        let mut list = conns.lock().unwrap();
        list.retain(|h| !h.is_finished());
        list.push(handle.abort_handle());
    }
}

pub(crate) fn listen_error(err: &std::io::Error, host: &str, port: u16) -> String {
    use std::io::ErrorKind::*;
    match err.kind() {
        AddrInUse => format!("Error: listen EADDRINUSE: address already in use {host}:{port}"),
        PermissionDenied => format!("Error: listen EACCES: permission denied {host}:{port}"),
        AddrNotAvailable => {
            format!("Error: listen EADDRNOTAVAIL: address not available {host}:{port}")
        }
        _ => format!("Error: listen {err}"),
    }
}
