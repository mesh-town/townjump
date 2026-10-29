//! Probe loops of a [`Holepuncher`]: consistent-NAT probing, random-NAT
//! probing, birthday-socket opening and the random-NAT keep-alive.

use super::recv_adapter::spawn_recv_adapter_for_socket;
use super::{BIRTHDAY_SOCKETS, Holepuncher};
use crate::messages::Ipv4Peer;
use crate::socket_pool::{SocketPool, random_port};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::{Duration, sleep};

impl Holepuncher {
    pub(super) async fn consistent_probe(&mut self) {
        if !self.is_initiator {
            sleep(Duration::from_secs(1)).await;
        }

        for tries in 0..10 {
            if !self.punching {
                break;
            }

            for addr in &self.remote_addresses {
                if !addr.verified && (tries & 3) != 0 {
                    continue;
                }
                let target: SocketAddr = match format!("{}:{}", addr.host, addr.port).parse() {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                if let Some(socket) = self.sockets.first() {
                    let _ = socket.send_holepunch(target, false);
                }
            }

            if self.punching {
                sleep(Duration::from_secs(1)).await;
            }
        }

        self.auto_destroy();
    }

    pub(super) async fn random_probes(&mut self, remote_addr: &Ipv4Peer) {
        for _ in 0..1750 {
            if !self.punching {
                break;
            }

            let port = random_port();
            let target: SocketAddr = match format!("{}:{port}", remote_addr.host).parse() {
                Ok(a) => a,
                Err(_) => continue,
            };
            if let Some(socket) = self.sockets.first() {
                let _ = socket.send_holepunch(target, false);
            }

            if self.punching {
                sleep(Duration::from_millis(20)).await;
            }
        }

        self.auto_destroy();
    }

    pub(super) async fn open_birthday_sockets(
        &mut self,
        pool: &SocketPool,
        runtime: &libudx::UdxRuntime,
        remote_addr: &Ipv4Peer,
    ) {
        let target: SocketAddr = match format!("{}:{}", remote_addr.host, remote_addr.port).parse()
        {
            Ok(a) => a,
            Err(_) => return,
        };

        while self.punching && self.sockets.len() < BIRTHDAY_SOCKETS {
            match pool.acquire(runtime).await {
                Ok(mut socket) => {
                    let _ = socket.send_holepunch(target, true);
                    // Cover this birthday socket's probe stream with its own
                    // recv-adapter task so a probe landing on it (rather than
                    // the primary socket) still fires `Connected`.
                    if let Some(handle) = spawn_recv_adapter_for_socket(
                        &mut socket,
                        self.is_initiator,
                        self.event_tx.clone(),
                        Arc::clone(&self.connected_flag),
                    ) {
                        self.recv_tasks.push(handle);
                    }
                    self.sockets.push(socket);
                }
                Err(_) => break,
            }
        }
    }

    pub(super) async fn keep_alive_random_nat(&mut self, remote_addr: &Ipv4Peer) {
        let target: SocketAddr = match format!("{}:{}", remote_addr.host, remote_addr.port).parse()
        {
            Ok(a) => a,
            Err(_) => return,
        };

        sleep(Duration::from_millis(100)).await;

        let mut i = 0;
        let mut low_ttl_rounds: u32 = 1;

        for _ in 0..1750 {
            if !self.punching {
                break;
            }

            if i == self.sockets.len() {
                i = 0;
                low_ttl_rounds = low_ttl_rounds.saturating_sub(1);
            }

            if let Some(socket) = self.sockets.get(i) {
                let _ = socket.send_holepunch(target, low_ttl_rounds > 0);
            }
            i += 1;

            if self.punching {
                sleep(Duration::from_millis(20)).await;
            }
        }

        self.auto_destroy();
    }
}
