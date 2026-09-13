//! Synchronisation helpers.
//!
//! A poisoned `std::sync::Mutex` means another thread panicked while holding
//! the guard, so the protected state may be half-updated. Most call sites
//! cannot recover from that and should propagate the panic; [`lock`] does so
//! with one uniform message instead of a hand-written `expect` string at
//! every site. Sites that deliberately recover from poisoning (shutdown and
//! drop paths) keep calling `Mutex::lock` directly.

use std::sync::{Mutex, MutexGuard};

/// Lock `mutex`, panicking with the protected type and the caller's location
/// if a previous holder panicked.
#[track_caller]
pub fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => panic!(
            "mutex poisoned: Mutex<{}> (a previous holder panicked)",
            std::any::type_name::<T>()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn lock_returns_guard_for_healthy_mutex() {
        let mutex = Mutex::new(1u32);
        *lock(&mutex) += 1;
        assert_eq!(*lock(&mutex), 2);
    }

    #[test]
    #[ignore = "requires LLVM codegen backend for panic unwinding; see docs/TESTING.md"]
    fn lock_panics_with_type_name_when_poisoned() {
        let mutex = Arc::new(Mutex::new(Vec::<u8>::new()));
        let poisoner = Arc::clone(&mutex);
        std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the mutex");
        })
        .join()
        .expect_err("poisoning thread panics");
        assert!(mutex.is_poisoned());

        let payload = std::panic::catch_unwind(|| {
            drop(lock(&mutex));
        })
        .expect_err("lock panics on a poisoned mutex");
        let message = payload
            .downcast_ref::<String>()
            .expect("panic payload is a formatted String");
        assert!(message.contains("mutex poisoned"), "{message}");
        assert!(message.contains("alloc::vec::Vec<u8>"), "{message}");
    }
}
