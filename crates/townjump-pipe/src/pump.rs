// JS: hyper-cmd-lib-net/index.js:connPiper (one pumping direction: `data` → `write`, `end` → `end`).
// Related: piper.rs (two such pumps and teardown), stats.rs (logger in the options).
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use townjump_logger::Logger;

/// Log texts of one direction: the chunk line and the source-end line.
#[derive(Clone, Copy)]
pub(crate) struct Direction {
    pub data: &'static str,
    pub end: &'static str,
}

/// From `loc` to `connection`.
pub(crate) const LOCAL_TO_REMOTE: Direction = Direction {
    data: "Data from local to remote",
    end: "Local end, ending connection",
};

/// From `connection` to `loc`.
pub(crate) const REMOTE_TO_LOCAL: Direction = Direction {
    data: "Data from remote to local",
    end: "Connection end, ending local",
};

pub(crate) fn debug(logger: &Option<Logger>, msg: &str) {
    logger.iter().for_each(|l| l.debug(msg));
}

pub(crate) fn info(logger: &Option<Logger>, msg: &str) {
    logger.iter().for_each(|l| l.info(msg));
}

/// Drives bytes `from` → `to` until `from` ends, then closes the write side of `to`; a read or write error is `Err`.
pub(crate) async fn pump<R, W>(
    mut from: R,
    mut to: W,
    dir: Direction,
    logger: &Option<Logger>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 16 * 1024];
    let mut unflushed = false;
    loop {
        let n = next_chunk(&mut from, &mut to, &mut buf, &mut unflushed).await?;
        if n == 0 {
            break;
        }
        debug(logger, &format!("{}: {n} bytes", dir.data));
        to.write_all(&buf[..n]).await?;
        unflushed = true;
    }
    debug(logger, dir.end);
    // The peer may have left already: ending a write to a closed stream is not a join error (in JS `end()` is silent).
    let _ = to.shutdown().await;
    Ok(())
}

/// Next chunk from the source. While there is none, flushes the write tail (`flush`): the transport may have kept a frame
/// to itself (SecretStream with a full UDX window) and without this it would leave only with the next write. `flush` does not delay reading:
/// in JS `connPiper` has no wait for an ACK per chunk, while the UDX stream's `flush` waits for an ACK of everything sent.
async fn next_chunk<R, W>(
    from: &mut R,
    to: &mut W,
    buf: &mut [u8],
    unflushed: &mut bool,
) -> io::Result<usize>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    while *unflushed {
        tokio::select! {
            biased;
            n = from.read(buf) => return n,
            flushed = to.flush() => {
                flushed?;
                *unflushed = false;
            }
        }
    }
    from.read(buf).await
}

#[cfg(test)]
#[path = "pump_tests.rs"]
mod tests;
