//! Application-owned cache maintenance. Views observe, never own, completion.
use futures::channel::oneshot;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

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
    next_operation: u64,
    status: CacheClearStatus,
    #[serde(skip)]
    receiver: Option<oneshot::Receiver<Result<usize, String>>>,
}

impl State {
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
        start: impl FnOnce(oneshot::Sender<Result<usize, String>>) -> Result<(), String>,
    ) -> Result<CacheClearStatus, String> {
        let mut state = self.state()?;
        state.poll();
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
        if let Err(error) = start(sender) {
            state.receiver = None;
            state.status = CacheClearStatus::Completed {
                operation,
                result: Err(error),
            };
        }
        Ok(state.status.clone())
    }

    pub(crate) fn begin(
        &self,
        application: crate::host::ApplicationContext,
    ) -> Result<CacheClearStatus, String> {
        self.begin_with(move |sender| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                std::thread::Builder::new()
                    .name("spellforge-cache-clear".to_owned())
                    .spawn(move || {
                        // The application clone retains the receiver throughout
                        // physical work, even if every settings panel closes.
                        let result = application.clear_distributed_mod_cache();
                        if let Err(result) = sender.send(result) {
                            tracing::error!(?result, "cache-clear completion receiver unavailable");
                        }
                    })
                    .map(|_| ())
                    .map_err(|error| format!("could not start cache-clear worker: {error}"))
            }
            #[cfg(target_arch = "wasm32")]
            {
                wasm_bindgen_futures::spawn_local(async move {
                    let _application = application;
                    let result = crate::distributed_mod_cache::clear().await;
                    if let Err(result) = sender.send(result) {
                        tracing::error!(?result, "cache-clear completion receiver unavailable");
                    }
                });
                Ok(())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_close_and_reopen_retains_pending_then_success_or_failure() {
        for result in [Ok(4), Err("mounted content prevents clearing".into())] {
            let application = CacheMaintenance::new();
            let mut worker = None;
            let panel = application
                .begin_with(|sender| {
                    worker = Some(sender);
                    Ok(())
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

    #[test]
    fn pending_request_does_not_admit_another_worker() {
        let application = CacheMaintenance::new();
        let mut worker = None;
        let first = application
            .begin_with(|sender| {
                worker = Some(sender);
                Ok(())
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
                Ok(())
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

    #[test]
    fn worker_disconnect_and_spawn_failure_are_retained_errors() {
        let application = CacheMaintenance::new();
        application
            .begin_with(|sender| {
                drop(sender);
                Ok(())
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

    #[test]
    fn independent_applications_and_decoded_data_have_no_shared_authority() {
        let first = CacheMaintenance::new();
        let second = CacheMaintenance::new();
        let mut worker = None;
        first
            .begin_with(|sender| {
                worker = Some(sender);
                Ok(())
            })
            .unwrap();
        assert_eq!(second.status().unwrap(), CacheClearStatus::Idle);
        second
            .begin_with(|sender| {
                sender.send(Ok(8)).unwrap();
                Ok(())
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
