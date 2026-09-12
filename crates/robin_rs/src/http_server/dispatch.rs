//! Mission-scoped query and command dispatch; transport routing stays outside.

use super::*;

impl SessionIngress {
    pub fn drain(
        &mut self,
        engine: &mut Engine,
        frontend: &mut crate::host::HostFrontend,
        local_seat: robin_engine::player_command::PlayerId,
        net: Option<&crate::multiplayer::NetChannels>,
        assets: &LevelAssets,
        post_commands: &mut FrameCommands,
    ) -> Vec<engine_api::ExternalAction> {
        let mut selected = frontend.selected_view_element();
        let mut external_actions = Vec::new();
        self.drain_with_capabilities(
            engine,
            assets,
            &mut selected,
            net,
            post_commands,
            &mut external_actions,
            DispatchCapabilities::Interactive {
                frontend,
                local_seat,
            },
        );
        external_actions
    }

    /// Drain requests for a headless tool that owns an [`Engine`] directly.
    ///
    /// This is the small counterpart to [`SessionIngress::drain`] used by deterministic
    /// replay/debug runners. Requests which need the renderer or live host UI are
    /// rejected, while engine inspection, script/native calls, player commands,
    /// and the pause/step queue remain available.
    pub fn drain_headless(
        &mut self,
        engine: &mut Engine,
        assets: &LevelAssets,
        selected_view_element: &mut Option<engine_element::EntityId>,
    ) -> FrameCommands {
        let mut commands = FrameCommands::new();
        let mut external_actions = Vec::new();
        self.drain_with_capabilities(
            engine,
            assets,
            selected_view_element,
            None,
            &mut commands,
            &mut external_actions,
            DispatchCapabilities::Headless,
        );
        commands
    }

    /// Admission, taint accounting and routing have one ordering for every
    /// runner. Only the named presentation/diagnostic capabilities differ.
    fn drain_with_capabilities(
        &mut self,
        engine: &mut Engine,
        assets: &LevelAssets,
        selected_view_element: &mut Option<engine_element::EntityId>,
        net: Option<&crate::multiplayer::NetChannels>,
        commands: &mut FrameCommands,
        external_actions: &mut Vec<engine_api::ExternalAction>,
        mut capabilities: DispatchCapabilities<'_>,
    ) {
        for req in self.take_requests() {
            if !req.admit_unless_deferred() {
                continue;
            }
            self.observe_ranked_input_taint(&req.payload);
            match req.payload.classify() {
                RoutedRequest::HostDebug => {
                    req.response_tx
                        .send(capabilities.host_debug(engine, assets));
                }
                RoutedRequest::Query(QueryRequest::EngineDump) => {
                    req.response_tx.send(capabilities.engine_dump(engine));
                }
                RoutedRequest::Query(query) => req.response_tx.send(dispatch_query(
                    query,
                    self.replay_status(),
                    engine,
                    assets,
                )),
                RoutedRequest::Deferred(request) => {
                    self.defer_request(request, req.response_tx, capabilities.has_presentation())
                }
                RoutedRequest::Process(request) => self.dispatch_process(request, req.response_tx),
                RoutedRequest::Command(command) => {
                    let reply = dispatch_command(
                        command,
                        engine,
                        assets,
                        selected_view_element,
                        net,
                        commands,
                        external_actions,
                    );
                    capabilities.publish_selection(*selected_view_element);
                    req.response_tx.send(reply);
                }
            }
        }
    }
}

/// Live capabilities are borrowed for one drain, never saved or reconstructed.
enum DispatchCapabilities<'a> {
    Interactive {
        frontend: &'a mut crate::host::HostFrontend,
        local_seat: robin_engine::player_command::PlayerId,
    },
    Headless,
}

