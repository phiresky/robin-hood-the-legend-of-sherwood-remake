//! Drive IndexedDB transaction bodies so their requests stay inside one
//! active transaction on threaded (`atomics`) wasm builds.
//!
//! IndexedDB commits a transaction as soon as control returns to the event
//! loop with no request pending. A body that awaits one request and then
//! issues the next therefore relies on being resumed from a *microtask* of the
//! request's success event. The single-threaded wasm-bindgen executor does
//! that. The `atomics` executor in js-sys does not: a woken task sleeps on an
//! `Atomics.waitAsync` promise, which resolves in a later macrotask, so the
//! transaction has already auto-committed and the next request (and the
//! subsequent `abort()`) fails. That broke browser replay persistence in the
//! threaded runtime regardless of cross-origin isolation.
//!
//! [`drive`] polls the body on the current thread from `queueMicrotask`
//! callbacks instead, and hands the result back through a oneshot channel. On
//! non-`atomics` builds it simply awaits the body.

use std::future::Future;

/// Run an IndexedDB transaction body so every wake re-polls it within the
/// current microtask checkpoint. The body runs to completion even if the
/// returned future is dropped: an open transaction must not be abandoned
/// half-way through a poll sequence.
pub async fn drive<T: 'static>(body: impl Future<Output = T> + 'static) -> T {
    #[cfg(not(target_feature = "atomics"))]
    {
        body.await
    }
    #[cfg(target_feature = "atomics")]
    {
        let (sender, receiver) = futures::channel::oneshot::channel();
        microtask::spawn(async move {
            // The receiver is only gone when the caller was cancelled; the
            // completed transaction's result is then intentionally unused.
            let _ = sender.send(body.await);
        });
        receiver
            .await
            .expect("IndexedDB microtask task was dropped before completing")
    }
}

#[cfg(target_feature = "atomics")]
mod microtask {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Wake, Waker};
    use std::thread::ThreadId;

    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        // Exposed on both Window and WorkerGlobalScope.
        #[wasm_bindgen(js_name = queueMicrotask)]
        fn queue_microtask(callback: &js_sys::Function);
    }

    type Body = Pin<Box<dyn Future<Output = ()>>>;

    thread_local! {
        static TASKS: RefCell<HashMap<u64, Rc<RefCell<Option<Body>>>>> = RefCell::new(HashMap::new());
        static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    }

    struct MicrotaskWaker {
        id: u64,
        owner: ThreadId,
        scheduled: AtomicBool,
    }

    impl Wake for MicrotaskWaker {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            // The body and its JS handles live in this thread's task table; a
            // wake from another thread could not reach them (or JS) at all.
            assert_eq!(
                std::thread::current().id(),
                self.owner,
                "IndexedDB microtask task woken from a different thread"
            );
            if self.scheduled.swap(true, Ordering::SeqCst) {
                return;
            }
            let waker = Arc::clone(self);
            let callback = Closure::once_into_js(move || poll(waker));
            queue_microtask(callback.unchecked_ref());
        }
    }

    pub(super) fn spawn(body: impl Future<Output = ()> + 'static) {
        let id = NEXT_ID.with(|next| {
            let id = next.get();
            next.set(id + 1);
            id
        });
        TASKS.with(|tasks| {
            tasks
                .borrow_mut()
                .insert(id, Rc::new(RefCell::new(Some(Box::pin(body)))))
        });
        Arc::new(MicrotaskWaker {
            id,
            owner: std::thread::current().id(),
            scheduled: AtomicBool::new(false),
        })
        .wake();
    }

    fn poll(waker: Arc<MicrotaskWaker>) {
        // Clear before polling so a wake during this poll schedules another.
        waker.scheduled.store(false, Ordering::SeqCst);
        let Some(task) = TASKS.with(|tasks| tasks.borrow().get(&waker.id).cloned()) else {
            // A stale wake after completion; nothing left to drive.
            return;
        };
        let mut slot = task.borrow_mut();
        let Some(body) = slot.as_mut() else {
            return;
        };
        let std_waker = Waker::from(Arc::clone(&waker));
        if body
            .as_mut()
            .poll(&mut Context::from_waker(&std_waker))
            .is_ready()
        {
            *slot = None;
            drop(slot);
            TASKS.with(|tasks| tasks.borrow_mut().remove(&waker.id));
        }
    }
}
