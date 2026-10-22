// Crate test helpers, part 2: observing a join (`Watch`), faulty streams, `connect` for the proxy.
// Included from support_tests.rs and available via `crate::support`.
use super::{Flag, LogCapture, PIPE_BUF, stays_pending, until_end, within};
use crate::stats::{OnDestroy, PipeStats, PiperOpts};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf, duplex};
use tokio::sync::mpsc;
use tokio::time::timeout;

/// An `on_destroy` call: `None` — no error, otherwise the error kind and text.
pub type Call = Option<(io::ErrorKind, String)>;

/// Records `on_destroy` calls: `None` — no error, otherwise the error kind and text.
#[derive(Clone, Default)]
pub struct Destroyed(Arc<Mutex<Vec<Call>>>);

impl Destroyed {
    pub fn hook(&self) -> OnDestroy {
        let calls = self.0.clone();
        Arc::new(move |err: Option<&io::Error>| {
            let call = err.map(|e| (e.kind(), e.to_string()));
            calls.lock().expect("calls").push(call);
        })
    }

    pub fn calls(&self) -> Vec<Call> {
        self.0.lock().expect("calls").clone()
    }
}

/// Everything a test observes of a join: counters, log, `on_destroy` calls, "join returned".
pub struct Watch {
    pub stats: Arc<PipeStats>,
    pub log: LogCapture,
    pub destroyed: Destroyed,
    pub ended: Flag,
    logging: bool,
}

impl Watch {
    pub fn new(logging: bool) -> Watch {
        Watch::sharing(Arc::default(), logging)
    }

    /// Several joins on the same counters: each has its own `Watch`, `stats` are shared.
    pub fn sharing(stats: Arc<PipeStats>, logging: bool) -> Watch {
        let (log, destroyed, ended) = Default::default();
        Watch {
            stats,
            log,
            destroyed,
            ended,
            logging,
        }
    }

    pub fn opts(&self) -> PiperOpts {
        PiperOpts {
            logger: self.logging.then(|| self.log.logger()),
            on_destroy: Some(self.destroyed.hook()),
        }
    }

    pub fn loc_rem_cnt(&self) -> (i64, i64) {
        let (loc, rem) = (&self.stats.loc_cnt, &self.stats.rem_cnt);
        (loc.load(Ordering::SeqCst), rem.load(Ordering::SeqCst))
    }
}

/// `conn_piper` over two `duplex`es: the far ends of `remote` and `local` go to the test.
pub fn duplex_piper(
    watch: &Watch,
) -> (DuplexStream, DuplexStream, impl Future<Output = ()> + use<>) {
    let (remote_far, remote_near) = duplex(PIPE_BUF);
    let (local_far, local_near) = duplex(PIPE_BUF);
    let piper =
        crate::piper::conn_piper(remote_near, local_near, watch.opts(), watch.stats.clone());
    (remote_far, local_far, until_end(watch.ended.clone(), piper))
}

/// `remote` is a `duplex` (its far end is returned), `local` is the given stream.
pub fn piper_with_local<B>(
    watch: &Watch,
    local: B,
) -> (DuplexStream, impl Future<Output = ()> + use<B>)
where
    B: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (remote_far, remote_near) = duplex(PIPE_BUF);
    let piper = crate::piper::conn_piper(remote_near, local, watch.opts(), watch.stats.clone());
    (remote_far, until_end(watch.ended.clone(), piper))
}

/// `local` is a `duplex` (its far end is returned), `remote` is the given stream.
pub fn piper_with_remote<A>(
    watch: &Watch,
    remote: A,
) -> (DuplexStream, impl Future<Output = ()> + use<A>)
where
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (local_far, local_near) = duplex(PIPE_BUF);
    let piper = crate::piper::conn_piper(remote, local_near, watch.opts(), watch.stats.clone());
    (local_far, until_end(watch.ended.clone(), piper))
}

/// A join over two streams whose read fails at once (no far ends).
pub fn failing_piper(
    watch: &Watch,
    remote_msg: &'static str,
    local_msg: &'static str,
) -> impl Future<Output = ()> + use<> {
    let (remote, local) = (
        FailingStream::read_fails(remote_msg),
        FailingStream::read_fails(local_msg),
    );
    let piper = crate::piper::conn_piper(remote, local, watch.opts(), watch.stats.clone());
    until_end(watch.ended.clone(), piper)
}

/// Faulty stream: read fails at once (`read_fails`), write fails (`write_fails`), `silent` stays silent.
pub struct FailingStream {
    read_error: Option<&'static str>,
    write_error: Option<&'static str>,
}

impl FailingStream {
    pub fn read_fails(msg: &'static str) -> Self {
        Self {
            read_error: Some(msg),
            write_error: None,
        }
    }

    pub fn write_fails(msg: &'static str) -> Self {
        Self {
            read_error: None,
            write_error: Some(msg),
        }
    }

    pub fn silent() -> Self {
        Self {
            read_error: None,
            write_error: None,
        }
    }
}

impl AsyncRead for FailingStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.read_error.take() {
            Some(msg) => Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionReset, msg))),
            None => Poll::Pending,
        }
    }
}

impl AsyncWrite for FailingStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.write_error {
            Some(msg) => Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, msg))),
            None => Poll::Ready(Ok(buf.len())),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

pub type ConnectResult = std::future::Ready<io::Result<DuplexStream>>;

/// "Remote side" for `connect`: each call hands the test the far end of a new `duplex`.
pub struct Remotes {
    count: Arc<AtomicUsize>,
    rx: mpsc::UnboundedReceiver<DuplexStream>,
}

impl Remotes {
    pub fn calls(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    pub async fn next(&mut self) -> DuplexStream {
        within("connect() should have been called", self.rx.recv())
            .await
            .expect("connect closed")
    }

    /// New stream, if one appears within `wait`.
    pub async fn next_within(&mut self, wait: Duration) -> Option<DuplexStream> {
        timeout(wait, self.rx.recv()).await.ok().flatten()
    }

    pub async fn none_new(&mut self) {
        // The channel closes together with `connect` (proxy stopped): that is not a call.
        let rx = &mut self.rx;
        let call = async {
            if rx.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        };
        stays_pending("connect() called for no reason", call).await;
    }
}

/// `connect` for the proxy: the first `failing_first` calls give `ConnectionRefused`, the rest give a `duplex`.
pub fn remotes(
    failing_first: usize,
) -> (Remotes, impl Fn() -> ConnectResult + Send + Sync + 'static) {
    let (tx, rx) = mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    let connect = move || {
        if counter.fetch_add(1, Ordering::SeqCst) < failing_first {
            let refused =
                io::Error::new(io::ErrorKind::ConnectionRefused, "connect refused (test)");
            return std::future::ready(Err(refused));
        }
        let (far, near) = duplex(1 << 20);
        tx.send(far).ok();
        std::future::ready(Ok(near))
    };
    (Remotes { count, rx }, connect)
}

/// `connect` whose remote side is an echo: what is written to the stream is read back.
pub fn echoing_remote() -> impl Fn() -> ConnectResult + Send + Sync + 'static {
    || {
        let (far, near) = duplex(1 << 20);
        tokio::spawn(async move {
            let (mut reader, mut writer) = tokio::io::split(far);
            tokio::io::copy(&mut reader, &mut writer).await.ok();
        });
        std::future::ready(Ok(near))
    }
}
