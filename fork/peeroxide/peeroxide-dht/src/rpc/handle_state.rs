// JS: dht-rpc/index.js
//! `impl DhtHandle`: `subscribe_requests`, `destroy` and the table/socket/address getters.
//! Neighbours: `handle` has the struct; `command` has the messages sent.

use tokio::sync::{mpsc, oneshot};

use libudx::UdxSocket;

use crate::messages::Ipv4Peer;
use crate::peer::NodeId;

use super::command::DhtCommand;
use super::handle::DhtHandle;
use super::requests::UserRequest;
use super::types::DhtError;

impl DhtHandle {
    /// Subscribes to forwarded user requests.
    pub async fn subscribe_requests(&self) -> Option<mpsc::UnboundedReceiver<UserRequest>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::SubscribeRequests { reply_tx: tx })
            .ok()?;
        rx.await.ok()
    }

    /// Returns the current routing table size.
    pub async fn table_size(&self) -> Result<usize, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::TableSize { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)
    }

    /// Routing table entries as host/port pairs, most recently seen first; `None` means all.
    pub async fn to_array(&self, limit: Option<usize>) -> Result<Vec<Ipv4Peer>, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::ToArray {
                limit,
                reply_tx: tx,
            })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)
    }

    /// Destroys the running DHT node.
    pub async fn destroy(&self) -> Result<(), DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::Destroy { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }

    /// Returns the local bound port.
    pub async fn local_port(&self) -> Result<u16, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::LocalPort { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }

    /// Returns the node's current routing table ID, or None if not yet assigned.
    pub async fn table_id(&self) -> Result<Option<NodeId>, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::TableId { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)
    }

    /// Returns a shared reference to the DHT server socket for UDX stream multiplexing.
    pub async fn server_socket(&self) -> Result<Option<UdxSocket>, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::ServerSocket { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)
    }

    /// Returns the actual listen socket (the socket bound to the advertised port).
    pub async fn listen_socket(&self) -> Result<Option<UdxSocket>, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::ListenSocket { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)
    }

    /// Returns the externally-observed IPv4 address of this node, as inferred
    /// from the `to` field of the last 16 DHT responses (dht-rpc `NatSampler`
    /// window; one sample per responder per minute).
    ///
    /// `None` until 3 samples, while the window disagrees (e.g. right after a
    /// network change), and always when running unfirewalled. May go back from
    /// `Some` to `None` and to a new address.
    pub async fn remote_address(&self) -> Result<Option<Ipv4Peer>, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::RemoteAddress { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        let (addr, _firewall) = rx.await.map_err(|_| DhtError::ChannelClosed)?;
        Ok(addr)
    }

    /// Returns the current firewall classification (one of `FIREWALL_OPEN`,
    /// `FIREWALL_UNKNOWN`, `FIREWALL_CONSISTENT`, `FIREWALL_RANDOM` constants
    /// from [`crate::hyperdht_messages`]).
    ///
    /// Follows the same window as [`Self::remote_address`].
    pub async fn firewalled(&self) -> Result<u64, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::RemoteAddress { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        let (_addr, firewall) = rx.await.map_err(|_| DhtError::ChannelClosed)?;
        Ok(firewall)
    }
}
