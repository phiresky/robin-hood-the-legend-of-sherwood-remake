//! Transport, lifecycle and shared-dispatch tests for the script-RPC server.

use super::*;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use super::{dispatch::dispatch_query, transport::start};
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use std::{collections::BTreeSet, time::Duration};

#[cfg(all(test, not(feature = "script-rpc"), not(target_arch = "wasm32")))]
mod disabled_transport_tests {
    use super::*;

    #[test]
    fn native_listener_requires_feature_but_disabled_ingress_remains_usable() {
        let replay = Arc::new(crate::replay_service::ReplayService::default());
        let mut transport = HttpTransport::default();
        assert!(
            transport
                .start(DEFAULT_PORT, replay.exports(), replay.launches())
                .is_err()
        );
        assert!(!transport.is_started());
        transport
            .start(0, replay.exports(), replay.launches())
            .unwrap();
        let _ingress = transport.attach();
        transport.stop();
        assert!(!transport.is_started());
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_transport_tests {
    use super::*;

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn stop_rejects_an_already_deferred_browser_promise_without_another_tick() {
        use futures::FutureExt as _;
        let replay = Arc::new(crate::replay_service::ReplayService::default());
        let mut transport = HttpTransport::default();
        transport
            .start(0, replay.exports(), replay.launches())
            .unwrap();
        let mut ingress = transport.attach();
        // Match the browser's plain JS object, not serde-wasm-bindgen's
        // default Map representation of serde_json::Value objects.
        let value =
            js_sys::JSON::parse(r#"{"method":"set-paused","params":{"paused":true}}"#).unwrap();
        let mut promise = Box::pin(wasm_rpc::rh_rpc(value));
        if let Some(reply) = promise.as_mut().now_or_never() {
            // wasm panic aborts without running Drop. Release the bridge before
            // reporting a bad fixture so one failure cannot contaminate tests.
            transport.stop();
            panic!("deferred RPC completed before mission dispatch: {reply:?}");
        }
        let request = ingress.take_requests().pop().expect("queued request");
        ingress.defer_request(
            DeferredRequest::Step(StepKind::SetPaused { paused: true }),
            request.response_tx,
            true,
        );
        transport.stop();
        assert_eq!(
            promise.await.unwrap_err().as_string().as_deref(),
            Some("HTTP transport stopped")
        );
        assert!(ingress.take_pending_steps().is_empty());
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn early_requests_survive_owner_transfer_and_stop_allows_rebinding() {
        let replay = Arc::new(crate::replay_service::ReplayService::default());
        let mut early = HttpTransport::default();
        early.start(0, replay.exports(), replay.launches()).unwrap();
        let queue = BROWSER_QUEUE
            .with(|binding| binding.borrow().upgrade())
            .unwrap();
        let (response_tx, _rx) = Responder::channel();
        queue.lock().unwrap().push_back(HttpRequest {
            payload: HttpPayload::LoadReplay {
                data: b"early replay".to_vec(),
                paused: true,
            },
            response_tx,
        });
        let mut application = early;
        // Native port options are irrelevant to the browser bridge binding.
        application
            .start(DEFAULT_PORT, replay.exports(), replay.launches())
            .unwrap();
        let mut mission = application.attach();
        assert_eq!(mission.take_requests().len(), 1);
        let mut replacement = HttpTransport::default();
        assert!(
            replacement
                .start(0, replay.exports(), replay.launches())
                .is_err()
        );
        application.stop();
        // The old mission and a caller still retain the old queue, but neither
        // can prevent a new application from taking over the JS entry point.
        replacement
            .start(0, replay.exports(), replay.launches())
            .unwrap();
        let current = BROWSER_QUEUE
            .with(|binding| binding.borrow().upgrade())
            .unwrap();
        assert!(!Arc::ptr_eq(&queue, &current));
    }
}

#[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
mod transport_lifecycle_tests {
    use super::*;
    use std::io::Write;

    fn running() -> (HttpTransport, Arc<crate::replay_service::ReplayService>) {
        let replay = Arc::new(crate::replay_service::ReplayService::default());
        let server =
            start(0, replay.exports(), replay.launches()).expect("ephemeral HTTP listener");
        let port = server.bind_addr.port();
        (
            HttpTransport {
                port: Some(port),
                server: Some(server),
            },
            replay,
        )
    }

    #[test]
    fn socket_disconnect_cancels_queued_and_deferred_requests() {
        for deferred in [false, true] {
            let (mut transport, _) = running();
            let port = transport.port.unwrap();
            let mut ingress = transport.attach();
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let body = r#"{"paused":true}"#;
            write!(client, "POST /set-paused HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let request = loop {
                if let Some(request) = ingress.take_requests().pop() {
                    break request;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "socket request was not queued"
                );
                thread::yield_now();
            };
            let cancelled = request.response_tx.cancellation_observer();
            let queued = if deferred {
                ingress.defer_request(
                    DeferredRequest::Step(StepKind::SetPaused { paused: true }),
                    request.response_tx,
                    true,
                );
                None
            } else {
                Some(request)
            };
            client.shutdown(std::net::Shutdown::Both).unwrap();
            drop(client);
            // Observe transport cancellation itself, without driving a mission
            // tick or asking admission to notice a disconnected fixture.
            while !cancelled() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Hyper retained the disconnected caller's reply future"
                );
                thread::yield_now();
            }
            if let Some(request) = queued {
                assert!(!request.response_tx.admit());
            }
            assert!(ingress.take_pending_steps().is_empty());
            transport.stop();
        }
    }

    #[test]
    fn stop_prevents_admission_after_inbox_extraction() {
        let (mut transport, _) = running();
        let queue = transport.server.as_ref().unwrap().queue.clone();
        let mut ingress = transport.attach();
        let (response_tx, _reply) = Responder::channel();
        queue.lock().unwrap().push_back(HttpRequest {
            payload: HttpPayload::Console("cheat".into()),
            response_tx: response_tx.with_router(&queue),
        });
        let request = ingress.take_requests().pop().unwrap();
        transport.stop();
        assert!(!request.admit_unless_deferred());
    }

    #[test]
    fn repeated_binding_checks_port_and_replay_authority_and_stop_releases_port() {
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        transport
            .start(port, replay.exports(), replay.launches())
            .unwrap();
        assert!(
            transport
                .start(0, replay.exports(), replay.launches())
                .is_err()
        );
        let other = Arc::new(crate::replay_service::ReplayService::default());
        assert!(
            transport
                .start(port, other.exports(), replay.launches())
                .is_err()
        );
        assert!(
            transport
                .start(port, replay.exports(), other.launches())
                .is_err()
        );
        transport.stop();
        assert!(!transport.is_started());
        transport
            .start(port, other.exports(), other.launches())
            .expect("rebind with new authority");
        assert!(transport.matches_replay(&other.exports(), &other.launches()));
    }

    #[test]
    fn stop_cancels_a_reply_already_deferred_by_the_mission() {
        let (mut transport, _) = running();
        let queue = transport.server.as_ref().unwrap().queue.clone();
        let mut ingress = transport.attach();
        let worker = thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(relay(&queue, HttpPayload::SetPaused { paused: true }))
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(request) = ingress.take_requests().pop() {
                // Keep the responder alive in the mission's deferred queue.
                ingress.defer_request(
                    DeferredRequest::Step(StepKind::SetPaused { paused: true }),
                    request.response_tx,
                    true,
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "request did not reach ingress"
            );
            thread::yield_now();
        }
        transport.stop();
        assert_eq!(worker.join().unwrap().0, 400);
    }

    #[test]
    fn repeated_start_reports_a_listener_that_has_exited() {
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        transport
            .server
            .as_mut()
            .unwrap()
            .stop
            .take()
            .unwrap()
            .send(())
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !transport
            .server
            .as_ref()
            .unwrap()
            .listener
            .as_ref()
            .unwrap()
            .is_finished()
        {
            assert!(
                std::time::Instant::now() < deadline,
                "listener did not exit"
            );
            thread::yield_now();
        }
        assert!(
            transport
                .start(port, replay.exports(), replay.launches())
                .unwrap_err()
                .contains("exited")
        );
        transport.stop();
        transport
            .start(port, replay.exports(), replay.launches())
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stop_rebinds_after_a_completed_http_connection() {
        use std::io::Read;
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            client,
            "GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        transport.stop();
        transport
            .start(port, replay.exports(), replay.launches())
            .expect("rebind despite TIME_WAIT");
    }

    #[test]
    fn shutdown_does_not_wait_forever_for_an_incomplete_request_body() {
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(client, "POST /console HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 100\r\nContent-Type: application/json\r\n\r\n{{").unwrap();
        // Allow the connection task to begin acquiring the incomplete body.
        thread::sleep(Duration::from_millis(50));
        let before = std::time::Instant::now();
        transport.stop();
        assert!(before.elapsed() < Duration::from_secs(2));
        transport
            .start(port, replay.exports(), replay.launches())
            .expect("listener released after partial body");
    }

    #[test]
    fn shutdown_cancels_a_continuously_trickled_body() {
        let (mut transport, _) = running();
        let port = transport.port.unwrap();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(
            client,
            "POST /console HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1000000\r\n\r\n{{"
        )
        .unwrap();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer_finished = finished.clone();
        let writer = thread::spawn(move || {
            while !writer_finished.load(std::sync::atomic::Ordering::Acquire) {
                if client.write_all(b" ").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        thread::sleep(Duration::from_millis(50));
        let before = std::time::Instant::now();
        transport.stop();
        let elapsed = before.elapsed();
        finished.store(true, std::sync::atomic::Ordering::Release);
        writer.join().unwrap();
        assert!(elapsed < Duration::from_secs(2));
    }

    #[test]
    fn native_transport_preserves_security_and_early_replay_header_rejection() {
        use std::io::Read;
        let (transport, _) = running();
        let port = transport.port.unwrap();
        let exchange = |request: String| {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            response
        };
        assert!(
            exchange(format!(
                "GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            ))
            .starts_with("HTTP/1.1 200")
        );
        assert!(exchange(format!("GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://example.com\r\nConnection: close\r\n\r\n")).starts_with("HTTP/1.1 403"));
        assert!(
            exchange(
                "GET /info HTTP/1.1\r\nHost: attacker.example\r\nConnection: close\r\n\r\n".into()
            )
            .starts_with("HTTP/1.1 403")
        );
        // No chunk/body follows: validation must reject from headers alone.
        let response = exchange(format!(
            "POST /load-replay HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
        ));
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(response.contains("Transfer-Encoding"));
    }

    #[test]
    fn native_query_validation_happens_before_mission_admission() {
        use std::io::Read;
        let (transport, _) = running();
        let port = transport.port.unwrap();
        let mut ingress = transport.attach();
        for path in [
            "/screenshot?frame=bad",
            "/screenshot?view_cones=maybe",
            "/screenshot?frame=1&frame=2",
            "/script/decompile?class=%FF",
        ] {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(
                client,
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
            assert!(response.contains("\"error\""));
            assert!(
                ingress.take_requests().is_empty(),
                "invalid query reached the mission"
            );
        }
        for path in [
            "/screenshot?view_cones&frame=12",
            "/script/decompile?class=Guard%20A%2BB",
        ] {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(
                client,
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let request = loop {
                if let Some(request) = ingress.take_requests().pop() {
                    break request;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "valid query was not queued"
                );
                thread::yield_now();
            };
            match &request.payload {
                HttpPayload::Screenshot(value) => {
                    assert_eq!(value.frame, Some(12));
                    assert_eq!(value.flags.view_cones, Some(true));
                }
                HttpPayload::Decompile { class } => assert_eq!(class.as_deref(), Some("Guard A+B")),
                _ => panic!("unexpected query payload"),
            }
            assert!(request.response_tx.admit());
            request
                .response_tx
                .send(Ok(serde_json::json!({"ok": true}).into()));
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        }
    }
}

#[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
mod dispatch_tests {
    use super::*;

    #[test]
    fn shared_dispatch_preserves_admission_commands_queries_and_capability_differences() {
        for graphical in [false, true] {
            let mut assets = LevelAssets::new();
            let mut engine = Engine::new_for_test(1024.0, 768.0, Default::default(), &mut assets)
                .expect("RPC fixture engine");
            let mut host = crate::host::Host::scratch(1024.0, 768.0);
            let mut ingress = SessionIngress::detached_for_test();
            // Cancellation must occur before taint accounting or mutation.
            drop(ingress.enqueue_for_test(HttpPayload::Console("UNBLIP".into())));
            let state = ingress.enqueue_for_test(HttpPayload::State);
            let command = ingress.enqueue_for_test(HttpPayload::Command(PlayerCommand::CrouchDown));
            let debug = ingress.enqueue_for_test(HttpPayload::HostDebug);
            let screenshot =
                ingress.enqueue_for_test(HttpPayload::Screenshot(ScreenshotRequest::default()));
            let step = ingress.enqueue_for_test(HttpPayload::StepForward {
                request: StepRequest::default(),
            });
            let process = ingress.enqueue_for_test(HttpPayload::GetReplay);
            let mut selected = None;
            let commands = if graphical {
                let mut commands = FrameCommands::new();
                let external = ingress.drain(
                    &mut engine,
                    &mut host.frontend,
                    robin_engine::player_command::PlayerId::HOST,
                    None,
                    &assets,
                    &mut commands,
                );
                assert!(external.is_empty());
                commands
            } else {
                ingress.drain_headless(&mut engine, &assets, &mut selected)
            };
            assert_eq!(commands.commands.len(), 1);
            assert_eq!(
                commands.commands[0].player_id,
                robin_engine::player_command::PlayerId::HOST
            );
            assert!(matches!(
                commands.commands[0].command,
                PlayerCommand::CrouchDown
            ));
            assert!(
                matches!(state.try_recv().unwrap(), Ok(ReplyBody::Json(value)) if value["frame"] == engine.frame_counter())
            );
            assert!(
                matches!(command.try_recv().unwrap(), Ok(ReplyBody::Json(value)) if value == serde_json::json!({"ok": true}))
            );
            let debug_reply = debug.try_recv().unwrap();
            if graphical {
                assert!(matches!(debug_reply, Ok(ReplyBody::Json(_))));
                assert!(
                    screenshot.try_recv().is_err(),
                    "graphical capture remains deferred"
                );
                assert_eq!(
                    ingress
                        .take_pending_screenshots(engine.frame_counter())
                        .len(),
                    1
                );
            } else {
                assert!(
                    matches!(debug_reply, Err(error) if error.kind == RpcErrorKind::UnavailableCapability && error.message == "host-debug is unavailable in a headless runner")
                );
                assert!(
                    matches!(screenshot.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::UnavailableCapability)
                );
            }
            assert!(step.try_recv().is_err());
            assert_eq!(ingress.take_pending_steps().len(), 1);
            assert!(
                matches!(process.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::UnavailableCapability)
            );
            assert_eq!(
                ingress.take_pending_replay_taints(),
                BTreeSet::from([
                    InputTaintKind::HttpPlayerCommand,
                    InputTaintKind::HttpSimulationStep,
                ])
            );
        }
    }

    #[test]
    fn headless_diagnostic_policy_omits_rng_without_mutating_live_engine() {
        for graphical in [false, true] {
            let mut assets = LevelAssets::new();
            let mut engine =
                Engine::new_for_test(1024.0, 768.0, Default::default(), &mut assets).unwrap();
            engine = Engine::new(engine_api::EngineArgs {
                campaign: engine.campaign().clone(),
                level: engine_api::LevelLoadArgs {
                    assets: &mut assets,
                    level_directory: "",
                    progress: &mut |_| {},
                    loaded: robin_engine::level_data::LoadedLevel::empty(),
                    bg_pixel_dims: (0.0, 0.0),
                },
                ground_mark_sprite: None,
                titbit_row_frame_counts: Vec::new(),
                rng_seed: 0,
                original_rng_replay: Some(vec![11, 22]),
                sim_config: engine_api::SimConfig {
                    script_enabled: false,
                    ..Default::default()
                },
            })
            .expect("original RNG diagnostic fixture");
            let rng_cursor = engine.original_rng_replay_cursor();
            assert!(rng_cursor.is_some());
            let mut ingress = SessionIngress::detached_for_test();
            let dump = ingress.enqueue_for_test(HttpPayload::EngineDump);
            if graphical {
                let mut host = crate::host::Host::scratch(1024.0, 768.0);
                ingress.drain(
                    &mut engine,
                    &mut host.frontend,
                    robin_engine::player_command::PlayerId::HOST,
                    None,
                    &assets,
                    &mut FrameCommands::new(),
                );
                assert!(
                    matches!(dump.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::Internal && error.message.starts_with("engine serialize:"))
                );
            } else {
                ingress.drain_headless(&mut engine, &assets, &mut None);
                assert!(matches!(dump.try_recv().unwrap(), Ok(ReplyBody::Json(_))));
            }
            assert_eq!(engine.original_rng_replay_cursor(), rng_cursor);
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn query_dispatch_has_only_read_authority() {
        // Function-pointer coercion is a compile-time capability proof: adding
        // mutable state, frontend input, or command sinks breaks this contract.
        let _: fn(QueryRequest, Option<ReplayStatus>, &Engine, &LevelAssets) -> Reply =
            dispatch_query;
        let _: fn(
            &Engine,
            &crate::host::HostFrontend,
            robin_engine::player_command::PlayerId,
            &LevelAssets,
        ) -> serde_json::Value = snapshot_host_debug;
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn transport_classification_separates_authority_without_losing_arguments() {
        for payload in [
            HttpPayload::State,
            HttpPayload::EngineDump,
            HttpPayload::LevelAssets,
            HttpPayload::Script,
        ] {
            assert!(matches!(payload.classify(), RoutedRequest::Query(_)));
        }
        assert!(matches!(
            HttpPayload::HostDebug.classify(),
            RoutedRequest::HostDebug
        ));
        assert!(
            matches!(HttpPayload::Decompile { class: Some("Mission".into()) }.classify(), RoutedRequest::Query(QueryRequest::Decompile { class: Some(class) }) if class == "Mission")
        );
        assert!(
            matches!(HttpPayload::Native { name: "test".into(), args: vec![1, -2], this: Some(3) }.classify(), RoutedRequest::Command(CommandRequest::Native { name, args, this: Some(3) }) if name == "test" && args == [1, -2])
        );
        assert!(
            matches!(HttpPayload::Console("UNBLIP".into()).classify(), RoutedRequest::Command(CommandRequest::Console(command)) if command == "UNBLIP")
        );
        assert!(matches!(
            HttpPayload::Command(PlayerCommand::CrouchDown).classify(),
            RoutedRequest::Command(CommandRequest::Player(PlayerCommand::CrouchDown))
        ));
        assert!(
            matches!(HttpPayload::Batch(vec![]).classify(), RoutedRequest::Command(CommandRequest::Batch(calls)) if calls.is_empty())
        );
        assert!(matches!(
            HttpPayload::GetReplay.classify(),
            RoutedRequest::Process(ProcessRequest::ExportReplay)
        ));
        assert!(
            matches!(HttpPayload::LoadReplay { data: b"encoded".to_vec(), paused: true }.classify(), RoutedRequest::Process(ProcessRequest::LoadReplay { data, paused: true }) if data == b"encoded")
        );
    }

    #[test]
    fn step_request_defaults_to_one_tick_and_auto_dismiss() {
        let request: StepRequest = serde_json::from_value(serde_json::json!({}))
            .expect("empty step request uses documented defaults");
        assert_eq!(request, StepRequest::default());
        assert_eq!(request.n, 1);
        assert!(request.modal_policy.auto_dismiss);
        assert!(!request.modal_policy.synchronized_multiplayer);
    }

    #[test]
    fn step_request_requires_explicit_multiplayer_synchronization() {
        let request: StepRequest = serde_json::from_value(serde_json::json!({
            "n": 2,
            "synchronized_multiplayer": true,
        }))
        .expect("explicit multiplayer step policy");
        assert!(request.modal_policy.synchronized_multiplayer);
    }

    #[test]
    fn step_request_decodes_typed_modal_outcomes() {
        let dismissal = HttpModalDismissal {
            kind: ModalKind::Dialog { dialog_id: 17 },
            result: DialogResult::Aborted,
        };
        let request: StepRequest = serde_json::from_value(serde_json::json!({
            "n": 4,
            "auto_dismiss": false,
            "dismissals": [serde_json::to_value(&dismissal).expect("dismissal JSON")],
        }))
        .expect("typed step request");
        assert_eq!(request.n, 4);
        assert!(!request.modal_policy.auto_dismiss);
        assert_eq!(request.modal_policy.dismissals, vec![dismissal]);
    }

    #[test]
    fn ranked_input_taints_cover_every_mutating_automation_lane() {
        let cases = vec![
            (
                HttpPayload::Command(PlayerCommand::CrouchDown),
                InputTaintKind::HttpPlayerCommand,
            ),
            (
                HttpPayload::StepForward {
                    request: StepRequest::default(),
                },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::StepBack {
                    request: StepRequest::default(),
                },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::GoToFrame {
                    target: 20,
                    modal_policy: StepModalPolicy::default(),
                },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::SetPaused { paused: true },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::Native {
                    name: "SetMoney".into(),
                    args: vec![100],
                    this: None,
                },
                InputTaintKind::HttpStateMutation,
            ),
            (
                HttpPayload::Batch(vec![NativeCall {
                    op: "SetMoney".into(),
                    args: vec![100],
                    this: None,
                }]),
                InputTaintKind::HttpStateMutation,
            ),
            (
                HttpPayload::Console("UNBLIP".into()),
                InputTaintKind::ConsoleCommand,
            ),
            (
                HttpPayload::LoadReplay {
                    data: b"binary-fixture".to_vec(),
                    paused: false,
                },
                InputTaintKind::ReplayPlayback,
            ),
        ];
        let mut ingress = SessionIngress::detached_for_test();
        for (payload, expected) in &cases {
            assert_eq!(ranked_input_taint(payload), Some(*expected));
            ingress.observe_ranked_input_taint(payload);
        }
        assert_eq!(
            ingress.take_pending_replay_taints(),
            BTreeSet::from([
                InputTaintKind::HttpPlayerCommand,
                InputTaintKind::HttpSimulationStep,
                InputTaintKind::HttpStateMutation,
                InputTaintKind::ConsoleCommand,
                InputTaintKind::ReplayPlayback,
            ]),
            "the exact ingress hook must retain every observed mutating lane"
        );
        assert_eq!(ranked_input_taint(&HttpPayload::State), None);
        assert_eq!(ranked_input_taint(&HttpPayload::GetReplay), None);
    }

    #[test]
    fn browser_guard_rejects_any_origin_header() {
        assert!(
            browser_rejection_reason(
                Some("https://evil.example"),
                None,
                Some("127.0.0.1:17640"),
                17640
            )
            .is_some()
        );
        // Even a "local-looking" Origin is rejected: no browser client exists.
        assert!(
            browser_rejection_reason(
                Some("http://127.0.0.1:17640"),
                None,
                Some("127.0.0.1:17640"),
                17640
            )
            .is_some()
        );
    }

    #[test]
    fn browser_guard_rejects_cross_site_fetch_metadata() {
        assert!(
            browser_rejection_reason(None, Some("cross-site"), Some("127.0.0.1:17640"), 17640)
                .is_some()
        );
        assert!(
            browser_rejection_reason(None, Some("same-site"), Some("127.0.0.1:17640"), 17640)
                .is_some()
        );
        assert!(
            browser_rejection_reason(None, Some("none"), Some("127.0.0.1:17640"), 17640).is_none()
        );
        assert!(
            browser_rejection_reason(None, Some("same-origin"), Some("127.0.0.1:17640"), 17640)
                .is_none()
        );
    }

    #[test]
    fn browser_guard_rejects_foreign_or_missing_host() {
        assert!(
            browser_rejection_reason(None, None, Some("attacker.example:17640"), 17640).is_some()
        );
        assert!(browser_rejection_reason(None, None, Some("127.0.0.1:9999"), 17640).is_some());
        assert!(browser_rejection_reason(None, None, None, 17640).is_some());
        // The curl default workflow keeps working.
        assert!(browser_rejection_reason(None, None, Some("127.0.0.1:17640"), 17640).is_none());
        assert!(browser_rejection_reason(None, None, Some("localhost:17640"), 17640).is_none());
        assert!(browser_rejection_reason(None, None, Some("LOCALHOST:17640"), 17640).is_none());
    }

    #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
    #[test]
    fn screenshot_query_parses_frame_and_full_map() {
        let req =
            crate::http_server::query::screenshot("frame=10&full_map=1&hide_ui=true&entity_ids=0")
                .unwrap();
        assert_eq!(req.frame, Some(10));
        assert!(req.full_map);
        assert!(req.hide_ui);
        assert_eq!(req.flags.entity_ids, Some(false));
    }
}
