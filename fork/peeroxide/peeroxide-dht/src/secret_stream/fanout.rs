//! Composite waker: keepalive polls the transport from the reading task, and the transport keeps
//! only one waker per direction, so the poll must also wake the task that was waiting on the
//! write side (a flush awaiting its ACK).

use std::sync::Arc;
use std::task::{Wake, Waker};

pub(super) struct Fanout {
    first: Waker,
    second: Waker,
}

impl Fanout {
    pub(super) fn waker(first: &Waker, second: &Waker) -> Waker {
        Waker::from(Arc::new(Self {
            first: first.clone(),
            second: second.clone(),
        }))
    }
}

impl Wake for Fanout {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.first.wake_by_ref();
        self.second.wake_by_ref();
    }
}
