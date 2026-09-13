//! One frame-polled completion slot shared by every leaderboard worker.
//!
//! Leaderboard work (HTTP, durable identity signing, owner-status polling)
//! runs inline, on a native worker thread, or as a browser `spawn_local`
//! future. Consumers only poll a [`PollTask`] once per graphical frame, so the
//! platform fork for starting work lives here and nowhere else.

use std::future::Future;

/// A result that a frame-polled consumer takes exactly once.
pub trait TryTake<T, E = String> {
    /// Return `None` while work remains pending. Implementations must never
    /// block the calling render frame.
    fn try_take(&mut self) -> Option<Result<T, E>>;
}

impl<T, E, F: FnMut() -> Option<Result<T, E>>> TryTake<T, E> for F {
    fn try_take(&mut self) -> Option<Result<T, E>> {
        self()
    }
}

/// The producing side of a [`PollTask`] went away without delivering a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskClosed;

/// Capacity-one completion channel polled without blocking.
pub struct PollTask<T> {
    receiver: async_channel::Receiver<T>,
    /// Kept only by tasks created already complete, so polling again after
    /// the value was taken reports "nothing more" (`None`) instead of a closed
    /// worker. Worker-fed tasks drop their sender when the worker finishes.
    completed_sender: Option<async_channel::Sender<T>>,
}

impl<T> std::fmt::Debug for PollTask<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollTask")
            .field("completed", &self.completed_sender.is_some())
            .finish_non_exhaustive()
    }
}

impl<T> PollTask<T> {
    /// A task fed by a caller-owned worker through the returned sender.
    pub fn channel() -> (async_channel::Sender<T>, Self) {
        let (sender, receiver) = async_channel::bounded(1);
        (
            sender,
            Self {
                receiver,
                completed_sender: None,
            },
        )
    }

    /// A task whose value is available on the first poll. Browser production
    /// work always completes asynchronously, so only native `start` and test
    /// fixtures construct completed tasks.
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn ready(value: T) -> Self {
        let (sender, receiver) = async_channel::bounded(1);
        sender
            .try_send(value)
            .expect("fresh capacity-one channel accepts one result");
        Self {
            receiver,
            completed_sender: Some(sender),
        }
    }

    /// Non-blocking completion check intended to run once per graphical frame.
    pub fn try_take(&self) -> Option<Result<T, TaskClosed>> {
        match self.receiver.try_recv() {
            Ok(value) => Some(Ok(value)),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => Some(Err(TaskClosed)),
        }
    }

    /// Await completion outside the frame loop (pre-frame admission). Never
    /// blocks a native executor thread or the browser event loop.
    pub async fn take(self) -> Result<T, TaskClosed> {
        self.receiver.recv().await.map_err(|_| TaskClosed)
    }
}

impl<T: 'static> PollTask<T> {
    /// Start `future` where the platform runs identity work.
    ///
    /// Native signers never suspend, so the future is driven to completion
    /// inline and the value is ready on this same frame. Browser signers
    /// cross into the isolated signer origin, so the future runs on
    /// `spawn_local` and completes on a later frame.
    pub fn start(future: impl Future<Output = T> + 'static) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::ready(futures::executor::block_on(future))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (sender, task) = Self::channel();
            wasm_bindgen_futures::spawn_local(async move {
                let value = future.await;
                let _ = sender.send(value).await;
            });
            task
        }
    }

    /// Run the future built by `make` off the graphical frame: on a named
    /// native worker thread, or as a browser `spawn_local` future. The future
    /// is built on the worker, so it does not need to be `Send`.
    pub fn spawn_background<F>(
        thread_name: &str,
        make: impl FnOnce() -> F + Send + 'static,
    ) -> std::io::Result<Self>
    where
        F: Future<Output = T> + 'static,
        T: Send,
    {
        let (sender, task) = Self::channel();
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::thread::Builder::new()
                .name(thread_name.to_owned())
                .spawn(move || {
                    let value = futures::executor::block_on(make());
                    let _ = sender.send_blocking(value);
                })?;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = thread_name;
            wasm_bindgen_futures::spawn_local(async move {
                let value = make().await;
                let _ = sender.send(value).await;
            });
        }
        Ok(task)
    }
}

impl<T, E> PollTask<Result<T, E>> {
    /// Poll a fallible task, reporting a vanished worker as `closed()`.
    pub fn poll(&self, closed: impl FnOnce() -> E) -> Option<Result<T, E>> {
        self.try_take()
            .map(|result| result.unwrap_or_else(|TaskClosed| Err(closed())))
    }

    /// Adapt into a boxed-friendly [`TryTake`] consumer.
    pub fn into_try_take(self, closed: impl Fn() -> E) -> impl FnMut() -> Option<Result<T, E>> {
        move || self.poll(&closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_task_yields_once_then_stays_empty() {
        let task = PollTask::ready(Ok::<_, String>(7));
        assert_eq!(task.poll(|| "closed".to_owned()), Some(Ok(7)));
        assert_eq!(task.poll(|| "closed".to_owned()), None);
    }

    #[test]
    fn worker_task_reports_pending_value_then_closed() {
        let (sender, task) = PollTask::<Result<u8, String>>::channel();
        assert_eq!(task.poll(|| "closed".to_owned()), None);
        sender.try_send(Ok(3)).unwrap();
        drop(sender);
        assert_eq!(task.poll(|| "closed".to_owned()), Some(Ok(3)));
        assert_eq!(
            task.poll(|| "closed".to_owned()),
            Some(Err("closed".to_owned()))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_start_is_ready_on_the_same_frame() {
        let task = PollTask::start(async { Ok::<_, String>(11) });
        assert_eq!(task.poll(|| "closed".to_owned()), Some(Ok(11)));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_background_task_completes_off_frame() {
        let task =
            PollTask::spawn_background("poll-task-test", || async { Ok::<_, String>(5) }).unwrap();
        assert_eq!(pollster::block_on(task.take()), Ok(Ok(5)));
    }
}
