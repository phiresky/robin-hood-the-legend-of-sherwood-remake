//! Scoped, observational test captures. LLVM exercises the unwind contract;
//! Cranelift currently cannot resume through `catch_unwind` in the test runner.
use std::cell::RefCell;

/// A thread-local observation sink. Captures restore the previous scope even
/// during unwinding, so a panicking test cannot contaminate the next capture.
pub struct Probe<E>(RefCell<Option<Vec<E>>>);

impl<E> Probe<E> {
    pub const fn new() -> Self {
        Self(RefCell::new(None))
    }

    pub fn record(&self, event: E) {
        if let Some(events) = self.0.borrow_mut().as_mut() {
            events.push(event);
        }
    }

    pub fn capture<T>(&self, operation: impl FnOnce() -> T) -> (T, Vec<E>) {
        struct Restore<'a, E> {
            probe: &'a Probe<E>,
            previous: Option<Vec<E>>,
        }
        impl<E> Drop for Restore<'_, E> {
            fn drop(&mut self) {
                self.probe.0.replace(self.previous.take());
            }
        }
        let restore = Restore {
            probe: self,
            previous: self.0.replace(Some(Vec::new())),
        };
        let value = operation();
        let events = self.0.borrow_mut().take().expect("active capture");
        drop(restore);
        (value, events)
    }
}

#[test]
fn probe_scopes_are_nested_and_restore_after_panics() {
    let probe = Probe::new();
    probe.record(0);
    let (_, outer) = probe.capture(|| {
        probe.record(1);
        assert_eq!(probe.capture(|| probe.record(2)).1, [2]);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            probe.capture(|| {
                probe.record(3);
                panic!("fixture unwind");
            });
        }));
        assert!(panic.is_err());
        probe.record(4);
    });
    assert_eq!(outer, [1, 4]);
    assert_eq!(probe.capture(|| probe.record(5)).1, [5]);
}
