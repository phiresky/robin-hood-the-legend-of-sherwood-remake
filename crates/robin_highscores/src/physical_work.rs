//! Completion ownership for the worker's blocking filesystem/process jobs.
//!
//! A Tokio blocking job survives dropping its join handle. Register its lifetime
//! before enqueueing it, not when the closure starts, so even queued jobs are
//! included in the worker's lease/fence drain.

use futures_util::FutureExt as _;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

// Runtime ownership is deliberately not serializable.
#[derive(Default)]
struct PendingWork {
    active: AtomicUsize,
    completed: Notify,
}

struct Completion(Arc<PendingWork>);

impl Drop for Completion {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        self.0.completed.notify_one();
    }
}

tokio::task_local! {
    static PENDING: Arc<PendingWork>;
}

/// Track a physical job when called from a worker completion scope. Other
/// callers retain ordinary `spawn_blocking` behavior. This must be used instead
/// of raw `spawn_blocking` by the worker's stores and verifier launcher.
pub fn spawn_blocking<F, T>(operation: F) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let completion = PENDING
        .try_with(|pending| {
            pending.active.fetch_add(1, Ordering::AcqRel);
            Completion(Arc::clone(pending))
        })
        .ok();
    tokio::task::spawn_blocking(move || {
        let _completion = completion;
        operation()
    })
}

/// Catch a recoverable operation panic and drain all registered physical work
/// before returning. The caller must retain this future in its detached owner
/// and release its lease/fence only afterward. Do not abort the owner or spawn
/// unscoped async descendants: Tokio task-local state is not inherited by them.
/// Process abort/runtime destruction is not a recoverable completion boundary.
pub async fn drain<T, F>(operation: F) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
{
    anyhow::ensure!(
        PENDING.try_with(|_| ()).is_err(),
        "physical-work scopes must not be nested"
    );
    let pending = Arc::new(PendingWork::default());
    let result = PENDING
        .scope(Arc::clone(&pending), async {
            AssertUnwindSafe(operation).catch_unwind().await
        })
        .await;
    // The operation can no longer enqueue work. notify_one retains a permit if
    // the final closure finishes between the count check and awaiting it.
    while pending.active.load(Ordering::Acquire) != 0 {
        pending.completed.notified().await;
    }
    result.unwrap_or_else(|_| {
        Err(anyhow::anyhow!(
            "worker operation panicked after physical-work drain"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn queued_physical_job_remains_owned_after_its_waiter_is_dropped() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (release, wait_release) = std::sync::mpsc::channel();
            let (occupied, wait_occupied) = tokio::sync::oneshot::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                occupied.send(()).unwrap();
                wait_release.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            wait_occupied.await.unwrap();
            let (registered, wait_registered) = tokio::sync::oneshot::channel();
            let completed = Arc::new(AtomicUsize::new(0));
            let job_completed = Arc::clone(&completed);
            let owner = tokio::spawn(drain(async move {
                drop(spawn_blocking(move || {
                    job_completed.store(1, Ordering::Release);
                }));
                registered.send(()).unwrap();
                Err::<(), _>(anyhow::anyhow!("operation failed before queued job ran"))
            }));
            wait_registered.await.unwrap();
            assert_eq!(completed.load(Ordering::Acquire), 0);
            assert!(
                !owner.is_finished(),
                "queued work escaped completion ownership"
            );
            release.send(()).unwrap();
            blocker.await.unwrap();
            assert!(owner.await.unwrap().is_err());
            assert_eq!(completed.load(Ordering::Acquire), 1);
        });
    }

    #[test]
    fn worker_store_and_launcher_jobs_use_the_completion_wrapper() {
        for source in [
            include_str!("secure_fs.rs"),
            include_str!("campaign_store.rs"),
            include_str!("replay_store.rs"),
            include_str!("verifier.rs"),
        ] {
            assert!(
                !source.contains("tokio::task::spawn_blocking"),
                "worker physical jobs must use the completion ownership wrapper"
            );
        }
    }
}
