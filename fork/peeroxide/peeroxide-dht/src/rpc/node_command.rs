// JS: dht-rpc/index.js
//! `impl DhtNode::handle_command`: executes every `DhtCommand` sent by a `DhtHandle`.
//! Neighbours: `command` defines the messages; `handle*` sends them.

use tokio::sync::mpsc;

use crate::io::{Owner, RequestParams};
use crate::messages::Ipv4Peer;

use super::command::DhtCommand;
use super::consts::CMD_FIND_NODE;
use super::node::DhtNode;
use super::node_seeds::export;
use super::requests::StandaloneRequest;

impl DhtNode {
    pub(super) fn handle_command(&mut self, cmd: DhtCommand) -> bool {
        match cmd {
            DhtCommand::Bootstrapped { reply_tx } => {
                if self.bootstrapped {
                    let _ = reply_tx.send(Ok(()));
                } else {
                    self.bootstrap_waiters.push(reply_tx);
                }
            }

            DhtCommand::Ping {
                host,
                port,
                reply_tx,
            } => {
                let target = self.table.lock().ok().map(|t| *t.id());
                let params = RequestParams {
                    to: Ipv4Peer { host, port },
                    token: None,
                    internal: true,
                    command: CMD_FIND_NODE,
                    target,
                    value: None,
                    timeout_ms: None,
                    retries: None,
                    node_id: None,
                };
                if let Some(tid) = self.io.create_request(params) {
                    self.standalone_tids
                        .insert(tid, StandaloneRequest::Ping(reply_tx));
                } else {
                    let _ = reply_tx.send(Err(self.unavailable()));
                }
            }

            DhtCommand::FindNode {
                target,
                session,
                reply_tx,
            } => {
                self.start_query(
                    target,
                    true,
                    CMD_FIND_NODE,
                    None,
                    false,
                    None,
                    session,
                    None,
                    reply_tx,
                );
            }

            DhtCommand::Query {
                params,
                session,
                values,
                reply_tx,
            } => {
                let concurrency = params.concurrency.unwrap_or(self.config.concurrency);
                self.start_query(
                    params.target,
                    false,
                    params.command,
                    params.value,
                    params.commit,
                    Some(concurrency),
                    session,
                    values,
                    reply_tx,
                );
            }

            DhtCommand::Request {
                params,
                host,
                port,
                session,
                via,
                reply_tx,
            } => {
                let rparams = RequestParams {
                    to: Ipv4Peer { host, port },
                    token: params.token,
                    internal: false,
                    command: params.command,
                    target: params.target,
                    value: params.value,
                    timeout_ms: params.timeout_ms,
                    retries: params.retries,
                    node_id: None,
                };
                let owner = session.map(Owner::Session);
                let created = match via {
                    Some(via) => self.io.create_request_via(rparams, owner, via),
                    None => self.io.create_request_owned(rparams, owner),
                };
                if let Some(tid) = created {
                    self.standalone_tids
                        .insert(tid, StandaloneRequest::UserRequest(reply_tx));
                } else {
                    let _ = reply_tx.send(Err(self.unavailable()));
                }
            }

            DhtCommand::DestroySession { id } => self.destroy_session(id),

            DhtCommand::Relay {
                command,
                target,
                value,
                to,
                preserve_tid,
                via,
            } => {
                self.io
                    .relay(command, target, value, &to, preserve_tid, via.as_ref());
            }

            DhtCommand::SendReplyTo {
                tid,
                target,
                to,
                value,
            } => {
                self.io.reply_to(tid, target, &to, value);
            }

            DhtCommand::SubscribeRequests { reply_tx } => {
                let (tx, rx) = mpsc::unbounded_channel();
                self.request_subscribers.push(tx);
                let _ = reply_tx.send(rx);
            }

            DhtCommand::TableSize { reply_tx } => {
                let size = self.table.lock().map(|t| t.len()).unwrap_or(0);
                let _ = reply_tx.send(size);
            }

            DhtCommand::ToArray { limit, reply_tx } => {
                let list = match self.table.lock() {
                    Ok(t) => export(&t, limit),
                    Err(_) => vec![],
                };
                let _ = reply_tx.send(list);
            }

            DhtCommand::Suspend { reply_tx } => {
                self.suspend_node();
                let _ = reply_tx.send(());
            }

            DhtCommand::SetAdaptive { on, reply_tx } => {
                self.set_adaptive(on);
                let _ = reply_tx.send(());
            }

            DhtCommand::Resume { reply_tx } => {
                self.resume_node();
                let _ = reply_tx.send(());
            }

            DhtCommand::Destroy { reply_tx } => {
                self.destroyed = true;
                let _ = reply_tx.send(Ok(()));
                return true;
            }

            DhtCommand::LocalPort { reply_tx } => {
                let _ = reply_tx.send(Ok(self.local_port));
            }

            DhtCommand::TableId { reply_tx } => {
                let id = self.table.lock().ok().map(|t| *t.id());
                let _ = reply_tx.send(id);
            }

            DhtCommand::ServerSocket { reply_tx } => {
                let socket = Some(self.io.primary_socket());
                let _ = reply_tx.send(socket);
            }

            DhtCommand::ListenSocket { reply_tx } => {
                let socket = Some(self.io.server_socket());
                let _ = reply_tx.send(socket);
            }

            DhtCommand::RemoteAddress { reply_tx } => {
                let _ = reply_tx.send(self.node_nat.remote_state());
            }
            DhtCommand::InboundReplyBytes { addr, data } => {
                if let Some(event) = self.io.handle_inbound_reply_bytes(addr, data) {
                    self.handle_io_event(event);
                }
            }
            DhtCommand::InboundVia { addr, data, via } => {
                if let Some(event) = self.io.handle_inbound_via(addr, data, via) {
                    self.handle_io_event(event);
                }
            }
            DhtCommand::PingViaSocket {
                target,
                socket,
                reply_tx,
            } => {
                let id_target = self.table.lock().ok().map(|t| *t.id());
                let params = RequestParams {
                    to: target,
                    token: None,
                    internal: true,
                    command: CMD_FIND_NODE,
                    target: id_target,
                    value: None,
                    timeout_ms: None,
                    retries: None,
                    node_id: None,
                };
                if let Some(tid) = self.io.send_request_via_socket(params, &socket) {
                    self.standalone_tids
                        .insert(tid, StandaloneRequest::PingViaSocket(reply_tx));
                } else {
                    let _ = reply_tx.send(Err(self.unavailable()));
                }
            }
        }
        false
    }
}
