//! [`ParityReplaySetup`] capability methods: explicit parity-tool
//! reconstruction seams over an [`Engine`] (moved out of `rollback_safe.rs`,
//! review 01/F3).

use super::*;

impl ParityReplaySetup<'_> {
    /// Whether the original game's ordinary pre-update orientation pass
    /// would have emitted a resolved record for this PC/action pair.
    pub fn orientation_would_emit_before_hourglass(
        &self,
        actor: EntityId,
        action: crate::profiles::Action,
    ) -> bool {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::profiles::Action;

        let entity = self
            .engine
            .inner
            .get_entity(actor)
            .unwrap_or_else(|| panic!("orientation actor {actor:?} is missing"));
        let animation = self.engine.inner.live_actor_animation(actor);
        match action {
            Action::Bow => {
                matches!(
                    entity.actor_data().map(|actor| actor.action_state),
                    Some(ActionState::AimingWithBow | ActionState::AimingWithBowUp)
                ) && matches!(
                    animation,
                    Some(
                        OrderType::AimingWithBow
                            | OrderType::AimingWithBowUp
                            | OrderType::AimingWithBowAnonymous
                            | OrderType::AimingWithBowUpAnonymous
                    )
                ) && entity
                    .human_data()
                    .is_some_and(|human| human.pending_shoots.is_empty())
            }
            Action::Apple | Action::Stone | Action::Net | Action::WaspNest | Action::Purse => {
                !matches!(
                    animation,
                    Some(
                        OrderType::ThrowingPurse
                            | OrderType::ThrowingStone
                            | OrderType::ThrowingNet
                            | OrderType::ThrowingWaspNest
                            | OrderType::ThrowingApple
                    )
                )
            }
            Action::HelpToClimb => {
                let posture = entity.posture();
                let action_state = entity.actor_data().map(|actor| actor.action_state);
                if posture == Posture::CarryingOnShoulders {
                    matches!(
                        action_state,
                        Some(ActionState::Waiting | ActionState::Bored)
                    ) && !matches!(
                        animation,
                        Some(
                            OrderType::TransitionHelpingClimbingDown
                                | OrderType::TransitionHelpingClimbingUp
                        )
                    )
                } else {
                    !matches!(
                        posture,
                        Posture::CarryingOnShoulders | Posture::HelpingToClimb
                    )
                }
            }
            Action::Beggar => {
                entity.posture() == Posture::Upright
                    && matches!(
                        entity.actor_data().map(|actor| actor.action_state),
                        Some(ActionState::Waiting | ActionState::Bored)
                    )
            }
            other => {
                panic!("pre-update orientation ownership is not implemented for action {other:?}")
            }
        }
    }

    /// Number of retained scrolls in the replayed world.
    pub fn retained_scroll_count(&self) -> usize {
        self.engine.inner.world.entities.scrolls().count()
    }

    /// Number of falling-arrow RNG draws made by the pending ordinary
    /// presentation pass.
    ///
    /// Legacy parity traces do not retain every host refresh boundary.  The
    /// runner uses this count together with the retained original-game event positions
    /// to distinguish repeated refreshes from a single refresh containing
    /// several arrows.
    pub fn pending_falling_arrow_refresh_draw_count(&self) -> usize {
        if !self.engine.inner.control.arrow_refresh_pending {
            return 0;
        }
        self.engine
            .inner
            .compute_display_order()
            .ids
            .into_iter()
            .filter_map(|id| match self.engine.inner.world.entities.get(id) {
                Some(crate::element::Entity::Projectile(projectile))
                    if projectile.object.object_type == crate::element::ObjectType::Arrow =>
                {
                    Some(projectile)
                }
                _ => None,
            })
            .filter(|projectile| {
                projectile.element.active
                    && projectile.projectile.falling
                    && (!projectile.projectile.trajectory.is_empty()
                        || projectile.element.sprite.position_iface.old_position()
                            != projectile.element.sprite.position_iface.get_position())
            })
            .count()
    }

    /// Replay additional Original presentation passes omitted by a legacy
    /// trace, leaving the ordinary pending pass armed for `advance_frame`.
    ///
    /// Each pass uses normal arrow refresh so
    /// both tumble orientation and RNG consumption advance together.  The
    /// exact expected draw count is retained by the trace and an inconsistent
    /// reconstructed lifecycle fails loudly.
    pub fn replay_legacy_additional_arrow_refreshes(&mut self, expected_draws: usize) {
        let start = self
            .engine
            .inner
            .control
            .rng
            .original_replay_cursor()
            .expect("legacy arrow refresh replay requires Original RNG");
        let mut consumed = 0;
        while consumed < expected_draws {
            let before_pass = consumed;
            let sim = self.engine.inner.control.simulation_context();
            self.engine.inner.refresh_arrows_for_presentation(&sim);
            let cursor = self
                .engine
                .inner
                .control
                .rng
                .original_replay_cursor()
                .expect("legacy arrow refresh replay lost Original RNG");
            consumed = cursor - start;
            assert!(
                consumed <= expected_draws,
                "legacy arrow refresh reconstruction consumed {consumed} draws, expected {expected_draws}"
            );
            assert!(
                consumed > before_pass,
                "legacy arrow refresh reconstruction made no progress"
            );
        }
    }

    /// Consume a proven legacy burst of presentation-only sprite RNG draws.
    ///
    /// Schema 16 retained the values and callsites but omitted the host-side
    /// lifecycle event which caused these draws. The parity runner gates this
    /// narrowly from that evidence. No retained game state is changed here.
    pub fn consume_legacy_presentation_sprite_rng(&mut self, draw_count: usize) {
        let sim = self.engine.inner.control.simulation_context();
        for _ in 0..draw_count {
            let _ = crate::sim_rng::u32(
                &sim,
                crate::sim_rng::RngSite::ScrollInitialFrame,
                0..u32::MAX,
            );
        }
    }

    /// Cross the game's post-recording presentation boundary.
    ///
    /// Target-sprite creation overwrites the serialized width/height
    /// cache with the current bank frame while refreshing visible entities.
    /// Rust rendering is read-only, so the parity runner applies that legacy
    /// side effect explicitly after comparing each recorded frame.
    pub fn refresh_sprite_dimension_cache(
        &mut self,
        assets: &LevelAssets,
        legacy_missing_presentation_view: bool,
    ) {
        // Schema 16 does not record the engine view point, the draw-time
        // viewport origin used for visibility. It can differ from
        // the serialized simulation camera at the exact pixel where Original
        // decides whether target-sprite creation updates this cache. Leave that
        // unobservable presentation state untouched instead of inventing a
        // viewport. The parity comparator projects these cache fields out.
        // TODO: Once a trace schema records the draw viewport, replay that
        // exact value and compare the cache normally.
        if legacy_missing_presentation_view {
            return;
        }
        let Some(frames) = assets.attachments.pixel_opacity.as_ref() else {
            return;
        };
        let camera = &self.engine.inner.feedback.cutscene_camera;
        let screen = EngineInner::director_camera_view_size();
        let view = crate::sprite::BBox::from_coords(
            camera.view_position.x,
            camera.view_position.y,
            camera.view_position.x + screen.x / camera.zoom_factor,
            camera.view_position.y + (screen.y - crate::engine::PANNEL_HEIGHT) / camera.zoom_factor,
        );
        let updates = self
            .engine
            .inner
            .world
            .entities
            .occupied()
            .filter_map(|(id, entity)| {
                let element = entity.element_data();
                if !element.active || element.hidden_in_building {
                    return None;
                }
                // Scroll refresh only delegates to the ordinary object
                // renderer in these two states. Invisible and taken scrolls keep
                // their saved dimension cache even when their geometry intersects
                // the camera view.
                if matches!(entity, crate::element::Entity::Scroll(_))
                    && !matches!(
                        self.engine.inner.scroll_status(id),
                        crate::engine::ScrollStatus::Visible | crate::engine::ScrollStatus::Opened
                    )
                {
                    return None;
                }
                let sprite = entity.sprite();
                let row = sprite
                    .current_scripts_opt()
                    .and_then(|scripts| scripts.get(usize::from(sprite.current_row)))?;
                let &bank_id = row.frame_ids.get(usize::from(sprite.current_frame))?;
                let (width, height) = frames.sprite_dimensions(bank_id)?;
                // Screen visibility uses the current surface dimensions;
                // the serialized cache is not consulted until target-sprite creation
                // publishes those same dimensions. Using the stale cache here can
                // incorrectly cull a frame sitting on the viewport boundary.
                let sprite_position = entity.gameplay_sprite_position();
                let offset = sprite.current_offset();
                let sprite_box = crate::sprite::BBox::from_coords(
                    sprite_position.x + offset.x,
                    sprite_position.y + offset.y,
                    sprite_position.x + offset.x + f32::from(width),
                    sprite_position.y + offset.y + f32::from(height),
                );
                if !sprite_box.is_intersecting(&view) {
                    return None;
                }

                // Target-sprite creation resets masking before applying the current
                // grid mask list. Keep this serialized presentation cache in step
                // with the same mask query used by the renderer.
                let kind = entity.kind();
                let masked = if kind.has_valid_box_for_masking() {
                    let world_box = crate::coordinates::MapBBox::from_coords(
                        sprite_box.min.x,
                        sprite_box.min.y,
                        sprite_box.max.x,
                        sprite_box.max.y,
                    );
                    let is_flying_human = element.posture() == crate::element::Posture::Flying;
                    if is_flying_human || kind.is_projectile() {
                        !self
                            .engine
                            .inner
                            .fast_grid()
                            .get_masks_applied_to_projectile(
                                self.engine.inner.fast_grid().level.special_layer,
                                &world_box,
                                element.position(),
                                is_flying_human,
                                self.engine.inner.sight_obstacles(assets),
                            )
                            .is_empty()
                    } else {
                        !self
                            .engine
                            .inner
                            .fast_grid()
                            .get_masks_applied_to_character(
                                element.layer(),
                                &world_box,
                                element.position_map(),
                            )
                            .is_empty()
                    }
                } else {
                    false
                };
                Some((id, width, height, masked))
            })
            .collect::<Vec<_>>();

        for (id, width, height, masked) in updates {
            let sprite = self
                .engine
                .inner
                .world
                .entities
                .get_mut(id)
                .expect("presentation refresh entity disappeared");
            let sprite = sprite.sprite_mut();
            sprite.current_width = width;
            sprite.current_height = height;
            sprite.masked = masked;
        }
    }

    /// Advance a frame whose commands were independently recorded by the
    /// Original parity tracer.
    ///
    /// Nested-selection recording retains raw-mouse depth-2
    /// messages while omitting the depth-3 restitution emitted by `SelectPc`.
    /// Consequently adjacent recorded commands are siblings, not a root and
    /// its nested callback. Keeping this policy behind the explicit parity
    /// capability prevents it from changing live or rollback semantics.
    pub fn advance_frame(
        &mut self,
        assets: &LevelAssets,
        frame: SimulationFrameInput,
    ) -> Result<SimulationFrameOutput, FrameAdvanceError> {
        self.engine.advance_frame_with_command_batch_mode(
            assets,
            frame,
            SelectionCommandBatchMode::IndependentRecordedMessages,
            crate::replay::state_hash,
        )
    }

    #[doc(hidden)]
    pub fn has_pending_recorded_drop_ale_route(
        &self,
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
    ) -> bool {
        self.engine
            .has_pending_recorded_drop_ale_route(actor, destination)
    }

    #[doc(hidden)]
    pub fn restore_npc_maximal_visibility(&mut self, id: EntityId, value: u16) {
        self.engine.restore_parity_npc_maximal_visibility(id, value);
    }

    #[doc(hidden)]
    pub fn restore_npc_dormant_macro_cursor(
        &mut self,
        id: EntityId,
        path_id: crate::ai::PathId,
        waypoint_index: u8,
        offset: usize,
        assets: &LevelAssets,
    ) -> bool {
        self.engine.restore_parity_npc_dormant_macro_cursor(
            id,
            path_id,
            waypoint_index,
            offset,
            assets,
        )
    }

    pub fn append_rng_draws(&mut self, draws: Vec<u32>) {
        self.engine.append_original_rng_replay(draws);
    }

    pub fn replace_rng_draws(&mut self, draws: Vec<u32>) {
        self.engine.replace_original_rng_replay(draws);
    }

    pub fn set_impossible_action_done_deadlines(
        &mut self,
        deadlines: impl IntoIterator<Item = (u32, u32, i16)>,
    ) {
        self.engine
            .set_original_impossible_action_done_deadlines(deadlines);
    }

    pub fn use_external_director_completions(&mut self, enabled: bool) {
        self.engine.set_external_director_completion_replay(enabled);
    }
}
