// JS: libudx/src/udx.c
//! First-packet firewall: plausibility of the first packet and the `set_firewall_hook` hook; related — `handle.rs`, `incoming.rs`.

use super::*;

/// Decide whether a UDX packet is plausible as the *first* arrival on an
/// unconnected stream awaiting its firewall-hook 4-tuple commit.
///
/// This is a defence-in-depth check; the socket-layer demux already enforces
/// header magic + version. Here we additionally reject flag combinations that
/// no legitimate fresh stream would lead with:
/// - `type_flags == 0`: a flagless packet carries no meaningful UDX intent
///   and looks like an unauthenticated probe.
/// - `FLAG_DESTROY`: a peer cannot legitimately tear down a stream that has
///   not yet been agreed (no prior packets, no negotiated state).
/// - `FLAG_SACK` in isolation: SACK ranges are meaningless before we have
///   sent any data to be selectively acknowledged.
///
/// Returns `false` to indicate "drop this packet, leave the hook armed for
/// the next inbound arrival". Returns `true` to permit hook firing and
/// 4-tuple commit.
#[inline]
pub(super) fn is_plausible_first_packet(header: &Header) -> bool {
    if header.type_flags == 0 {
        return false;
    }
    if header.has_flag(FLAG_DESTROY) {
        return false;
    }
    if header.type_flags == FLAG_SACK {
        return false;
    }
    true
}

impl UdxStream {
    /// Register a single-fire firewall hook for deferred 4-tuple commitment.
    ///
    /// The stream is registered with `socket` for demux immediately (so incoming
    /// packets addressed to `self.local_id` are routed here), but `connected`
    /// stays `false` until the hook fires.  On the first incoming packet the hook
    /// is called with `(socket, src_port, src_ip)`; if it returns `true` the
    /// stream adopts that source as its remote address and becomes connected,
    /// then processes the packet normally.  If it returns `false` the processor
    /// exits and the stream is permanently closed.
    ///
    /// `remote_id` is the remote stream's local ID (known from the noise handshake
    /// payload) and is required for correct ACK construction.
    ///
    // NOTE: this hook is single-fire (`FnOnce`) to match the Node
    // Hyperswarm reference. A re-armable variant (`Fn`, fired on every
    // unknown-source packet) would let the stream re-bind its 4-tuple
    // mid-stream — useful for NAT rebinding (when a NAT's external
    // mapping for the local port changes during a long-lived connection,
    // typically after an idle period or under aggressive NAT timers)
    // and for follow-the-mobile-peer scenarios (a peer that roams
    // between networks/interfaces without tearing the stream down).
    // We don't ship that variant today — the reference doesn't, and
    // rebinding mid-stream has subtle interactions with congestion
    // control, RTT estimation, and the secret-stream cipher state. If
    // we ever need it, add a sibling `set_firewall_hook_rearmable` that
    // takes an `Fn` and re-arms after each fire; do NOT change this
    // method's signature.
    pub fn set_firewall_hook<F>(
        &self,
        socket: &super::socket::UdxSocket,
        remote_id: u32,
        hook: F,
    ) -> Result<()>
    where
        F: FnOnce(&super::socket::UdxSocket, u16, std::net::IpAddr) -> bool + Send + 'static,
    {
        let udp = socket.udp_arc()?;
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();

        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.remote_id = remote_id;
            inner.udp = Some(udp);
            inner.notify_tx = Some(notify_tx);
            inner.firewall_hook = Some(Box::new(hook));
            inner.firewall_socket = Some(socket.clone());
        }

        self.attach(socket)?;

        self.spawn_processor(socket, notify_rx)?;

        tracing::debug!(
            local_id = self.local_id,
            remote_id,
            "stream listening (firewall hook set)"
        );
        Ok(())
    }
}
