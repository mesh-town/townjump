//! `FramedStream` adapter: lets protomux run over a `SecretStream`.
// JS: @hyperswarm/secret-stream/index.js

use tokio::io::{AsyncRead, AsyncWrite};

use super::error::SecretStreamError;
use super::session::SecretStream;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> crate::protomux::FramedStream
    for SecretStream<T>
{
    async fn read_frame(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        self.read().await.map_err(|e| match e {
            SecretStreamError::Io(io_err) => io_err,
            other => std::io::Error::other(other.to_string()),
        })
    }

    async fn write_frame(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.write(data).await.map_err(|e| match e {
            SecretStreamError::Io(io_err) => io_err,
            other => std::io::Error::other(other.to_string()),
        })
    }
}
