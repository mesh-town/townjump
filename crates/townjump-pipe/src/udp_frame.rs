// JS: @holesail/hyper-cmd-lib-net/index.js (frame `[u32 BE length][body]` in createUdpFramedProxy and pipeUdpFramedServer).
// Related: udp_proxy.rs and udp_server.rs encode and parse frames with this module.

/// Datagram frame: body length (4 bytes, big-endian) and body; an empty datagram has length 0.
pub fn encode_frame(datagram: &[u8]) -> Vec<u8> {
    let len = u32::try_from(datagram.len()).expect("datagram shorter than 4 GiB");
    [len.to_be_bytes().as_slice(), datagram].concat()
}

/// Accumulates stream bytes and yields complete frames.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    /// Bytes that have not yet formed a whole frame.
    pub buf: Vec<u8>,
}

impl FrameDecoder {
    /// Adds a stream chunk; returns the bodies of all frames that became complete, in order.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        std::iter::from_fn(|| self.next_frame()).collect()
    }

    fn next_frame(&mut self) -> Option<Vec<u8>> {
        let header: [u8; 4] = self.buf.get(..4)?.try_into().ok()?;
        let end = 4 + u32::from_be_bytes(header) as usize;
        let body = self.buf.get(4..end)?.to_vec();
        self.buf.drain(..end);
        Some(body)
    }
}

#[cfg(test)]
#[path = "udp_frame_tests.rs"]
mod tests;
