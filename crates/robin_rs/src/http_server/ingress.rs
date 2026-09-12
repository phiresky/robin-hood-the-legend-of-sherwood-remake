//! Process transport routing is separate from mission-owned execution state.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Weak;

type Requests = Arc<Mutex<VecDeque<HttpRequest>>>;

/// Each capture owns a GPU readback buffer until completion or cancellation.
/// Keep both allocation pressure and per-turn PNG completion work bounded.
const MAX_IN_FLIGHT_CAPTURES: usize = 2;

/// Deserializing diagnostics never restores live request authority.
#[derive(Serialize, Deserialize)]
pub struct RequestRouter {
    retired: bool,
    #[serde(skip, default = "retirement_channel")]
    retirement: (async_channel::Sender<()>, async_channel::Receiver<()>),
    #[serde(skip)]
    active: Weak<Mutex<VecDeque<HttpRequest>>>,
    #[serde(skip)]
    idle: VecDeque<HttpRequest>,
}

fn retirement_channel() -> (async_channel::Sender<()>, async_channel::Receiver<()>) {
    async_channel::bounded(1)
}

impl Default for RequestRouter {
    fn default() -> Self {
        Self {
            retired: false,
            retirement: retirement_channel(),
            active: Weak::new(),
            idle: VecDeque::new(),
        }
    }
}

impl RequestRouter {
    pub(super) fn is_retired(&self) -> bool {
        self.retired
    }
    #[cfg(any(test, feature = "script-rpc", target_arch = "wasm32"))]
    pub(super) fn retirement_receiver(&self) -> async_channel::Receiver<()> {
        self.retirement.1.clone()
    }
    pub(super) fn retire(&mut self) {
        self.retired = true;
        self.retirement.0.close();
        for request in self.idle.drain(..) {
            request
                .response_tx
                .send(Err(RpcError::retired("HTTP transport stopped")));
        }
        if let Some(active) = self.active.upgrade() {
            for request in active.lock().expect("session RPC queue poisoned").drain(..) {
                request
                    .response_tx
                    .send(Err(RpcError::retired("HTTP transport stopped")));
            }
        }
    }

    pub(super) fn take_idle(&mut self) -> VecDeque<HttpRequest> {
        std::mem::take(&mut self.idle)
    }
    #[cfg(any(test, feature = "script-rpc", target_arch = "wasm32"))]
    pub(super) fn push_back(&mut self, request: HttpRequest) {
        if self.retired {
            request
                .response_tx
                .send(Err(RpcError::retired("HTTP transport stopped")));
        } else if let Some(active) = self.active.upgrade() {
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
            request
                .response_tx
                .send(Err(RpcError::unavailable_capability(
                    "engine not ready — no active mission RPC session",
                )));
        }
    }
}

/// One mission's typed RPC inbox and deferred work. It is never cloned or
/// restored from saves: replacing a mission cancels its outstanding replies.
#[derive(Serialize)]
pub struct SessionIngress {
    pub(super) replay_capabilities: Option<(
        crate::replay_service::ReplayExports,
        crate::replay_service::ReplayLaunches,
    )>,
    #[serde(skip)]
    router: Option<Queue>,
    #[serde(skip)]
    requests: Requests,
    #[serde(skip)]
    steps: Vec<PendingStep>,
    #[serde(skip)]
    screenshots: Vec<PendingScreenshot>,
    #[serde(skip)]
    captures: Vec<(PendingScreenshot, crate::renderer::PendingCapture)>,
    taints: BTreeSet<InputTaintKind>,
    replay: Option<ReplayStatus>,
}

robin_util::deny_deserialize!(SessionIngress, "live RPC ingress cannot be deserialized");

impl SessionIngress {
    pub(super) fn attach(server: Option<&HttpServer>) -> Self {
        let mut ingress = Self::with_router(server.map(|server| server.queue.clone()));
        ingress.replay_capabilities = server.map(|server| {
            (
                server.replay_exports.clone(),
                server.replay_launches.clone(),
            )
        });
        ingress
    }

