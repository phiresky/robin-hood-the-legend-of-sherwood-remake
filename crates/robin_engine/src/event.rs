//! Thread synchronization primitive (condvar + mutex).
//!
//! Wraps a `std::sync` condvar/mutex pair behind `wait` / `trigger`.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Timeout sentinel: wait forever.
const NO_TIMEOUT: i32 = -1;

/// Stateless event: a `trigger()` that arrives before any `wait()` is
/// lost, and every `wait()` blocks for the next broadcast (or timeout).
/// The mutex is the condvar's required companion and carries no predicate.
pub struct Event {
    mutex: Mutex<()>,
    condvar: Condvar,
}

impl Default for Event {
    fn default() -> Self {
        Self::new()
    }
}

impl Event {
    pub fn new() -> Self {
        Event {
            mutex: Mutex::new(()),
            condvar: Condvar::new(),
        }
    }

    /// Wait for the event to be triggered.
    ///
    /// * `timeout_ms < 0` (NO_TIMEOUT) — wait indefinitely.
    /// * `timeout_ms >= 0` — wait up to that many milliseconds.
    ///
    /// Returns `true` if woken by a broadcast (or a spurious wake), `false`
    /// on timeout.
    pub fn wait(&self, timeout_ms: i32) -> bool {
        let guard = self.mutex.lock().unwrap();
        self.wait_locked(guard, timeout_ms)
    }

    fn wait_locked(&self, guard: std::sync::MutexGuard<'_, ()>, timeout_ms: i32) -> bool {
        if timeout_ms != NO_TIMEOUT {
            let dur = Duration::from_millis(timeout_ms.max(0) as u64);
            let (_guard, result) = self.condvar.wait_timeout(guard, dur).unwrap();
            !result.timed_out()
        } else {
            let _guard = self.condvar.wait(guard).unwrap();
            true
        }
    }

    /// Signal all waiters. Always returns `true`; `Condvar::notify_all`
    /// cannot fail.
    pub fn trigger(&self) -> bool {
        self.condvar.notify_all();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn wait_with_timeout_returns_false_on_no_signal() {
        let ev = Event::new();
        assert!(!ev.wait(50));
    }

    #[test]
    fn trigger_wakes_waiting_thread() {
        assert_trigger_wakes_waiter(5000);
    }

    #[test]
    fn indefinite_wait_wakes_on_trigger() {
        assert_trigger_wakes_waiter(NO_TIMEOUT);
    }

    fn assert_trigger_wakes_waiter(timeout_ms: i32) {
        let ev = Arc::new(Event::new());
        let ev2 = ev.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let guard = ev2.mutex.lock().unwrap();
            ready_tx.send(()).unwrap();
            ev2.wait_locked(guard, timeout_ms)
        });

        ready_rx.recv().unwrap();
        // The worker releases this mutex atomically when entering condvar
        // wait. Holding it through notify prevents a scheduler-dependent
        // broadcast-before-wait race without guessing a sleep duration.
        let guard = ev.mutex.lock().unwrap();
        ev.trigger();
        drop(guard);
        assert!(handle.join().unwrap());
    }

    #[test]
    fn trigger_before_wait_is_lost() {
        // Stateless: a broadcast that arrives before a waiter is not
        // remembered, so a subsequent timed wait must time out.
        let ev = Event::new();
        assert!(ev.trigger());
        assert!(!ev.wait(50));
    }
}