impl DispatchCapabilities<'_> {
    fn has_presentation(&self) -> bool {
        matches!(self, Self::Interactive { .. })
    }

    fn engine_dump(&self, engine: &Engine) -> Reply {
        // Original-parity runners own a nonserializable RNG source. Their
        // established diagnostic policy removes it from a clone only; the
        // interactive endpoint deliberately retains its full-snapshot policy.
        let value = match self {
            Self::Headless => {
                engine_dump_json(&engine.diagnostic_snapshot_without_original_rng_replay())
            }
            Self::Interactive { .. } => engine_dump_json(engine),
        };
        value
            .map(ReplyBody::Json)
            .map_err(|error| RpcError::internal(format!("engine serialize: {error}")))
    }

    fn host_debug(&self, engine: &Engine, assets: &LevelAssets) -> Reply {
        match self {
            Self::Interactive {
                frontend,
                local_seat,
            } => Ok(snapshot_host_debug(engine, frontend, *local_seat, assets).into()),
            Self::Headless => Err(RpcError::unavailable_capability(
                "host-debug is unavailable in a headless runner",
            )),
        }
    }

    fn publish_selection(&mut self, selected: Option<engine_element::EntityId>) {
        if let Self::Interactive { frontend, .. } = self {
            frontend.set_selected_view_element(selected);
        }
    }
}

fn admit_external_actions(
    engine: &mut Engine,
    assets: &LevelAssets,
    actions: Vec<engine_api::ExternalAction>,
    journal: &mut Vec<engine_api::ExternalAction>,
) -> Result<Vec<engine_api::ExternalActionResult>, RpcError> {
    let output = engine
        .advance_frame(
            assets,
            engine_api::SimulationFrameInput::no_hourglass()
                .with_post_external_actions(actions.clone()),
        )
        .map_err(|error| {
            let message = format!("developer action frame admission failed: {error}");
            match error {
                engine_api::FrameAdvanceError::RankedSimulationSettingCommandRejected {
                    ..
                } => RpcError::invalid_request(message),
                engine_api::FrameAdvanceError::RankedSimulationConfigViolation { .. }
                | engine_api::FrameAdvanceError::SpellforgeMissionAborted { .. }
                | engine_api::FrameAdvanceError::DirectorCompletionRejected { .. }
                | engine_api::FrameAdvanceError::SoundBoundaryRejected { .. }
                | engine_api::FrameAdvanceError::RecordedDropAleRouteRejected { .. } => {
                    RpcError::internal(message)
                }
            }
        })?;
    journal.extend(actions);
    Ok(output.external_action_results)
}

