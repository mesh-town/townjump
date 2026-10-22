// JS: dht-rpc/lib/query.js (Query as Readable), dht-rpc/index.js Session
//! `QueryStream`: a user query whose successful replies with a value are polled one by one as
//! they arrive (js `for await (const data of query)`), plus the final result. The query lives in
//! its own session: dropping the stream cancels it, `detach` lets it finish.
//! Neighbours: `command` carries the value listener, `node_query::start_query` wires it in.

use std::future::Future;
use std::task::{Context, Poll};

use tokio::sync::{mpsc, oneshot};

use crate::query::QueryReply;

use super::command::DhtCommand;
use super::handle::DhtHandle;
use super::session::Session;
use super::types::{DhtError, UserQueryParams};

type Outcome = Result<Vec<QueryReply>, DhtError>;
type Done = oneshot::Receiver<Outcome>;

pub(crate) struct QueryStream {
    values: mpsc::UnboundedReceiver<QueryReply>,
    done: Option<Done>,
    session: Session,
}

impl DhtHandle {
    /// Starts a query in a session of its own (also on a `Session` handle: the stream is dropped
    /// independently of the caller's session) and returns the stream of its value replies.
    pub(crate) fn query_stream(&self, params: UserQueryParams) -> QueryStream {
        let session = Session::new(self);
        let (values_tx, values) = mpsc::unbounded_channel();
        let (reply_tx, done) = oneshot::channel();
        // A closed node drops both senders with the command: the stream just ends.
        let _ = self.cmd_tx.send(DhtCommand::Query {
            params,
            session: session.dht().session_id,
            values: Some(values_tx),
            reply_tx,
        });
        QueryStream {
            values,
            done: Some(done),
            session,
        }
    }
}

impl QueryStream {
    /// Next reply with a value; `None` once the query ended (or the node is closed).
    pub(crate) fn poll_value(&mut self, cx: &mut Context<'_>) -> Poll<Option<QueryReply>> {
        self.values.poll_recv(cx)
    }

    /// The final result (closest replies). After the first `Ready` it is `Err(ChannelClosed)`.
    pub(crate) fn poll_done(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Vec<QueryReply>, DhtError>> {
        let Some(done) = self.done.as_mut() else {
            return Poll::Ready(Err(DhtError::ChannelClosed));
        };
        let finished = std::task::ready!(std::pin::Pin::new(done).poll(cx));
        self.done = None;
        Poll::Ready(finished.unwrap_or(Err(DhtError::ChannelClosed)))
    }

    /// Lets the query run to its end (late replies still reach the routing table) and drops the
    /// session after it. Without `detach`, dropping the stream cancels the query. Needs a runtime.
    /// `on_done` gets the result when it arrives (not called if the result was already taken).
    pub(crate) fn detach(self, on_done: impl FnOnce(Outcome) + Send + 'static) {
        let Self {
            values,
            done,
            session,
        } = self;
        drop(values);
        match done {
            Some(done) => {
                tokio::spawn(async move {
                    on_done(done.await.unwrap_or(Err(DhtError::ChannelClosed)));
                    drop(session);
                });
            }
            None => drop(session),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> (
        QueryStream,
        mpsc::UnboundedSender<QueryReply>,
        oneshot::Sender<Outcome>,
    ) {
        let (handle, _commands) = super::session_tests::channel_handle();
        let (values_tx, values) = mpsc::unbounded_channel();
        let (done_tx, done) = oneshot::channel();
        let stream = QueryStream {
            values,
            done: Some(done),
            session: Session::new(&handle),
        };
        (stream, values_tx, done_tx)
    }
}

#[cfg(test)]
#[path = "query_stream_tests.rs"]
mod tests;
