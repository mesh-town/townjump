// JS: libudx/src/udx.c
//! Incoming packet processor `process_incoming` (reliability engine); related — `acks.rs`, `receive.rs`, `wire.rs`.

use super::send_step::{
    AckKick, LoopTimers, ack_kick, drain_notifies, kick_by_timer, notify_step, send_step,
};
use super::*;

// ── Processor task (reliability engine) ──────────────────────────────────────

pub(super) async fn process_incoming(
    inner: Arc<Mutex<StreamInner>>,
    mut incoming_rx: mpsc::UnboundedReceiver<IncomingPacket>,
    mut notify_rx: mpsc::UnboundedReceiver<StreamNotify>,
) {
    let mut shutting_down = false;
    let mut timers = LoopTimers::new();

    loop {
        tokio::select! {
            // ── Incoming packet processing ───────────────────────────
            packet = incoming_rx.recv() => {
                let Some(packet) = packet else { break };
                let zt0 = diag::mode().z.then(Instant::now);
                if let (Some(stamp), Some(now)) = (packet.stamp, zt0) {
                    let delay = now.saturating_duration_since(stamp).as_micros() as u64;
                    let depth = incoming_rx.len();
                    inner.lock().unwrap_or_else(|e| e.into_inner()).zp.note_rx(delay, depth);
                }

                // ── Header decode (early — defense in depth) ─────────
                // The socket-layer demux (`UdxSocket::ensure_recv_loop`) only
                // routes packets whose header magic+version pass `Header::decode`,
                // so reaching this point implies a well-formed UDX header. We
                // re-decode here regardless: it is cheap (a 20-byte parse), it
                // makes the firewall-hook gate immediately below independent of
                // the socket-layer contract, and it lets the gate inspect the
                // first packet's flag bits before committing the 4-tuple.
                let header = match Header::decode(&packet.data) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                // ── Firewall hook: single-fire 4-tuple commit ────────────
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
                {
                    let is_connected = inner.lock().unwrap_or_else(|e| e.into_inner()).connected;
                    if !is_connected {
                        // Defense in depth: drop implausible first packets
                        // without firing the hook. Leaves the hook armed for
                        // the legitimate first arrival. UDX header magic+version
                        // are already enforced by the socket demux; here we
                        // additionally reject flag combinations that cannot
                        // legitimately begin a fresh stream.
                        if !is_plausible_first_packet(&header) {
                            tracing::debug!(
                                addr = ?packet.addr,
                                flags = header.type_flags,
                                "firewall hook: dropping implausible first packet (hook left armed)"
                            );
                            continue;
                        }
                        let hook_and_socket = {
                            let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                            guard.firewall_hook.take().zip(guard.firewall_socket.take())
                        };
                        match hook_and_socket {
                            Some((hook, sock)) => {
                                if hook(&sock, packet.addr.port(), packet.addr.ip()) {
                                    let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                                    guard.remote_addr = Some(packet.addr);
                                    guard.apply_link_mtu(super::link_mtu::link_mtu(packet.addr));
                                    guard.connected = true;
                                    // Patch any packets queued before the hook fired
                                    // (they were staged with the 0.0.0.0:0 sentinel).
                                    let sentinel: std::net::SocketAddr =
                                        "0.0.0.0:0".parse().unwrap();
                                    guard.send_queue.readdress(sentinel, packet.addr);
                                    if let Some(ref tx) = guard.notify_tx {
                                        let _ = tx.send(StreamNotify::DataQueued);
                                    }
                                    tracing::debug!(addr = ?packet.addr, "firewall hook: accepted");
                                } else {
                                    tracing::debug!(addr = ?packet.addr, "firewall hook: rejected");
                                    break;
                                }
                            }
                            None => continue,
                        }
                    }
                }

                if !path::path_gate(&inner, packet.addr) {
                    continue;
                }

                // ── Relay fast-path ──────────────────────────────
                //
                // Deliberately placed *after* the firewall-hook gate above,
                // not before it: a relay-bridged stream is typically wired
                // up with both `set_firewall_hook` (to learn its own real
                // 4-tuple from the first packet its owner sends) and
                // `relay_to` (to forward everything to its paired stream)
                // configured before any packets arrive. If this fast-path
                // ran first unconditionally, it would `continue` past the
                // firewall-hook gate on every packet forever — the hook
                // would never fire, `remote_addr`/`connected` would never
                // be set, and (since the *destination* stream in a pair has
                // the exact same problem simultaneously) neither side of a
                // relayed pairing could ever adopt its real address, so
                // nothing would ever forward (both
                // ends of a blind-relay pairing would retransmit until RTO
                // exhaustion with zero bytes ever bridged). Running the
                // hook gate first lets the triggering packet commit this
                // stream's 4-tuple, and *this same packet* still gets
                // relayed below rather than being absorbed into normal
                // (unread) stream processing.
                if let Some(dest) = relay_path::pick_dest(&inner) {
                    let mut fwd = packet.data;
                    relay_path::forward(&dest, &mut fwd).await;
                    if fwd.len() > 2 && fwd[2] & FLAG_DESTROY != 0 {
                        tracing::debug!("relay: DESTROY received, closing relay stream");
                        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(tx) = guard.close_tx.take() {
                            let _ = tx.send(());
                        }
                        break;
                    }
                    continue;
                }

                tracing::trace!(
                    flags = header.type_flags,
                    seq = header.seq,
                    ack = header.ack,
                    data_offset = header.data_offset,
                    "process_incoming: received packet"
                );

                // D3: reply ACK and exit before window, ack, SACK and data (JS: udx.c:process_packet)
                if header.has_flag(FLAG_HEARTBEAT) {
                    let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                    send_ack(&guard);
                    continue;
                }

                // ── ACK + SACK processing ────────────────────────────
                let sacks = incoming_sacks(&header, &packet.data);
                let outcome = inner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .on_ack_packet(header.ack, header.recv_window, &sacks);
                for tx in outcome.completed.into_iter().flatten() {
                    let _ = tx.send(Ok(()));
                }
                if let Some(w) = outcome.writer {
                    w.wake();
                }
                if let Some(d) = outcome.rto_timer {
                    timers.arm_rto(d);
                }
                if let Some(d) = outcome.zwp_timer {
                    timers.arm_zwp(d);
                }
                if let Some(d) = outcome.reo_timer {
                    timers.arm_reo(d);
                }
                if outcome.end_acked {
                    let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                    guard.ack_confirmed_end(header.type_flags); // D8: final state packet
                }

                // ── DATA / END processing (unified — handles combined DATA+END packets) ──
                let is_data = header.has_flag(FLAG_DATA);
                let is_end = header.has_flag(FLAG_END);
                if is_data || is_end {
                    let payload_start = header.payload_offset();
                    let payload = if payload_start <= packet.data.len() {
                        packet.data[payload_start..].to_vec()
                    } else {
                        Vec::new()
                    };
                    let recv_flags = (if is_data { FLAG_DATA } else { 0 })
                        | (if is_end { FLAG_END } else { 0 });

                    // D8: ACK for every DATA/END, and for a duplicate
                    let (events, read_tx) = {
                        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                        let events = guard.receive_data(header.seq, payload, recv_flags);
                        send_ack(&guard);
                        (events, guard.read_tx.clone())
                    };
                    for event in events {
                        let _ = read_tx.send(event);
                    }
                }

                path::note_direct(&mut inner.lock().unwrap_or_else(|e| e.into_inner()), &header, packet.addr);

                // ── DESTROY processing ───────────────────────────────
                if header.has_flag(FLAG_DESTROY) {
                    let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                    guard.connected = false;
                    let _ = guard.read_tx.send(StreamEvent::End);
                    if let Some(tx) = guard.close_tx.take() {
                        let _ = tx.send(());
                    }
                    break;
                }

                if let Some(t0) = zt0 {
                    let us = t0.elapsed().as_micros() as u64;
                    diag::ack_proc(us);
                    inner.lock().unwrap_or_else(|e| e.into_inner()).zp.note_ack_proc(us);
                }
                match ack_kick(outcome.kick_pacing, fix::mode().ackstep) {
                    AckKick::None => {}
                    AckKick::Timer => kick_by_timer(&mut timers),
                    AckKick::Inline => {
                        if diag::mode().z {
                            diag::step_inline();
                        }
                        if send_step(&inner, &mut timers, shutting_down).await.is_break() {
                            break;
                        }
                    }
                }
            }

            Some(notify) = notify_rx.recv() => {
                shutting_down |= drain_notifies(notify, &mut notify_rx);
                if notify_step(&inner, &mut timers, shutting_down).await.is_break() {
                    break;
                }
            }

            _ = &mut timers.pacing => {
                if diag::mode().z {
                    diag::step_src(timers.by_kick);
                    if let Some(at) = timers.kick_at.take().filter(|_| timers.by_kick) {
                        diag::kick_to_step(at.elapsed().as_micros() as u64);
                    }
                }
                if send_step(&inner, &mut timers, shutting_down).await.is_break() {
                    break;
                }
            }

            _ = &mut timers.rto => {
                let next = inner.lock().unwrap_or_else(|e| e.into_inner()).on_rto_fire();
                let Some(next) = next else {
                    tracing::warn!("stream failed: exceeded max RTO timeouts");
                    break;
                };
                timers.arm_rto(next);
                kick_by_timer(&mut timers);
            }

            _ = &mut timers.reo => {
                timers.arm_reo(FAR_FUTURE);
                let resend = inner.lock().unwrap_or_else(|e| e.into_inner()).on_reo_fire();
                if resend {
                    kick_by_timer(&mut timers);
                }
            }

            _ = &mut timers.zwp => {
                let next = inner.lock().unwrap_or_else(|e| e.into_inner()).on_zwp_fire();
                timers.arm_zwp(next);
            }

        }
    }

    inner.lock().unwrap_or_else(|e| e.into_inner()).terminate();
}

#[cfg(test)]
#[path = "incoming_tests.rs"]
mod tests;
