// One way to take a `Mutex` in the crate: lock poisoning (a panic in a test) must not bring down neighbouring tests.
use std::sync::{Mutex, MutexGuard, PoisonError};

pub(crate) fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
