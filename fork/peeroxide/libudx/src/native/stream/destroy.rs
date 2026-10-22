// JS: libudx/src/udx.c
//! Stream destruction `UdxStream::destroy`; related — `handle.rs`, `end.rs`.

use super::*;

impl UdxStream {
    /// Destroy the stream, sending a DESTROY packet to the remote peer.
    pub async fn destroy(mut self) -> Result<()> {
        let destroy_info = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.connected {
                inner.udp.clone().zip(inner.remote_addr).map(|(udp, addr)| {
                    let header = Header {
                        type_flags: FLAG_DESTROY,
                        data_offset: 0,
                        remote_id: inner.remote_id,
                        recv_window: 0,
                        seq: inner.next_seq,
                        ack: inner.next_remote_seq,
                    };
                    (udp, addr, header)
                })
            } else {
                None
            }
        };

        if let Some((udp, addr, header)) = destroy_info {
            let _sent =
                crate::native::tx_err::send_counted(&udp, &header.encode(), addr, "destroy").await;
        }

        let processor_handle = self.processor.lock().ok().and_then(|mut g| g.take());
        if let Some(handle) = processor_handle {
            handle.abort();
            let _ = handle.await;
        }

        self.incoming_tx = None;

        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.terminate();
            if let Some(tx) = inner.close_tx.take() {
                let _ = tx.send(());
            }
        }

        if let Some(streams) = self
            .socket_streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            streams
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.local_id);
        }

        tracing::debug!(local_id = self.local_id, "stream destroyed");
        Ok(())
    }
}
