// JS: dht-rpc/index.js
//! `impl DhtHandle`: `relay`, `relay_with_tid`, `relay_with_tid_via`, `send_reply_to`.
//! Neighbours: `handle` has the struct; `command` has the messages sent.

use libudx::UdxSocket;

use crate::messages::Ipv4Peer;
use crate::peer::NodeId;

use super::command::DhtCommand;
use super::handle::DhtHandle;
use super::types::DhtError;

impl DhtHandle {
    /// Fire-and-forget relay send (no response tracking).
    /// Relays an RPC command to `to` without waiting for a reply.
    pub fn relay(
        &self,
        command: u64,
        target: Option<NodeId>,
        value: Option<Vec<u8>>,
        to: &Ipv4Peer,
    ) -> Result<(), DhtError> {
        self.cmd_tx
            .send(DhtCommand::Relay {
                command,
                target,
                value,
                to: to.clone(),
                preserve_tid: None,
                via: None,
            })
            .map_err(|_| DhtError::ChannelClosed)
    }

    /// Fire-and-forget relay send that PRESERVES the inbound request's tid.
    ///
    /// Used by handshake-routing logic when forwarding a response back through
    /// a relay chain (mode FROM_SERVER). Mirrors Node `dht-rpc::Request.relay`:
    /// the tid is kept end-to-end so the original requester's inflight entry
    /// matches when the eventual REPLY arrives. Without this, the response is
    /// silently dropped at the relay hop.
    pub fn relay_with_tid(
        &self,
        command: u64,
        target: Option<NodeId>,
        value: Option<Vec<u8>>,
        to: &Ipv4Peer,
        tid: u16,
    ) -> Result<(), DhtError> {
        self.relay_with_tid_via(command, target, value, to, tid, None)
    }

    /// Like [`Self::relay_with_tid`], but sends from `via` when given (JS
    /// `req.relay(value, to, { socket })`: the server answers a relayed
    /// handshake from its hole-punch socket).
    pub fn relay_with_tid_via(
        &self,
        command: u64,
        target: Option<NodeId>,
        value: Option<Vec<u8>>,
        to: &Ipv4Peer,
        tid: u16,
        via: Option<UdxSocket>,
    ) -> Result<(), DhtError> {
        self.cmd_tx
            .send(DhtCommand::Relay {
                command,
                target,
                value,
                to: to.clone(),
                preserve_tid: Some(tid),
                via,
            })
            .map_err(|_| DhtError::ChannelClosed)
    }

    /// Fire-and-forget REPLY send to an arbitrary address using the supplied
    /// tid. Mirrors Node `dht-rpc::Request.reply(value, { to })`.
    ///
    /// Used by handshake routing when an FE-holder finalises the tid-preserved
    /// relay chain by sending the REPLY directly to the original client
    /// (FROM_SERVER → REPLY transition). The `tid` must be the inbound
    /// request's tid (= the original client's inflight tid, propagated end-to-end
    /// via [`Self::relay_with_tid`] at the previous hop).
    pub fn send_reply_to(
        &self,
        tid: u16,
        target: Option<NodeId>,
        to: &Ipv4Peer,
        value: Option<Vec<u8>>,
    ) -> Result<(), DhtError> {
        self.cmd_tx
            .send(DhtCommand::SendReplyTo {
                tid,
                target,
                to: to.clone(),
                value,
            })
            .map_err(|_| DhtError::ChannelClosed)
    }
}
