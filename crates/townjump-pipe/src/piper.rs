// JS: @holesail/hyper-cmd-lib-net/index.js:connPiper (pumping, half-close, teardown of both sides, log).
// Related: tcp_proxy.rs and tcp_server.rs call `pipe_connecting`; counters and options are in stats.rs; pumping is in pump.rs.
use crate::pump::{LOCAL_TO_REMOTE, REMOTE_TO_LOCAL, info, pump};
use crate::stats::{PipeStats, PiperOpts};
use std::future::{Future, pending, ready};
use std::io;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// How many bytes of `connection` accumulate while `dst` is not ready; once the buffer is full, reading pauses until `dst`.
const PRE_CONNECT_MAX: usize = 64 * 1024;

/// Joins `remote` and `local`; returns after teardown (any error or a close of either side).
pub async fn conn_piper<A, B>(remote: A, local: B, opts: PiperOpts, stats: Arc<PipeStats>)
where
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    B: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    pipe_connecting(remote, ready(Ok(local)), opts, stats).await;
}

/// `connPiper` with a `loc` that is not ready yet: `dst` is its connection (in JS `dst()` returns the stream at once, the data of
/// `connection` accumulates until it is ready). While `dst` is not ready, `connection` is read into a buffer (up to
/// `PRE_CONNECT_MAX`) and goes to `loc` first; a close of `connection` before `dst` tears the join down without an error
/// and drops `dst`. An error of `dst` tears the join down like an error of `loc`.
///
/// Deviation from JS: there the proxy is created with `allowHalfOpen: true` and on a half-close of `connection` it waits for `dst`,
/// sends what was accumulated and half-closes `loc`. We exit at once (the accumulated data is discarded, the log gets the
/// byte count) to stop DHT work after the client leaves. Cost: a client that half-closed its socket before `dst` was ready
/// (`printf … | nc -N`) gets no response.
pub(crate) async fn pipe_connecting<A, B, F>(
    remote: A,
    dst: F,
    opts: PiperOpts,
    stats: Arc<PipeStats>,
) where
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    B: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: Future<Output = io::Result<B>>,
{
    info(&opts.logger, "Starting TCP connection piper");
    stats.loc_cnt.fetch_add(1, SeqCst);
    stats.rem_cnt.fetch_add(1, SeqCst);
    let result = run(remote, dst, &opts).await;
    stats.loc_cnt.fetch_sub(1, SeqCst);
    stats.rem_cnt.fetch_sub(1, SeqCst);
    destroy(&opts, result.err());
}

async fn run<A, B, F>(remote: A, dst: F, opts: &PiperOpts) -> io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
    F: Future<Output = io::Result<B>>,
{
    let mut remote = remote;
    let mut early = Vec::new();
    let Some(mut local) = await_dst(&mut remote, dst, &mut early).await? else {
        if !early.is_empty() {
            let msg = format!(
                "connection closed before remote ready, {} bytes dropped",
                early.len()
            );
            info(&opts.logger, &msg);
        }
        return Ok(());
    };
    if !early.is_empty() {
        local.write_all(&early).await?;
    }
    let (remote_rd, remote_wr) = tokio::io::split(remote);
    let (local_rd, local_wr) = tokio::io::split(local);
    let log = &opts.logger;
    tokio::try_join!(
        pump(local_rd, remote_wr, LOCAL_TO_REMOTE, log),
        pump(remote_rd, local_wr, REMOTE_TO_LOCAL, log),
    )
    .map(|_| ())
}

/// Waits for `dst` while reading `remote` into `early`. `None` — `remote` closed first (`dst` dropped);
/// a read error is `Err`. `read` is cancelled safely: what was read is not lost.
async fn await_dst<A, B, F>(remote: &mut A, dst: F, early: &mut Vec<u8>) -> io::Result<Option<B>>
where
    A: AsyncRead + Unpin,
    F: Future<Output = io::Result<B>>,
{
    let mut dst = pin!(dst);
    let mut chunk = [0u8; 8192];
    loop {
        let room = (PRE_CONNECT_MAX - early.len()).min(chunk.len());
        let read = async {
            if room == 0 {
                pending().await
            } else {
                remote.read(&mut chunk[..room]).await
            }
        };
        tokio::select! {
            biased;
            ready = &mut dst => return ready.map(Some),
            n = read => match n? {
                0 => return Ok(None),
                n => early.extend_from_slice(&chunk[..n]),
            },
        }
    }
}

fn destroy(opts: &PiperOpts, err: Option<io::Error>) {
    let reason = err.as_ref().map(|e| format!(" due to error: {e}"));
    info(
        &opts.logger,
        &format!("Destroying connection piper{}", reason.unwrap_or_default()),
    );
    opts.on_destroy.iter().for_each(|f| f(err.as_ref()));
}

#[cfg(test)]
#[path = "piper_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "piper_connecting_tests.rs"]
mod connecting_tests;
