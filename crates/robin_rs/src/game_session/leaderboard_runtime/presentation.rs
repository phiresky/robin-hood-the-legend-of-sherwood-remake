//! Cooperative post-mission UI ownership. Admission and signature tasks are separate owners.

use super::*;

/// Cooperative presentation/background state installed only after the final
/// authoritative debrief decision has been recorded. The first poll therefore
/// happens one outer frame after that decision and cannot export a replay that
/// is missing its terminal record.
pub(in crate::game_session) struct MissionEndLeaderboardTaskState {
    phase: MissionEndLeaderboardTaskPhase,
    application_context: crate::host::ApplicationContext,
}

enum MissionEndLeaderboardTaskPhase {
    Preparing(MissionEndPreparation),
    Visible(MissionEndLeaderboardScreen),
    Finished,
}

pub(in crate::game_session) enum MissionEndLeaderboardTaskProgress {
    Pending,
    Finished,
    Detach(MissionEndLeaderboardController),
}

impl MissionEndLeaderboardTaskState {
    pub(in crate::game_session) fn new(
        preparation: MissionEndPreparation,
        application_context: crate::host::ApplicationContext,
    ) -> Self {
        Self {
            phase: MissionEndLeaderboardTaskPhase::Preparing(preparation),
            application_context,
        }
    }

    pub(in crate::game_session) fn owns_presentation(&self) -> bool {
        matches!(
            &self.phase,
            MissionEndLeaderboardTaskPhase::Preparing(_)
                | MissionEndLeaderboardTaskPhase::Visible(_)
        )
    }