    #[cfg(test)]
    pub(crate) fn detached_for_test() -> Self {
        Self::with_router(None)
    }

    #[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
    pub(super) fn enqueue_for_test(&mut self, payload: HttpPayload) -> request_lifetime::ReplyWait {
        let (response_tx, receiver) = Responder::channel();
        self.requests
            .lock()
            .expect("test inbox poisoned")
            .push_back(HttpRequest {
                payload,
                response_tx,
            });
        receiver
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
            *requests.lock().expect("session RPC queue poisoned") = route.take_idle();
        }
        Self {
            replay_capabilities: None,
            router,
            requests,
            steps: Vec::new(),
            screenshots: Vec::new(),
            captures: Vec::new(),
            taints: BTreeSet::new(),
            replay: None,
        }
    }

    pub(super) fn take_requests(&mut self) -> Vec<HttpRequest> {
        self.cancel_stopped_work();
        self.poll_captures();
        let mut pending = {
            let mut inbox = self.requests.lock().expect("session RPC queue poisoned");
            std::mem::take(&mut *inbox)
        };
        // Eligibility checks and discarded responders do not need inbox access.
        pending.retain(|request| request.response_tx.eligible());
        pending.into()
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
                    response_tx.send(Err(RpcError::unavailable_capability(
                        "screenshots are unavailable in a headless runner",
                    )));
                }
                return;
            }
        };
        self.steps.push(PendingStep { response_tx, kind });
    }

    #[cfg(test)]
    fn defer(&mut self, request: HttpRequest, graphical: bool) {
        // Test adapter exercises the same exhaustive classification as production.
        let RoutedRequest::Deferred(deferred) = request.payload.classify() else {
            panic!("deferral test supplied a non-deferred operation");
        };
        self.defer_request(deferred, request.response_tx, graphical);
    }

    pub fn take_pending_steps(&mut self) -> Vec<PendingStep> {
        self.cancel_stopped_work();
        std::mem::take(&mut self.steps)
            .into_iter()
            .filter(|step| step.response_tx.admit())
            .collect()
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
        self.cancel_stopped_work();
        self.poll_captures();
        // Reserve before rendering/allocating GPU buffers. Leave excess work
        // cancellable in ingress, preserving its original requested frame gate.
        let mut available = MAX_IN_FLIGHT_CAPTURES.saturating_sub(self.captures.len());
        let (ready, waiting) = std::mem::take(&mut self.screenshots)
            .into_iter()
            .filter(|pending| pending.response_tx.eligible())
            .partition(|pending| {
                let ready = available > 0
                    && pending.request.frame.is_none_or(|frame| sim_frame >= frame)
                    && predicate(&pending.request);
                if ready {
                    available -= 1;
                }
                ready
            });
        self.screenshots = waiting;
        let ready: Vec<PendingScreenshot> = ready;
        ready
            .into_iter()
            .filter(|pending| pending.response_tx.admit())
            .collect()
    }

    pub(crate) fn submit_screenshot(
        &mut self,
        request: PendingScreenshot,
        capture: crate::renderer::PendingCapture,
    ) {
        assert!(
            self.captures.len() < MAX_IN_FLIGHT_CAPTURES,
            "screenshot admission exceeded capture quota"
        );
        self.captures.push((request, capture));
    }

    fn poll_captures(&mut self) {
        use futures::FutureExt;
        let mut index = 0;
        while index < self.captures.len() {
            let (request, capture) = &mut self.captures[index];
            if request.response_tx.consumer_gone() {
                drop(self.captures.remove(index));
                continue;
            }
            let Some(result) = capture.as_mut().now_or_never() else {
                index += 1;
                continue;
            };
            // Preserve pending order and allocation; this queue holds at most
            // MAX_IN_FLIGHT_CAPTURES entries, so shifting survivors is bounded.
            let (request, _capture) = self.captures.remove(index);
            match result {
                Ok((width, height, rgba)) => request.respond(width, height, &rgba),
                Err(error) => request.respond_err(RpcError::internal(error.to_string())),
            }
        }
    }

    fn cancel_stopped_work(&mut self) {
        if self
            .router
            .as_ref()
            .is_some_and(|router| router.lock().expect("RPC router poisoned").is_retired())
        {
            for step in self.steps.drain(..) {
                step.respond_err(RpcError::retired("HTTP transport stopped"));
            }
            for screenshot in self.screenshots.drain(..) {
                screenshot.respond_err(RpcError::retired("HTTP transport stopped"));
            }
            for (screenshot, _) in self.captures.drain(..) {
                screenshot.respond_err(RpcError::retired("HTTP transport stopped"));
            }
        }
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
        for (screenshot, _) in self.captures.drain(..) {
            screenshot.respond_err(RpcError::retired(message));
        }
        for request in self.take_requests() {
            request.response_tx.send(Err(RpcError::retired(message)));
        }
        for step in self.steps.drain(..) {
            step.respond_err(RpcError::retired(message));
        }
        for screenshot in self.screenshots.drain(..) {
            screenshot.respond_err(RpcError::retired(message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type ReplyReceiver = request_lifetime::ReplyWait;

    fn request(payload: HttpPayload) -> (HttpRequest, ReplyReceiver) {
        let (response_tx, rx) = Responder::channel();
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
        assert!(
            matches!(reply, Err(error) if error.kind == RpcErrorKind::Retired && error.message.contains("mission ended"))
        );
    }

    fn router() -> Queue {
        Arc::new(Mutex::new(RequestRouter::default()))
    }

    fn queued_screenshot(session: &mut SessionIngress) -> ReplyReceiver {
        let (shot, reply) = request(HttpPayload::Screenshot(ScreenshotRequest::default()));
        session.defer(shot, true);
        reply
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn capture_quota_keeps_excess_requests_queued_until_owned_readback_completes() {
        let mut session = SessionIngress::detached_for_test();
        let first = queued_screenshot(&mut session);
        let second = queued_screenshot(&mut session);
        let third = queued_screenshot(&mut session);
        let mut ready = session.take_pending_screenshots(0).into_iter();
        let (complete, receive) = futures::channel::oneshot::channel();
        session.submit_screenshot(
            ready.next().unwrap(),
            Box::pin(async move { receive.await.expect("test completion owner retained") }),
        );
        session.submit_screenshot(ready.next().unwrap(), Box::pin(std::future::pending()));
        assert!(ready.next().is_none());
        assert!(session.take_pending_screenshots(1).is_empty());
        assert!(first.try_recv().is_err());
        // The submitted pixels belong to the earlier frame even though the
        // simulation has advanced before the map callback arrives.
        complete.send(Ok((1, 1, vec![12, 34, 56, 255]))).unwrap();
        let next = session.take_pending_screenshots(20);
        assert_eq!(next.len(), 1);
        let Ok(ReplyBody::Binary {
            content_type: "image/png",
            data,
        }) = first.try_recv().unwrap()
        else {
            panic!("completed screenshot must return PNG");
        };
        let mut png = png::Decoder::new(std::io::Cursor::new(data))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; png.output_buffer_size().unwrap()];
        let info = png.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (1, 1));
        assert_eq!(&pixels[..info.buffer_size()], &[12, 34, 56, 255]);
        assert!(second.try_recv().is_err());
        next.into_iter()
            .next()
            .unwrap()
            .respond_err(RpcError::internal("test third"));
        assert!(third.try_recv().unwrap().is_err());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn disconnected_capture_releases_buffer_owner_and_quota() {
        let mut session = SessionIngress::detached_for_test();
        let reply = queued_screenshot(&mut session);
        let shot = session.take_pending_screenshots(0).pop().unwrap();
        let owner = std::rc::Rc::new(());
        let pending_owner = owner.clone();
        session.submit_screenshot(
            shot,
            Box::pin(async move {
                let _owner = pending_owner;
                std::future::pending().await
            }),
        );
        assert_eq!(std::rc::Rc::strong_count(&owner), 2);
        drop(reply);
        session.poll_captures();
        assert!(session.captures.is_empty());
        assert_eq!(std::rc::Rc::strong_count(&owner), 1);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn capture_failure_and_session_exit_complete_replies() {
        let mut session = SessionIngress::detached_for_test();
        let failed = queued_screenshot(&mut session);
        let stopped = queued_screenshot(&mut session);
        let mut ready = session.take_pending_screenshots(0).into_iter();
        session.submit_screenshot(
            ready.next().unwrap(),
            Box::pin(async {
                Err(crate::renderer::CaptureError::Map(
                    "synthetic mapping failure".into(),
                ))
            }),
        );
        session.submit_screenshot(ready.next().unwrap(), Box::pin(std::future::pending()));
        session.poll_captures();
        assert!(
            matches!(failed.try_recv().unwrap(), Err(error) if error.message.contains("synthetic mapping failure"))
        );
        drop(session);
        assert_cancelled(stopped);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn capture_polling_preserves_storage_and_polls_each_survivor_once() {
        for completed_index in 0..2 {
            let mut session = SessionIngress::detached_for_test();
            let replies = [
                queued_screenshot(&mut session),
                queued_screenshot(&mut session),
            ];
            let polls = std::rc::Rc::new(std::cell::Cell::new(0));
            for (index, shot) in session.take_pending_screenshots(0).into_iter().enumerate() {
                let polls = polls.clone();
                session.submit_screenshot(
                    shot,
                    Box::pin(std::future::poll_fn(move |_| {
                        if index == completed_index {
                            std::task::Poll::Ready(Err(crate::renderer::CaptureError::Map(
                                "finished".into(),
                            )))
                        } else {
                            polls.set(polls.get() + 1);
                            std::task::Poll::Pending
                        }
                    })),
                );
            }
            let storage = session.captures.as_ptr();
            let capacity = session.captures.capacity();
            for expected_polls in 1..=3 {
                session.poll_captures();
                assert_eq!(polls.get(), expected_polls);
                assert_eq!(session.captures.len(), 1);
                assert_eq!(session.captures.as_ptr(), storage);
                assert_eq!(session.captures.capacity(), capacity);
            }
            assert!(
                matches!(replies[completed_index].try_recv().unwrap(), Err(error) if error.message.contains("finished"))
            );
            assert!(replies[1 - completed_index].try_recv().is_err());
            drop(replies);
            session.poll_captures();
            assert_eq!(polls.get(), 3, "cancelled captures must not be polled");
            assert!(session.captures.is_empty());
            assert_eq!(session.captures.capacity(), capacity);
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn transport_retirement_releases_admitted_capture() {
        let router = router();
        let mut session = SessionIngress::with_router(Some(router.clone()));
        let reply = queued_screenshot(&mut session);
        let shot = session.take_pending_screenshots(0).pop().unwrap();
        session.submit_screenshot(shot, Box::pin(std::future::pending()));
        router.lock().unwrap().retire();
        session.take_requests();
        assert!(session.captures.is_empty());
        assert!(
            matches!(reply.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::Retired)
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn browser_capture_survives_an_event_loop_turn() {
        let mut session = SessionIngress::detached_for_test();
        let reply = queued_screenshot(&mut session);
        let shot = session.take_pending_screenshots(0).pop().unwrap();
        session.submit_screenshot(
            shot,
            Box::pin(async {
                gloo_timers::future::TimeoutFuture::new(1).await;
                Ok((1, 1, vec![1, 2, 3, 255]))
            }),
        );
        session.poll_captures();
        assert!(reply.try_recv().is_err());
        for _ in 0..100 {
            gloo_timers::future::TimeoutFuture::new(1).await;
            session.poll_captures();
            if session.captures.is_empty() {
                break;
            }
        }
        assert!(matches!(
            reply.try_recv().unwrap(),
            Ok(ReplyBody::Binary {
                content_type: "image/png",
                ..
            })
        ));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn disconnected_callers_cannot_admit_queued_or_deferred_mutations() {
        let router = router();
        let mut session = SessionIngress::with_router(Some(router.clone()));
        let (raw, raw_reply) = request(HttpPayload::Console("cheat".into()));
        router.lock().unwrap().push_back(raw);
        drop(raw_reply);
        assert!(session.take_requests().is_empty());
        let (step, reply) = request(HttpPayload::SetPaused { paused: true });
        session.defer(step, true);
        drop(reply);
        assert!(session.take_pending_steps().is_empty());
        let (shot, reply) = request(HttpPayload::Screenshot(ScreenshotRequest {
            frame: Some(900),
            ..Default::default()
        }));
        session.defer(shot, true);
        drop(reply);
        assert!(session.take_pending_screenshots(900).is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn expiration_is_checked_at_dispatch_and_deferred_admission() {
        let router = router();
        let mut session = SessionIngress::with_router(Some(router.clone()));
        let (raw, reply) = request(HttpPayload::Console("cheat".into()));
        router.lock().unwrap().push_back(raw);
        // Model the interval between inbox extraction and command execution.
        let raw = session.take_requests().pop().unwrap();
        reply.expire();
        assert!(!raw.admit_unless_deferred());
        let (mut step, reply) = request(HttpPayload::SetPaused { paused: true });
        step.response_tx = step.response_tx.with_deadline(std::time::Instant::now());
        session.defer(step, true);
        assert!(session.take_pending_steps().is_empty());
        assert!(reply.is_expired());
        let (shot, reply) = request(HttpPayload::Screenshot(ScreenshotRequest::default()));
        session.defer(shot, true);
        reply.expire();
        assert!(session.take_pending_screenshots(0).is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dropping_native_relay_future_cancels_its_queued_request() {
        use futures::FutureExt as _;
        let router = router();
        let mut session = SessionIngress::with_router(Some(router.clone()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert!(
                relay(&router, HttpPayload::Console("cheat".into()))
                    .now_or_never()
                    .is_none()
            );
        });
        assert!(session.take_requests().is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn timeout_does_not_revoke_an_admitted_operation() {
        let (request, reply) = request(HttpPayload::Console("cheat".into()));
        assert!(request.admit_unless_deferred());
        reply.expire();
        assert!(!reply.is_expired());
        request
            .response_tx
            .send(Ok(serde_json::json!({"ok": true}).into()));
        assert!(reply.try_recv().unwrap().is_ok());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn admitted_work_survives_disconnect_without_reopening_admission() {
        let (request, reply) = request(HttpPayload::Console("cheat".into()));
        assert!(request.admit_unless_deferred());
        drop(reply);
        assert!(!request.admit_unless_deferred());
        // Accepted work is not rolled back when its consumer disappears.
        request
            .response_tx
            .send(Ok(serde_json::json!({"ok": true}).into()));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn transport_stop_cancels_deferred_work_before_another_mission_tick() {
        let router = router();
        let mut ingress = SessionIngress::with_router(Some(router.clone()));
        let (step, step_reply) = request(HttpPayload::StepForward {
            request: StepRequest::default(),
        });
        let (shot, shot_reply) = request(HttpPayload::Screenshot(ScreenshotRequest {
            frame: Some(900),
            ..ScreenshotRequest::default()
        }));
        ingress.defer(step, true);
        ingress.defer(shot, true);
        router.lock().unwrap().retire();
        assert!(ingress.take_pending_steps().is_empty());
        assert!(ingress.take_pending_screenshots(900).is_empty());
        assert!(
            matches!(step_reply.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::Retired && error.message == "HTTP transport stopped")
        );
        assert!(
            matches!(shot_reply.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::Retired && error.message == "HTTP transport stopped")
        );
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
        old.defer(step, true);
        old.defer(shot, true);
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
            matches!(state_reply.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::UnavailableCapability && error.message.contains("engine not ready"))
        );
        let (replay, replay_reply) = request(HttpPayload::GetReplay);
        router.lock().unwrap().push_back(replay);
        let original = router.lock().unwrap().idle.front().unwrap() as *const HttpRequest;
        let mut session = SessionIngress::with_router(Some(router.clone()));
        assert!(router.lock().unwrap().idle.is_empty());
        assert!(std::ptr::eq(
            original,
            session.requests.lock().unwrap().front().unwrap()
        ));
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
    fn idle_batch_transfer_preserves_fifo_and_leaves_new_requests_queued() {
        let mut router = RequestRouter::default();
        let mut replies = Vec::new();
        for _ in 0..3 {
            let (request, reply) = request(HttpPayload::GetReplay);
            router.push_back(request);
            replies.push(reply);
        }
        let pending = router.take_idle();
        assert!(router.take_idle().is_empty());
        let (later, later_reply) = request(HttpPayload::GetReplay);
        router.push_back(later);
        for (index, request) in pending.into_iter().enumerate() {
            request
                .response_tx
                .send(Ok(serde_json::json!(index).into()));
        }
        for (index, reply) in replies.into_iter().enumerate() {
            let response = reply.try_recv().unwrap().unwrap();
            assert!(
                matches!(response, ReplyBody::Json(value) if value == serde_json::json!(index))
            );
        }
        assert!(later_reply.try_recv().is_err());
        let mut next = router.take_idle();
        assert_eq!(next.len(), 1);
        next.pop_front()
            .unwrap()
            .response_tx
            .send(Ok(serde_json::json!("later").into()));
        assert!(later_reply.try_recv().unwrap().is_ok());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn extracted_batch_filters_cancelled_requests_without_reordering_survivors() {
        let router = router();
        let mut session = SessionIngress::with_router(Some(router.clone()));
        let mut replies = Vec::new();
        for index in 0..3 {
            let (request, reply) = request(HttpPayload::GetReplay);
            router.lock().unwrap().push_back(request);
            if index == 1 {
                drop(reply);
            } else {
                replies.push(reply);
            }
        }
        let pending = session.take_requests();
        assert_eq!(pending.len(), 2);
        assert!(session.take_requests().is_empty());
        for (index, request) in pending.into_iter().enumerate() {
            request
                .response_tx
                .send(Ok(serde_json::json!(index).into()));
        }
        for (index, reply) in replies.into_iter().enumerate() {
            assert!(
                matches!(reply.try_recv().unwrap().unwrap(), ReplyBody::Json(value) if value == serde_json::json!(index))
            );
        }
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
                session.defer(request, graphical);
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
            shot.respond_err(RpcError::internal("captured UI"));
        }
        let scene = session.take_pending_scene_screenshots(10);
        assert_eq!(scene.len(), 1);
        assert_eq!(scene[0].request().width, Some(2));
        for shot in scene {
            shot.respond_err(RpcError::internal("captured scene"));
        }
        assert!(session.take_pending_screenshots(19).is_empty());
        let future = session.take_pending_screenshots(20);
        assert_eq!(future.len(), 1);
        assert_eq!(future[0].request().width, Some(1));
        for shot in future {
            shot.respond_err(RpcError::internal("captured future"));
        }
        assert!(
            replies
                .into_iter()
                .all(|reply| reply.try_recv().unwrap().is_err())
        );
        let (request, reply) = request(HttpPayload::Screenshot(ScreenshotRequest::default()));
        session.defer(request, false);
        assert!(
            matches!(reply.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::UnavailableCapability && error.message.contains("headless"))
        );
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
