//! Application-owned cache maintenance. Views observe, never own, completion.
use futures::channel::oneshot;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[cfg(not(target_arch = "wasm32"))]
type Worker = std::thread::JoinHandle<()>;
#[cfg(target_arch = "wasm32")]
type Worker = futures::future::AbortHandle;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CacheClearStatus {
    #[default]
    Idle,
    Pending {
        operation: u64,
    },
    Completed {
        operation: u64,
        result: Result<usize, String>,
    },
}

impl CacheClearStatus {
    pub(crate) fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    closed: bool,
    #[serde(skip)]
    worker: Option<Worker>,
    next_operation: u64,
    status: CacheClearStatus,
    #[serde(skip)]
    receiver: Option<oneshot::Receiver<Result<usize, String>>>,
}

impl State {
    fn retire_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            #[cfg(not(target_arch = "wasm32"))]
            if worker.join().is_err() {
                tracing::error!("cache-clear worker panicked");
            }
            #[cfg(target_arch = "wasm32")]
            worker.abort();
        }
    }

    fn poll(&mut self) {
        let Some(receiver) = self.receiver.as_mut() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(None) => return,
            Ok(Some(result)) => result,
            Err(error) => Err(format!(
                "cache-clear worker disconnected without reporting completion: {error}"
            )),
        };
        let CacheClearStatus::Pending { operation } = self.status else {
            panic!("cache-clear receiver requires a pending operation");
        };
        self.receiver = None;
        self.retire_worker();
        self.status = CacheClearStatus::Completed { operation, result };
    }
}

/// Default/decoded instances have no runtime authority. Application startup
/// explicitly constructs an owner; serialized notices cannot admit work.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct CacheMaintenance {
    #[serde(skip)]
    runtime: Option<Mutex<State>>,
}

impl CacheMaintenance {
    pub(crate) fn new() -> Self {
        Self {
            runtime: Some(Mutex::new(State::default())),
        }
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>, String> {
        self.runtime
            .as_ref()
            .ok_or_else(|| "application cache-maintenance authority is unavailable".to_owned())?
            .lock()
            .map_err(|_| "application cache-maintenance lock poisoned".to_owned())
    }

    pub(crate) fn status(&self) -> Result<CacheClearStatus, String> {
        let mut state = self.state()?;
        state.poll();
        Ok(state.status.clone())
    }

    /// Admission and receiver installation share one lock. A second panel gets
    /// the pending operation, never a second worker. Completed notices remain
    /// observable until an explicitly requested subsequent operation replaces them.
    fn begin_with(
        &self,
        start: impl FnOnce(oneshot::Sender<Result<usize, String>>) -> Result<Option<Worker>, String>,
    ) -> Result<CacheClearStatus, String> {
        let mut state = self.state()?;
        state.poll();
        if state.closed {
            return Err("cache maintenance is shut down".into());
        }
        if state.status.is_pending() {
            return Ok(state.status.clone());
        }
        let operation = state
            .next_operation
            .checked_add(1)
            .ok_or_else(|| "cache-clear operation identity exhausted".to_owned())?;
        state.next_operation = operation;
        let (sender, receiver) = oneshot::channel();
        state.status = CacheClearStatus::Pending { operation };
        state.receiver = Some(receiver);
        match start(sender) {
            Ok(worker) => state.worker = worker,
            Err(error) => {
                state.receiver = None;
                state.status = CacheClearStatus::Completed {
                    operation,
                    result: Err(error),
                };
            }
        }
        Ok(state.status.clone())
    }

    pub(crate) fn begin(
        &self,
        #[cfg(not(target_arch = "wasm32"))] cache: std::sync::Arc<
            Mutex<Result<crate::distributed_mod_cache::DistributedModCache, String>>,
        >,
    ) -> Result<CacheClearStatus, String> {
        self.begin_with(move |sender| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                std::thread::Builder::new()
                    .name("spellforge-cache-clear".to_owned())
                    .spawn(move || {
                        let result = cache
                            .lock()
                            .map_err(|_| "distributed-mod cache lock poisoned".to_owned())
                            .and_then(|mut cache| match cache.as_mut() {
                                Ok(cache) => cache.clear(),
                                Err(error) => {
                                    Err(format!("distributed-mod cache is unavailable: {error}"))
                                }
                            });
                        if let Err(result) = sender.send(result) {
                            tracing::error!(?result, "cache-clear completion receiver unavailable");
                        }
                    })
                    .map(Some)
                    .map_err(|error| format!("could not start cache-clear worker: {error}"))
            }
            #[cfg(target_arch = "wasm32")]
            {
                let (abort, registration) = futures::future::AbortHandle::new_pair();
                wasm_bindgen_futures::spawn_local(async move {
                    let task = async move {
                        let result = crate::distributed_mod_cache::clear().await;
                        if let Err(result) = sender.send(result) {
                            tracing::error!(?result, "cache-clear completion receiver unavailable");
                        }
                    };
                    let _ = futures::future::Abortable::new(task, registration).await;
                });
                Ok(Some(abort))
            }
        })
    }

    /// Close admission and await physical work before application services retire.
    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        if self.runtime.is_none() {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut state = self.state()?;
            state.closed = true;
            state.retire_worker();
            state.poll();
            if let CacheClearStatus::Completed {
                result: Err(error), ..
            } = &state.status
            {
                return Err(error.clone());
            }
            Ok(())
        }
        #[cfg(target_arch = "wasm32")]
        {
            let pending = {
                let mut state = self.state()?;
                state.closed = true;
                state.poll();
                state
                    .receiver
                    .take()
                    .map(|receiver| (state.next_operation, receiver))
            };
            if let Some((operation, receiver)) = pending {
                let result = receiver.await.unwrap_or_else(|error| {
                    Err(format!(
                        "cache-clear worker disconnected during shutdown: {error}"
                    ))
                });
                let mut state = self.state()?;
                state.retire_worker();
                state.status = CacheClearStatus::Completed {
                    operation,
                    result: result.clone(),
                };
                result.map(|_| ())
            } else {
                match self.status()? {
                    CacheClearStatus::Completed {
                        result: Err(error), ..
                    } => Err(error),
                    CacheClearStatus::Pending { .. } => {
                        Err("cache shutdown is already pending".into())
                    }
                    _ => Ok(()),
                }
            }
        }
    }

    /// Native Drop joins; browser Drop cancels future continuations. An already
    /// submitted IndexedDB request may complete, but cannot schedule further work.
    pub(crate) fn shutdown_on_drop(&self) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let mut state = runtime.lock().unwrap_or_else(|error| {
            tracing::error!("cache maintenance poisoned during shutdown");
            error.into_inner()
        });
        state.closed = true;
        state.retire_worker();
        state.poll();
    }
}

