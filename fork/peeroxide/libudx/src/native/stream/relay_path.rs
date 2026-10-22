//! Relay fast path: picks the paired stream and forwards a packet to it without ever holding two stream locks.
// JS: libudx/src/udx.c:relay_packet

use super::*;

/// Where a relayed packet goes: snapshot of the paired stream taken under its own lock.
pub(super) struct RelayDest {
    pub(super) remote_id: u32,
    pub(super) addr: Option<SocketAddr>,
    pub(super) udp: Option<Arc<tokio::net::UdpSocket>>,
}

/// Returns the destination of this stream's relay, if `relay_to` was configured.
///
/// The own lock is held only to clone the `Arc` of the target. The target lock is taken after
/// the own one is released: two bridged streams take these locks in opposite order, so holding
/// both at once deadlocks them (ABBA).
pub(super) fn pick_dest(inner: &Mutex<StreamInner>) -> Option<RelayDest> {
    let weak = inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .relay_target
        .clone()?;
    // Held weakly: bridged streams point at each other, strong links would form an `Arc` cycle.
    // A freed target looks like an unconnected one: `forward` drops the packet.
    let Some(target) = weak.upgrade() else {
        return Some(RelayDest {
            remote_id: 0,
            addr: None,
            udp: None,
        });
    };
    let tgt = target.lock().unwrap_or_else(|e| e.into_inner());
    tracing::trace!(dest_id = tgt.remote_id, dest_addr = ?tgt.remote_addr, "relay: target");
    Some(RelayDest {
        remote_id: tgt.remote_id,
        addr: tgt.remote_addr,
        udp: tgt.udp.clone(),
    })
}

/// Rewrites the destination id and sends the packet. `false` means it was dropped because the
/// target is not connected yet (the peer's retransmit recovers it, as in the C relay).
pub(super) async fn forward(dest: &RelayDest, fwd: &mut [u8]) -> bool {
    if fwd.len() >= 8 {
        fwd[4..8].copy_from_slice(&dest.remote_id.to_le_bytes());
    }
    let (Some(udp), Some(addr)) = (dest.udp.as_ref(), dest.addr) else {
        tracing::debug!(
            dest_id = dest.remote_id,
            addr_is_some = dest.addr.is_some(),
            udp_is_some = dest.udp.is_some(),
            "relay: dropped, target not connected"
        );
        return false;
    };
    let _sent = crate::native::tx_err::send_counted(udp, fwd, addr, "relay").await;
    tracing::trace!(len = fwd.len(), to = %addr, "relay: forwarded");
    true
}

#[cfg(test)]
#[path = "relay_path_tests.rs"]
mod tests;
