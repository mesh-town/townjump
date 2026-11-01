//! Wire format of the LAN discovery packets.
//!
//! Request: `"TJLD"(4) + version(1) + nonce(8) + target(32)` = 45 bytes.
//! Reply: the same fields followed by `port(2, big-endian)` = 47 bytes.
//! Trailing bytes are ignored; anything else malformed is dropped silently.

use std::net::Ipv4Addr;

/// Packet magic.
pub const MAGIC: &[u8; 4] = b"TJLD";
/// Format version.
pub const VERSION: u8 = 1;
/// Length of a request packet.
pub const REQUEST_LEN: usize = 45;
/// Length of a reply packet.
pub const REPLY_LEN: usize = 47;
/// Default multicast group.
pub const DEFAULT_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 84, 74);
/// Default UDP port of the group.
pub const DEFAULT_PORT: u16 = 28474;

/// Random value echoed by the reply.
pub type Nonce = [u8; 8];
/// Hash of the public key being looked for.
pub type Target = [u8; 32];

/// A decoded request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub nonce: Nonce,
    pub target: Target,
}

/// A decoded reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    pub nonce: Nonce,
    pub target: Target,
    pub port: u16,
}

fn header(nonce: &Nonce, target: &Target, cap: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(cap);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(nonce);
    out.extend_from_slice(target);
    out
}

fn parse(buf: &[u8], min: usize) -> Option<(Nonce, Target)> {
    if buf.len() < min || &buf[..4] != MAGIC || buf[4] != VERSION {
        return None;
    }
    let nonce = buf[5..13].try_into().ok()?;
    let target = buf[13..45].try_into().ok()?;
    Some((nonce, target))
}

/// Encodes a request.
pub fn encode_request(req: &Request) -> Vec<u8> {
    header(&req.nonce, &req.target, REQUEST_LEN)
}

/// Decodes a request; `None` for anything that is not a valid one.
pub fn decode_request(buf: &[u8]) -> Option<Request> {
    let (nonce, target) = parse(buf, REQUEST_LEN)?;
    Some(Request { nonce, target })
}

/// Encodes a reply.
pub fn encode_reply(reply: &Reply) -> Vec<u8> {
    let mut out = header(&reply.nonce, &reply.target, REPLY_LEN);
    out.extend_from_slice(&reply.port.to_be_bytes());
    out
}

/// Decodes a reply; `None` for anything that is not a valid one.
pub fn decode_reply(buf: &[u8]) -> Option<Reply> {
    let (nonce, target) = parse(buf, REPLY_LEN)?;
    let port = u16::from_be_bytes([buf[45], buf[46]]);
    Some(Reply {
        nonce,
        target,
        port,
    })
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;