    pub(in crate::game_session) fn tick(
        &mut self,
        window: &mut GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> MissionEndLeaderboardTaskProgress {
        match &mut self.phase {
            MissionEndLeaderboardTaskPhase::Preparing(preparation) => {
                let Some(result) = preparation.poll_bundle() else {
                    render_preparing(renderer, resources, cursor);
                    return MissionEndLeaderboardTaskProgress::Pending;
                };
                let bundle = match result {
                    Ok(bundle) => bundle,
                    Err(error) => {
                        tracing::warn!("mission-end leaderboards unavailable: {error}");
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    }
                };
                let preferences = preparation.preferences().clone();
                let api = match LeaderboardApi::from_preferences(&preferences) {
                    Ok(api) => api,
                    Err(error) => {
                        tracing::warn!("mission-end leaderboard endpoint unavailable: {error}");
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    }
                };
                let peer_co_signer = if preparation.ranked_multiplayer_port.as_ref().is_some_and(
                    |port| port.role() == crate::multiplayer::RankedMultiplayerRole::Client,
                ) {
                    let Some(port) = preparation.ranked_multiplayer_port.take() else {
                        unreachable!("ranked client port was present")
                    };
                    let RankedMissionAdmission::Signed(signed) = &preparation.admission else {
                        tracing::warn!(
                            "ranked client port reached mission end without retained signed admission"
                        );
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    };
                    match MultiplayerPeerCoSigner::new(
                        port,
                        signed,
                        preparation.mission_id.clone(),
                        preparation.starting_campaign_bytes.clone(),
                    ) {
                        Ok(peer) => {
                            let receipt_controller_public_key = peer
                                .campaign_controller_public_key
                                .filter(|controller| *controller == peer.local_public_key());
                            Some((
                                Box::new(peer) as Box<dyn MissionEndPeerCoSigner>,
                                receipt_controller_public_key,
                            ))
                        }
                        Err(error) => {
                            tracing::warn!("ranked peer co-signing unavailable: {error}");
                            self.phase = MissionEndLeaderboardTaskPhase::Finished;
                            return MissionEndLeaderboardTaskProgress::Finished;
                        }
                    }
                } else {
                    None
                };
                let authorizer: Box<dyn MissionEndSubmissionAuthorizer> = if bundle.multiplayer
                    && bundle.eligible_submission.is_some()
                {
                    let Some(port) = preparation.ranked_multiplayer_port.take() else {
                        tracing::error!(
                            "ranked multiplayer admission reached mission end without its authenticated authorization port"
                        );
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    };
                    match MultiplayerHostSubmissionAuthorizer::new(port) {
                        Ok(authorizer) => Box::new(authorizer),
                        Err(error) => {
                            tracing::error!("ranked multiplayer authorizer unavailable: {error}");
                            self.phase = MissionEndLeaderboardTaskPhase::Finished;
                            return MissionEndLeaderboardTaskProgress::Finished;
                        }
                    }
                } else {
                    Box::new(LocalMissionEndSubmissionAuthorizer)
                };
                let controller = if let Some((peer, receipt_controller_public_key)) = peer_co_signer
                {
                    MissionEndLeaderboardController::new_peer(
                        bundle,
                        preferences,
                        Box::new(HttpMissionEndLeaderboardBackend::new(api)),
                        peer,
                        receipt_controller_public_key,
                    )
                } else {
                    MissionEndLeaderboardController::new(
                        bundle,
                        preferences,
                        Box::new(HttpMissionEndLeaderboardBackend::new(api)),
                        authorizer,
                        Box::new(ActiveMissionReplayExporter),
                    )
                };
                let mut controller = match controller {
                    Ok(controller) => controller,
                    Err(error) => {
                        tracing::warn!("mission-end leaderboard setup failed: {error}");
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    }
                };
                if controller.is_visible() {
                    self.phase = MissionEndLeaderboardTaskPhase::Visible(
                        MissionEndLeaderboardScreen::new(controller, resources),
                    );
                    // Do not poll the freshly-created controller a second time
                    // in this host frame.
                    MissionEndLeaderboardTaskProgress::Pending
                } else {
                    controller
                        .apply_action(MissionEndLeaderboardAction::Close)
                        .unwrap_or_else(|error| {
                            panic!("validated hidden leaderboard could not close: {error}")
                        });
                    self.phase = MissionEndLeaderboardTaskPhase::Finished;
                    retire_or_detach(controller)
                }
            }
            MissionEndLeaderboardTaskPhase::Visible(screen) => {
                let event = screen.tick(window, renderer, resources, cursor);
                if let Err(error) = screen
                    .controller_mut()
                    .persist_queued_receipt_watch(&self.application_context)
                {
                    tracing::error!(
                        "queued leaderboard verification could not be handed to durable tracking: {error}"
                    );
                }
                if event != Some(MissionEndLeaderboardEvent::Closed) {
                    return MissionEndLeaderboardTaskProgress::Pending;
                }
                let MissionEndLeaderboardTaskPhase::Visible(screen) =
                    std::mem::replace(&mut self.phase, MissionEndLeaderboardTaskPhase::Finished)
                else {
                    unreachable!()
                };
                let controller = screen.into_controller();
                retire_or_detach(controller)
            }
            MissionEndLeaderboardTaskPhase::Finished => MissionEndLeaderboardTaskProgress::Finished,
        }
    }
}

fn retire_or_detach(
    controller: MissionEndLeaderboardController,
) -> MissionEndLeaderboardTaskProgress {
    if controller.can_retire_after_close() {
        MissionEndLeaderboardTaskProgress::Finished
    } else {
        MissionEndLeaderboardTaskProgress::Detach(controller)
    }
}

fn render_preparing(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<&ModalCursor<'_>>,
) {
    enter_modal_gpu_phase(renderer);
    dim_screen(renderer);
    if let Some(background) = resources.menu_bg[0] {
        draw_screen_background(renderer, &background);
    }
    if let Some(font) = resources.title_font_any() {
        let transform = MenuTransform::centered(
            i32::from(renderer.screen_width()),
            i32::from(renderer.screen_height()),
        );
        let text = "Loading verified leaderboards...";
        render_text_virt_font(
            renderer,
            font,
            transform,
            text,
            (crate::ingame_menu::layout::MENU_W - font.text_width(text)) / 2,
            220,
        );
        if let Some(cursor) = cursor {
            // The preparation page has no controls, but retaining the current
            // cursor avoids a visible jump when the board becomes ready.
            cursor.draw(
                renderer,
                transform,
                &crate::ingame_menu::widget_bridge::ModalInputState::default(),
            );
        }
    }
    renderer.present();
}
