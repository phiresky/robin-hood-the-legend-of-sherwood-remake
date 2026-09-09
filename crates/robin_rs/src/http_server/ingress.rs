//! Process transport routing is separate from mission-owned execution state.
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::Weak;

type Requests = Arc<Mutex<VecDeque<HttpRequest>>>;

/// Deserializing diagnostics never restores live request authority.
#[derive(Default, Serialize, Deserialize)]
pub struct RequestRouter {
    #[serde(skip)]
    active: Weak<Mutex<VecDeque<HttpRequest>>>,
    #[serde(skip)]
    idle: VecDeque<HttpRequest>,
}

impl RequestRouter {
    pub(super) fn take_idle(&mut self) -> Vec<HttpRequest> {
        self.idle.drain(..).collect()
    }
    pub(super) fn push_back(&mut self, request: HttpRequest) {
        if let Some(active) = self.active.upgrade() {
            active
                .lock()
                .expect("session RPC queue poisoned")
                .push_back(request);
        } else if matches!(
            request.payload,
            HttpPayload::GetReplay | HttpPayload::LoadReplay { .. }
        ) {
            // Replay import/export intentionally works between missions.
            self.idle.push_back(request);
        } else {
            request.response_tx.send(Err(
                "engine not ready — no active mission RPC session".into()
            ));
        }
    }
}

/// One mission's typed RPC inbox and deferred work. It is never cloned or
/// restored from saves: replacing a mission cancels its outstanding replies.
#[derive(Serialize)]
pub struct SessionIngress {
    #[serde(skip)]
    router: Option<Queue>,
    #[serde(skip)]
    requests: Requests,
    #[serde(skip)]
    steps: Vec<PendingStep>,
    #[serde(skip)]
    screenshots: Vec<PendingScreenshot>,
    taints: BTreeSet<InputTaintKind>,
    replay: Option<ReplayStatus>,
}

impl<'de> Deserialize<'de> for SessionIngress {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "live RPC ingress cannot be deserialized",
        ))
    }
}

impl SessionIngress {
    pub fn attach() -> Self {
        Self::with_router(GLOBAL.get().map(|server| server.queue.clone()))
    }

    #[cfg(test)]
    pub(crate) fn detached_for_test() -> Self {
        Self::with_router(None)
    }

    fn with_router(router: Option<Queue>) -> Self {
        let requests = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(router) = &router {
            let mut route = router.lock().expect("RPC router poisoned");
            assert!(
                route.active.upgrade().is_none(),
                "previous mission RPC session still active"
            );
            route.active = Arc::downgrade(&requests);
            // Only process-scoped replay operations can be queued while idle.
            requests
                .lock()
                .expect("session RPC queue poisoned")
                .extend(route.idle.drain(..));
        }
        Self {
            router,
            requests,
            steps: Vec::new(),
            screenshots: Vec::new(),
            taints: BTreeSet::new(),
            replay: None,
        }
    }

    pub(super) fn take_requests(&mut self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .expect("session RPC queue poisoned")
            .drain(..)
            .collect()
    }

    pub(super) fn observe_ranked_input_taint(&mut self, payload: &HttpPayload) {
        if let Some(kind) = ranked_input_taint(payload) {
            self.taints.insert(kind);
        }
    }

    pub fn take_pending_replay_taints(&mut self) -> BTreeSet<InputTaintKind> {
        std::mem::take(&mut self.taints)
    }

    pub fn set_replay_status(&mut self, replay: Option<ReplayStatus>) {
        self.replay = replay;
    }
    pub(super) fn replay_status(&self) -> Option<ReplayStatus> {
        self.replay
    }

    /// Shared graphical/headless deferral preserves the same ordered step queue.
    pub(super) fn defer_request(
        &mut self,
        request: DeferredRequest,
        response_tx: Responder,
        graphical: bool,
    ) {
        let kind = match request {
            DeferredRequest::Step(kind) => kind,
            DeferredRequest::Screenshot(request) => {
                if graphical {
                    self.screenshots.push(PendingScreenshot {
                        request,
                        response_tx,
                    });
                } else {
                    response_tx.send(Err(
                        "screenshots are unavailable in a headless runner".into()
                    ));
                }
                return;
            }
        };
        self.steps.push(PendingStep { response_tx, kind });
    }

    #[cfg(test)]
    fn defer(&mut self, request: HttpRequest, graphical: bool) -> Option<HttpRequest> {
        // Test adapter exercises the same exhaustive classification as production.
        let RoutedRequest::Deferred(deferred) = request.payload.classify() else {
            panic!("deferral test supplied a non-deferred operation");
        };
        self.defer_request(deferred, request.response_tx, graphical);
        None
    }

