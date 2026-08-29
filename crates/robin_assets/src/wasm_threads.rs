//! Browser worker pool for parallel VQ sprite decode (feature `wasm-threads`).
//!
//! Wraps [`wasm_bindgen_rayon`]: `init_pool` spawns the Web Worker pool that
//! backs rayon's global thread pool and records the thread count, which
//! [`crate::shipping_datadir`]'s materialization paths consult to choose
//! between the parallel and serial decode strategies. Initialization is only
//! possible on a cross-origin-isolated page (`SharedArrayBuffer` required);
//! callers must treat a failed/skipped init as "stay serial", never as fatal.

// `wasm-bindgen-rayon` itself carries the same guard, but fail early with a
// project-specific hint if the feature is enabled without an atomics build.
#[cfg(not(target_feature = "atomics"))]
compile_error!(
    "feature `wasm-threads` requires an atomics build: \
     use scripts/build-wasm-threads.sh (rustflags -C target-feature=+atomics,+bulk-memory,\
     +mutable-globals and -Zbuild-std=std,panic_abort)"
);

use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, anyhow};

static POOL_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Number of worker threads in the initialized rayon pool, or 0 when the
/// pool was never initialized (serial fallback).
pub fn pool_threads() -> usize {
    POOL_THREADS.load(Ordering::Acquire)
}

/// Run one closure on the rayon worker pool and resolve with its result,
/// without ever blocking the calling thread (the browser main thread must
/// not `atomics.wait`). The pool must be initialized; callers check
/// [`pool_threads`] and run inline otherwise.
pub async fn run_on_pool<T, F>(task: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = futures_channel::oneshot::channel();
    rayon::spawn(move || {
        // An unreceived result only means the caller was dropped.
        let _ = sender.send(task());
    });
    receiver
        .await
        .map_err(|_| anyhow!("wasm pool task dropped its result"))
}

/// Spawn `threads` Web Workers and install them as rayon's global pool.
///
/// Resolves once every worker has instantiated the module against the shared
/// memory. Must be called at most once per page load (rayon's global pool
/// cannot be rebuilt); a second call is rejected here before it can panic
/// inside rayon.
pub async fn init_pool(threads: usize) -> Result<()> {
    if threads == 0 {
        return Err(anyhow!("wasm thread pool needs at least one thread"));
    }
    if pool_threads() != 0 {
        return Err(anyhow!("wasm thread pool is already initialized"));
    }
    wasm_bindgen_futures::JsFuture::from(wasm_bindgen_rayon::init_thread_pool(threads))
        .await
        .map_err(|error| anyhow!("spawn wasm worker pool: {error:?}"))
        .context("init wasm rayon thread pool")?;
    POOL_THREADS.store(threads, Ordering::Release);
    Ok(())
}