impl Drop for CacheMaintenance {
    fn drop(&mut self) {
        self.shutdown_on_drop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_shutdown_and_drop_join_workers_and_close_admission() {
        for explicit in [false, true] {
            let owner = CacheMaintenance::new();
            let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let worker_finished = finished.clone();
            owner
                .begin_with(|sender| {
                    Ok(Some(std::thread::spawn(move || {
                        sender.send(Ok(7)).unwrap();
                        worker_finished.store(true, std::sync::atomic::Ordering::SeqCst);
                    })))
                })
                .unwrap();
            if explicit {
                pollster::block_on(owner.shutdown()).unwrap();
                assert!(
                    owner
                        .begin_with(|_| panic!("closed owner admitted work"))
                        .is_err()
                );
                assert!(matches!(
                    owner.status().unwrap(),
                    CacheClearStatus::Completed { result: Ok(7), .. }
                ));
            }
            drop(owner);
            assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "requires LLVM panic unwinding; run explicitly with test codegen-backend=llvm"]
    fn native_worker_panic_is_joined_and_reported_at_shutdown() {
        let owner = CacheMaintenance::new();
        owner
            .begin_with(|sender| {
                Ok(Some(std::thread::spawn(move || {
                    let _sender = sender;
                    panic!("injected cache worker panic");
                })))
            })
            .unwrap();
        assert!(
            pollster::block_on(owner.shutdown())
                .unwrap_err()
                .contains("disconnected")
        );
        assert!(owner.state().unwrap().worker.is_none());
        assert!(
            owner
                .begin_with(|_| panic!("closed owner admitted work"))
                .is_err()
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn browser_shutdown_waits_for_work_and_closes_admission() {
        let owner = CacheMaintenance::new();
        let (release, gate) = oneshot::channel::<()>();
        owner
            .begin_with(|sender| {
                let (abort, registration) = futures::future::AbortHandle::new_pair();
                wasm_bindgen_futures::spawn_local(async move {
                    let _ = futures::future::Abortable::new(
                        async move {
                            gate.await.unwrap();
                            sender.send(Ok(7)).unwrap();
                        },
                        registration,
                    )
                    .await;
                });
                Ok(Some(abort))
            })
            .unwrap();
        let shutdown = owner.shutdown();
        futures::pin_mut!(shutdown);
        assert!(futures::poll!(&mut shutdown).is_pending());
        assert!(
            owner
                .begin_with(|_| panic!("closed owner admitted work"))
                .is_err()
        );
        release.send(()).unwrap();
        shutdown.await.unwrap();
        assert!(matches!(
            owner.status().unwrap(),
            CacheClearStatus::Completed { result: Ok(7), .. }
        ));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn browser_drop_cancels_worker_continuations() {
        let owner = CacheMaintenance::new();
        let continued = std::rc::Rc::new(std::cell::Cell::new(false));
        let worker_continued = continued.clone();
        let (release, gate) = oneshot::channel::<()>();
        let (finished, completion) = oneshot::channel();
        owner
            .begin_with(|sender| {
                let (abort, registration) = futures::future::AbortHandle::new_pair();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = futures::future::Abortable::new(
                        async move {
                            let _ = gate.await;
                            worker_continued.set(true);
                            let _ = sender.send(Ok(1));
                        },
                        registration,
                    )
                    .await;
                    finished.send(result.is_err()).unwrap();
                });
                Ok(Some(abort))
            })
            .unwrap();
        drop(owner);
        let _ = release.send(());
        assert!(completion.await.unwrap());
        assert!(!continued.get());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn panel_close_and_reopen_retains_pending_then_success_or_failure() {
        for result in [Ok(4), Err("mounted content prevents clearing".into())] {
            let application = CacheMaintenance::new();
            let mut worker = None;
            let panel = application
                .begin_with(|sender| {
                    worker = Some(sender);
                    Ok(None)
                })
                .unwrap();
            assert!(panel.is_pending());
            drop(panel);
            assert!(application.status().unwrap().is_pending());
            worker.take().unwrap().send(result.clone()).unwrap();
            let reopened = application.status().unwrap();
            assert_eq!(
                reopened,
                CacheClearStatus::Completed {
                    operation: 1,
                    result
                }
            );
            drop(reopened);
            assert!(matches!(
                application.status().unwrap(),
                CacheClearStatus::Completed { .. }
            ));
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn pending_request_does_not_admit_another_worker() {
        let application = CacheMaintenance::new();
        let mut worker = None;
        let first = application
            .begin_with(|sender| {
                worker = Some(sender);
                Ok(None)
            })
            .unwrap();
        assert_eq!(
            application
                .begin_with(|_| panic!("duplicate worker admitted"))
                .unwrap(),
            first
        );
        worker.unwrap().send(Ok(0)).unwrap();
        assert!(matches!(
            application.status().unwrap(),
            CacheClearStatus::Completed { operation: 1, .. }
        ));
        let second = application
            .begin_with(|sender| {
                sender.send(Ok(2)).unwrap();
                Ok(None)
            })
            .unwrap();
        assert_eq!(second, CacheClearStatus::Pending { operation: 2 });
        assert_eq!(
            application.status().unwrap(),
            CacheClearStatus::Completed {
                operation: 2,
                result: Ok(2)
            }
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn worker_disconnect_and_spawn_failure_are_retained_errors() {
        let application = CacheMaintenance::new();
        application
            .begin_with(|sender| {
                drop(sender);
                Ok(None)
            })
            .unwrap();
        let disconnected = application.status().unwrap();
        assert!(
            matches!(&disconnected, CacheClearStatus::Completed { result: Err(error), .. } if error.contains("disconnected without reporting completion"))
        );
        assert_eq!(application.status().unwrap(), disconnected);
        let failed = application
            .begin_with(|_| Err("thread unavailable".into()))
            .unwrap();
        assert_eq!(
            failed,
            CacheClearStatus::Completed {
                operation: 2,
                result: Err("thread unavailable".into())
            }
        );
        assert_eq!(application.status().unwrap(), failed);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn independent_applications_and_decoded_data_have_no_shared_authority() {
        let first = CacheMaintenance::new();
        let second = CacheMaintenance::new();
        let mut worker = None;
        first
            .begin_with(|sender| {
                worker = Some(sender);
                Ok(None)
            })
            .unwrap();
        assert_eq!(second.status().unwrap(), CacheClearStatus::Idle);
        second
            .begin_with(|sender| {
                sender.send(Ok(8)).unwrap();
                Ok(None)
            })
            .unwrap();
        assert!(first.status().unwrap().is_pending());
        assert_eq!(
            second.status().unwrap(),
            CacheClearStatus::Completed {
                operation: 1,
                result: Ok(8)
            }
        );
        let decoded: CacheMaintenance =
            serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
        assert!(decoded.status().is_err());
        assert!(
            decoded
                .begin_with(|_| panic!("decoded authority admitted work"))
                .is_err()
        );
        worker.unwrap().send(Ok(1)).unwrap();
    }
}
