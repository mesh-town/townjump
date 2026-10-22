// JS: the DHT stream `close` event in holesail-server/index.js:handleTCP/handleUDP; related: connection.rs.
// Stream wrapper: calls `on_close` once when the peer finished sending (EOF, error) or the stream is dropped
// (a drop counts as a close only if the connection reports `close` on drop - not a JS fake stream).
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(crate) struct Watched<S> {
    inner: S,
    on_close: Option<Box<dyn FnOnce() + Send>>,
    close_on_drop: bool,
}

impl<S> Watched<S> {
    pub(crate) fn new(
        inner: S,
        close_on_drop: bool,
        on_close: impl FnOnce() + Send + 'static,
    ) -> Self {
        Watched {
            inner,
            on_close: Some(Box::new(on_close)),
            close_on_drop,
        }
    }

    fn closed(&mut self) {
        self.on_close.take().into_iter().for_each(|f| f());
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Watched<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        let ended = match &polled {
            Poll::Ready(Ok(())) => buf.filled().len() == before,
            Poll::Ready(Err(_)) => true,
            Poll::Pending => false,
        };
        if ended {
            self.closed();
        }
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Watched<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<S> Drop for Watched<S> {
    fn drop(&mut self) {
        if self.close_on_drop {
            self.closed();
        }
    }
}

#[cfg(test)]
#[path = "watched_tests.rs"]
mod tests;