    pub fn take_pending_steps(&mut self) -> Vec<PendingStep> {
        std::mem::take(&mut self.steps)
    }

    pub fn take_pending_screenshots(&mut self, sim_frame: u32) -> Vec<PendingScreenshot> {
        self.take_screenshots_matching(sim_frame, |_| true)
    }

    pub fn take_pending_ui_screenshots(&mut self, sim_frame: u32) -> Vec<PendingScreenshot> {
        self.take_screenshots_matching(sim_frame, can_capture_presented_ui)
    }

    pub fn take_pending_scene_screenshots(&mut self, sim_frame: u32) -> Vec<PendingScreenshot> {
        self.take_screenshots_matching(sim_frame, |request| !can_capture_presented_ui(request))
    }

    fn take_screenshots_matching(
        &mut self,
        sim_frame: u32,
        predicate: impl Fn(&ScreenshotRequest) -> bool,
    ) -> Vec<PendingScreenshot> {
        let (ready, waiting) =
            std::mem::take(&mut self.screenshots)
                .into_iter()
                .partition(|pending| {
                    pending.request.frame.is_none_or(|frame| sim_frame >= frame)
                        && predicate(&pending.request)
                });
        self.screenshots = waiting;
        ready
    }
}

impl Drop for SessionIngress {
    fn drop(&mut self) {
        if let Some(router) = &self.router {
            // Routing and retirement use the same lock: a concurrent listener
            // either binds to this session before retirement or observes idle.
            router.lock().expect("RPC router poisoned").active = Weak::new();
        }
        let message = "mission ended before RPC request completed";
        for request in self.take_requests() {
            request.response_tx.send(Err(message.into()));
        }
        for step in self.steps.drain(..) {
            step.respond_err(message);
        }
        for screenshot in self.screenshots.drain(..) {
            screenshot.respond_err(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    type ReplyReceiver = mpsc::Receiver<Reply>;
    #[cfg(target_arch = "wasm32")]
    type ReplyReceiver = async_channel::Receiver<Reply>;

    fn request(payload: HttpPayload) -> (HttpRequest, ReplyReceiver) {
        #[cfg(not(target_arch = "wasm32"))]
        let (response_tx, rx) = {
            let (tx, rx) = mpsc::sync_channel(1);
            (Responder::Channel(tx), rx)
        };
        #[cfg(target_arch = "wasm32")]
        let (response_tx, rx) = {
            let (tx, rx) = async_channel::bounded(1);
            (Responder::Wasm(tx), rx)
        };
        (
            HttpRequest {
                payload,
                response_tx,
            },
            rx,
        )
    }

    fn assert_cancelled(rx: ReplyReceiver) {
        let reply = rx
            .try_recv()
            .expect("response must complete, not disconnect or wait");
        assert!(matches!(reply, Err(error) if error.contains("mission ended")));
    }

    fn router() -> Queue {
        Arc::new(Mutex::new(RequestRouter::default()))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn retired_session_cancels_raw_steps_and_future_screenshots() {
        let router = router();
        let mut old = SessionIngress::with_router(Some(router.clone()));
        let (step, step_reply) = request(HttpPayload::StepForward {
            request: StepRequest::default(),
        });
        let (shot, shot_reply) = request(HttpPayload::Screenshot(ScreenshotRequest {
            frame: Some(900),
            ..Default::default()
        }));
        assert!(old.defer(step, true).is_none());
        assert!(old.defer(shot, true).is_none());
        let (raw, raw_reply) = request(HttpPayload::SetPaused { paused: true });
        router.lock().unwrap().push_back(raw);
        old.observe_ranked_input_taint(&HttpPayload::SetPaused { paused: true });
        old.set_replay_status(Some(ReplayStatus {
            frame: 10,
            total: 90,
            paused: true,
        }));
        drop(old);
        assert_cancelled(step_reply);
        assert_cancelled(shot_reply);
        assert_cancelled(raw_reply);
        let mut replacement = SessionIngress::with_router(Some(router));
        assert!(replacement.take_requests().is_empty());
        assert!(replacement.take_pending_steps().is_empty());
        assert!(replacement.take_pending_screenshots(1000).is_empty());
        assert!(replacement.take_pending_replay_taints().is_empty());
        assert!(replacement.replay_status().is_none());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn idle_rejects_gameplay_but_preserves_process_replay_operations() {
        let router = router();
        let (state, state_reply) = request(HttpPayload::State);
        router.lock().unwrap().push_back(state);
        assert!(
            matches!(state_reply.try_recv().unwrap(), Err(error) if error.contains("engine not ready"))
        );
        let (replay, replay_reply) = request(HttpPayload::GetReplay);
        router.lock().unwrap().push_back(replay);
        let mut session = SessionIngress::with_router(Some(router));
        let mut pending = session.take_requests();
        assert_eq!(pending.len(), 1);
        let request = pending.pop().unwrap();
        assert!(matches!(request.payload, HttpPayload::GetReplay));
        request
            .response_tx
            .send(Ok(serde_json::json!({"ok":true}).into()));
        assert!(replay_reply.try_recv().unwrap().is_ok());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn graphical_and_headless_steps_keep_identical_fifo() {
        for graphical in [false, true] {
            let router = router();
            let mut session = SessionIngress::with_router(Some(router.clone()));
            let mut replies = Vec::new();
            for payload in [
                HttpPayload::SetPaused { paused: true },
                HttpPayload::StepForward {
                    request: StepRequest {
                        n: 3,
                        ..Default::default()
                    },
                },
                HttpPayload::SetPaused { paused: false },
            ] {
                let (request, reply) = request(payload);
                router.lock().unwrap().push_back(request);
                replies.push(reply);
            }
            for request in session.take_requests() {
                session.observe_ranked_input_taint(&request.payload);
                assert!(session.defer(request, graphical).is_none());
            }
            assert_eq!(
                session.take_pending_replay_taints(),
                BTreeSet::from([InputTaintKind::HttpSimulationStep])
            );
            let steps = session.take_pending_steps();
            assert_eq!(
                steps
                    .iter()
                    .map(|step| step.kind.clone())
                    .collect::<Vec<_>>(),
                vec![
                    StepKind::SetPaused { paused: true },
                    StepKind::Forward {
                        n: 3,
                        modal_policy: StepModalPolicy::default()
                    },
                    StepKind::SetPaused { paused: false }
                ]
            );
            for step in steps {
                step.respond_ok(serde_json::json!({"ok":true}));
            }
            assert!(
                replies
                    .into_iter()
                    .all(|reply| reply.try_recv().unwrap().is_ok())
            );
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn screenshot_partition_preserves_waiting_order_and_headless_rejection() {
        let mut session = SessionIngress::with_router(None);
        let mut replies = Vec::new();
        for (frame, hide_ui, width) in [(Some(20), false, 1), (None, true, 2), (None, false, 3)] {
            let (request, reply) = request(HttpPayload::Screenshot(ScreenshotRequest {
                frame,
                hide_ui,
                width: Some(width),
                ..Default::default()
            }));
            session.defer(request, true);
            replies.push(reply);
        }
        let ui = session.take_pending_ui_screenshots(10);
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].request().width, Some(3));
        for shot in ui {
            shot.respond_err("captured UI");
        }
        let scene = session.take_pending_scene_screenshots(10);
        assert_eq!(scene.len(), 1);
        assert_eq!(scene[0].request().width, Some(2));
        for shot in scene {
            shot.respond_err("captured scene");
        }
        assert!(session.take_pending_screenshots(19).is_empty());
        let future = session.take_pending_screenshots(20);
        assert_eq!(future.len(), 1);
        assert_eq!(future[0].request().width, Some(1));
        for shot in future {
            shot.respond_err("captured future");
        }
        assert!(
            replies
                .into_iter()
                .all(|reply| reply.try_recv().unwrap().is_err())
        );
        let (request, reply) = request(HttpPayload::Screenshot(ScreenshotRequest::default()));
        session.defer(request, false);
        assert!(matches!(reply.try_recv().unwrap(), Err(error) if error.contains("headless")));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn listener_racing_retirement_never_leaks_into_replacement() {
        let router = router();
        let session = SessionIngress::with_router(Some(router.clone()));
        let sender = router.clone();
        let listener = std::thread::spawn(move || {
            let mut replies = Vec::new();
            for _ in 0..32 {
                let (request, reply) = request(HttpPayload::State);
                sender.lock().unwrap().push_back(request);
                replies.push(reply);
            }
            replies
        });
        drop(session);
        let replies = listener.join().unwrap();
        for reply in replies {
            assert!(reply.try_recv().unwrap().is_err());
        }
        let mut replacement = SessionIngress::with_router(Some(router));
        assert!(replacement.take_requests().is_empty());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn diagnostics_cannot_restore_live_rpc_authority() {
        let session = SessionIngress::with_router(None);
        let diagnostic = serde_json::to_value(&session).unwrap();
        assert!(serde_json::from_value::<SessionIngress>(diagnostic).is_err());
    }
}