fn dispatch_command(
    payload: CommandRequest,
    engine: &mut Engine,
    assets: &LevelAssets,
    selected_view_element: &mut Option<engine_element::EntityId>,
    net: Option<&crate::multiplayer::NetChannels>,
    frame_commands: &mut FrameCommands,
    external_actions: &mut Vec<engine_api::ExternalAction>,
) -> Reply {
    match payload {
        CommandRequest::Native { name, args, this } => {
            let results = admit_external_actions(
                engine,
                assets,
                vec![engine_api::ExternalAction::Native {
                    name,
                    args,
                    this_actor: this,
                }],
                external_actions,
            )?;
            match results.into_iter().next() {
                Some(engine_api::ExternalActionResult::Native(result)) => result
                    .map(|value| ReplyBody::Json(serde_json::json!({"return": value})))
                    .map_err(RpcError::invalid_request),
                _ => Err(RpcError::internal(
                    "native frame admission returned no native result",
                )),
            }
        }
        CommandRequest::Batch(calls) => {
            let actions = calls
                .into_iter()
                .map(|call| engine_api::ExternalAction::Native {
                    name: call.op,
                    args: call.args,
                    this_actor: call.this,
                })
                .collect();
            let results = admit_external_actions(engine, assets, actions, external_actions)?
                .into_iter()
                .map(|result| match result {
                    engine_api::ExternalActionResult::Native(Ok(value)) => {
                        serde_json::json!({"return": value})
                    }
                    engine_api::ExternalActionResult::Native(Err(error)) => {
                        serde_json::json!({"error": error})
                    }
                    _ => serde_json::json!({"error": "non-native batch result"}),
                })
                .collect::<Vec<_>>();
            Ok(ReplyBody::Json(serde_json::json!({"results": results})))
        }
        CommandRequest::Console(cmd) => {
            // HTTP forces the full developer parser, but it has no live
            // `DevState`. Presentation-only commands therefore stay outside
            // the authoritative journal and report that limitation.
            let Some(command) = robin_engine::console::parse_with_final(&cmd, false) else {
                return Ok(ReplyBody::Json(frame_console_response_to_json(
                    engine_api::FrameConsoleResponse::Unknown,
                )));
            };
            if command.is_host_only() {
                return Ok(ReplyBody::Json(frame_console_response_to_json(
                    engine_api::FrameConsoleResponse::NotImplemented(
                        "host-only console command over HTTP".to_owned(),
                    ),
                )));
            }
            let results = admit_external_actions(
                engine,
                assets,
                vec![engine_api::ExternalAction::ConsoleCommand {
                    command,
                    selected_view_element: *selected_view_element,
                }],
                external_actions,
            )?;
            match results.into_iter().next() {
                Some(engine_api::ExternalActionResult::ConsoleCommand {
                    response,
                    selected_view_element: selected,
                }) => {
                    *selected_view_element = selected;
                    Ok(ReplyBody::Json(frame_console_response_to_json(response)))
                }
                _ => Err(RpcError::internal(
                    "console frame admission returned no console result",
                )),
            }
        }
        CommandRequest::Player(cmd) => {
            // In multiplayer, route the command over the wire so every
            // peer applies it at the same `target_frame`.  The local
            // engine doesn't mutate here; the echo lands via
            // `drain_net_inputs` at `sim_frame + INPUT_DELAY_FRAMES`.
            if let Some(net) = net {
                net.send_input(cmd)
                    .map_err(RpcError::unavailable_capability)?;
            } else {
                frame_commands.push(cmd);
            }
            Ok(ReplyBody::Json(serde_json::json!({"ok": true})))
        }
    }
}

pub(super) fn dispatch_query(
    query: QueryRequest,
    replay: Option<ReplayStatus>,
    engine: &Engine,
    assets: &LevelAssets,
) -> Reply {
    match query {
        QueryRequest::State => Ok(ReplyBody::Json(snapshot_state(engine, replay))),
        QueryRequest::EngineDump => engine_dump_json(engine)
            .map(ReplyBody::Json)
            .map_err(|e| RpcError::internal(format!("engine serialize: {e}"))),
        QueryRequest::LevelAssets => level_assets_json(engine, assets)
            .map(ReplyBody::Json)
            .map_err(|e| RpcError::internal(format!("level assets serialize: {e}"))),
        QueryRequest::Script => Ok(ReplyBody::Json(snapshot_script(engine))),
        QueryRequest::Decompile { class } => {
            Ok(ReplyBody::Json(decompile_script(engine, class.as_deref())))
        }
    }
}

impl SessionIngress {
    fn dispatch_process(&self, request: ProcessRequest, response: Responder) {
        let Some((exports, launches)) = &self.replay_capabilities else {
            response.send(Err(RpcError::unavailable_capability(
                "replay transport capabilities were not attached",
            )));
            return;
        };
        match request {
            ProcessRequest::ExportReplay => start_replay_export(exports, response),
            ProcessRequest::LoadReplay { data, paused } => {
                response.send(decode_load_replay(launches, &data, paused))
            }
        }
    }
}

pub(super) fn start_replay_export(
    exports: &crate::replay_service::ReplayExports,
    response_tx: Responder,
) {
    response_tx.send(Ok(ReplyBody::ReplayExport(exports.export())));
}
