//! Rendering: draw, view cone overlay, background, selection box.

use super::*;
use crate::coordinates::{GroundPoint, MapPoint, MapVec, ScreenPoint, ScreenVec};
use crate::element::EntityId;
use crate::messenger::{Message, MessageType, SimpleMessage};
use crate::shadow_polygon::ViewParameters;
use std::collections::HashMap;

/// Tuple returned by `selected_view_cone_params`: (eye point, view
/// parameters, optional RGB tint for the darkening overlay).
pub type ViewConeParams = (GroundPoint, ViewParameters, Option<(u8, u8, u8)>);

/// Back-to-front entity render order plus the per-entity depth value
/// the sort used as its key.
///
/// Computed by [`EngineInner::compute_display_order`] as a pure function
/// of sim state (entity positions, FX polylines, carried/attached refs).
/// The host caches the result between tick and render / input hit-test;
/// it is *not* sim state and never participates in the rollback hash.
#[derive(Debug, Clone, Default)]
pub struct DrawOrder {
    /// Back-to-front entity IDs (first = furthest back, last = topmost).
    pub ids: Vec<EntityId>,
    /// Depth key used by the sort (`position.y`, or `ref.depth ± 0.001`
    /// for carried/attached entities). Consumed by the titbit Z-flush
    /// in the host render loop.
    pub depths: HashMap<EntityId, f32>,
}

impl DrawOrder {
    /// Depth for one entity, if present in the current ordering.
    pub fn depth(&self, id: EntityId) -> Option<f32> {
        self.depths.get(&id).copied()
    }
}

impl EngineInner {
    // ─── Rendering ───────────────────────────────────────────────

    pub(super) fn with_camera_display_state<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut CameraDisplayState) -> R,
    ) -> R {
        let mut display = std::mem::take(&mut self.feedback.cutscene_camera.display);
        let result = f(self, &mut display);
        self.feedback.cutscene_camera.display = display;
        result
    }

    /// Advance the script/director camera display state stored inside
    /// the engine snapshot.
    pub(super) fn tick_camera_display_state(&mut self) -> u32 {
        self.with_camera_display_state(|engine, display| engine.tick_display_state(display))
    }

    /// Apply an engine state request against the internal camera display
    /// state instead of the host's local presentation scratch.
    #[cfg(test)]
    pub(crate) fn change_state_with_camera_display(
        &mut self,
        seat: usize,
        request: EngineStateRequest,
    ) -> bool {
        self.with_camera_display_state(|engine, display| {
            engine.change_state(display, seat, request)
        })
    }

    //
    // The Rust GPU renderer re-composes the scene every frame, so there
    // is no offscreen cache to invalidate from the engine — host cache
    // bookkeeping (patch bake, window-regain-focus) is driven by
    // `SideEffects::invalidate_background`, and save-load
    // clears mid-zoom state via `EngineSnapshot::apply_to`.
    pub(super) fn tick_display_state(&mut self, display: &mut CameraDisplayState) -> u32 {
        // Director work is cinematic / script-driven; it targets the
        // single canonical cutscene camera and runs once per tick.
        self.perform_director_work(display);

        // Skip rendering in fast-forward mode (draw every 32nd frame).
        // This is a frame-level gate, not per-seat — applies to every
        // active seat's pipeline uniformly.
        if self.control.fast_forward && (self.control.frame_counter & 31) != 0 {
            return 1;
        }

        // ── Script-camera pipeline integration ───────────────────
        // Local player scroll/zoom is host-side viewport state.  The
        // remaining engine pipeline advances shared script/director camera
        // transitions and must evolve identically on every machine.
        let seat_count = self.players.seats.len();
        for seat_idx in 0..seat_count {
            if !self.players.seats[seat_idx].is_active(seat_idx) {
                continue;
            }
            self.tick_display_state_for_seat(display, seat_idx);
        }

        // ── Once-per-tick post-processing ────────────────────────

        // Update the deterministic sound listener from the shared
        // cutscene camera. Local viewport audio routing can be added
        // host-side when split-view playback needs it.
        self.update_sound_listener_position();

        0
    }

    /// Display-state tick for the shared cutscene camera pipeline.
    /// Advances `seat`'s display-op/background-transform bookkeeping
    /// and mutates `cutscene_camera`, then resets display_op for the
    /// next frame.
    fn tick_display_state_for_seat(&mut self, display: &mut CameraDisplayState, seat: usize) {
        // ── Scrolling deceleration ───────────────────────────────
        if display.display_op == DisplayOpCode::NoBackgroundMove
            || display.display_op == DisplayOpCode::Scroll
        {
            self.decelerate_scrolling(display);
        }

        // ── Dispatch on display opcode ───────────────────────────
        match display.display_op {
            DisplayOpCode::Redraw => {
                // Full background redraw. Background rendering and entity
                // rendering both happen externally in game_session
                // (`draw_background`, `sort_for_display` +
                // `render_entities_gpu` + `render_selection_outlines_gpu`),
                // so nothing to do here beyond snapshotting the camera.
                // A cache-surface workflow is a future perf optimisation.
                self.feedback.cutscene_camera.old_view_position =
                    self.feedback.cutscene_camera.view_position;
                self.feedback.cutscene_camera.old_zoom_factor =
                    self.feedback.cutscene_camera.zoom_factor;
            }
            DisplayOpCode::Scroll => {
                self.perform_check_scroll(display);
                let scroll = display.background_transform.scrolling_vector;
                self.set_view_position_for_seat(
                    seat,
                    MapPoint::new(
                        self.feedback.cutscene_camera.view_position.x + scroll.x,
                        self.feedback.cutscene_camera.view_position.y + scroll.y,
                    ),
                );
                // The full background is redrawn each frame externally;
                // no incremental cache scroll needed here.
                self.feedback.cutscene_camera.old_view_position =
                    self.feedback.cutscene_camera.view_position;
                self.feedback.cutscene_camera.old_zoom_factor =
                    self.feedback.cutscene_camera.zoom_factor;
                // Mouse position is updated by game_session after draw().
            }
            DisplayOpCode::InitZoom => {
                // Initialise a zoom transition.
                //
                // The renderer composes the whole scene live every frame,
                // so instead of snapshotting two offscreen surfaces (one
                // at the old zoom/view, one at the new) and cross-fading
                // them, we record the zoom/view endpoints on
                // `background_transform` and let `perform_zoom_step`
                // interpolate `cutscene_camera.zoom_factor` /
                // `cutscene_camera.view_position` between them. The host
                // re-runs `draw_background` + entity rendering at the
                // current interpolated zoom each frame.
                let screen = Self::director_camera_view_size();
                let screen_vec = ScreenVec::new(screen.x, screen.y - PANNEL_HEIGHT);
                let level_size = self.feedback.cutscene_camera.level_size;

                // Source state = whatever the camera is at right now.
                let view_from = self.feedback.cutscene_camera.view_position;
                let zoom_from = self.feedback.cutscene_camera.zoom_factor;

                // Non-mechanized zooms re-center on the mouse: compute
                // `mouse_bias = (screen_center - mouse_screen) / zoom`
                // and add it to the new view position so the pixel under
                // the mouse stays anchored. Host sets
                // `cutscene_camera.pending_zoom_mouse_screen` at
                // ZoomingUp/Down dispatch time; we consume-and-clear here.
                let mouse_bias = if self.feedback.cutscene_camera.mechanized_zoom {
                    MapVec::ZERO
                } else {
                    let mouse_screen = self
                        .feedback
                        .cutscene_camera
                        .pending_zoom_mouse_screen
                        .take();
                    mouse_screen
                        .map(|m| {
                            MapVec::new(
                                (screen_vec.x * 0.5 - m.x) / zoom_from,
                                (screen_vec.y * 0.5 - m.y) / zoom_from,
                            )
                        })
                        .unwrap_or(MapVec::ZERO)
                };

                if display.background_transform.zoom_to_up {
                    // Zoom IN: `current_zoom_level` was already incremented
                    // so `zoom_values[level]` is the new factor.
                    let zoom_level = display.background_transform.current_zoom_level;
                    let new_factor = display.background_transform.zoom_values[zoom_level as usize];

                    let view_to = MapPoint::new(
                        view_from.x + screen_vec.x / (2.0 * zoom_from)
                            - screen_vec.x / (2.0 * new_factor)
                            + mouse_bias.x,
                        view_from.y + screen_vec.y / (2.0 * zoom_from)
                            - screen_vec.y / (2.0 * new_factor)
                            + mouse_bias.y,
                    );

                    display.background_transform.center_zoom = mouse_bias;
                    display.background_transform.clipped_zoom = MapVec::ZERO;
                    display.background_transform.zoom_from = zoom_from;
                    display.background_transform.zoom_to = new_factor;
                    display.background_transform.view_from = view_from;
                    display.background_transform.view_to = view_to;

                    self.orders.messenger.send(Message::new(MessageType::Simple(
                        SimpleMessage::ZoomUpStart,
                    )));
                } else {
                    // Zoom OUT: `current_zoom_level` was already decremented.
                    let zoom_level = display.background_transform.current_zoom_level;
                    let new_factor = display.background_transform.zoom_values[zoom_level as usize];

                    let mut target = MapPoint::new(
                        view_from.x + screen_vec.x / (2.0 * zoom_from)
                            - screen_vec.x / (2.0 * new_factor)
                            + mouse_bias.x,
                        view_from.y + screen_vec.y / (2.0 * zoom_from)
                            - screen_vec.y / (2.0 * new_factor)
                            + mouse_bias.y,
                    );

                    display.background_transform.center_zoom = mouse_bias;
                    display.background_transform.clipped_zoom = MapVec::ZERO;

                    // Clamp target view within level bounds at the NEW zoom.
                    if target.x < 0.0 {
                        display.background_transform.clipped_zoom.x = target.x;
                        target.x = 0.0;
                    }
                    if target.x + screen_vec.x / new_factor >= level_size.x {
                        display.background_transform.clipped_zoom.x =
                            target.x + screen_vec.x / new_factor - level_size.x;
                        target.x = level_size.x - screen_vec.x / new_factor;
                    }
                    if target.y < 0.0 {
                        display.background_transform.clipped_zoom.y = target.y;
                        target.y = 0.0;
                    }
                    if target.y + screen_vec.y / new_factor >= level_size.y {
                        display.background_transform.clipped_zoom.y =
                            target.y + screen_vec.y / new_factor - level_size.y;
                        target.y = level_size.y - screen_vec.y / new_factor;
                    }

                    target.x = target.x.floor();
                    target.y = target.y.floor();

                    display.background_transform.zoom_from = zoom_from;
                    display.background_transform.zoom_to = new_factor;
                    display.background_transform.view_from = view_from;
                    display.background_transform.view_to = target;

                    self.orders.messenger.send(Message::new(MessageType::Simple(
                        SimpleMessage::ZoomDownStart,
                    )));
                }

                // Leave the camera at the source state; `perform_zoom_step`
                // will advance it one step toward the target.
                display.background_transform.zoom_count = 0;
                display.background_transform.number_of_zoom_steps = 8;
                self.feedback.cutscene_camera.zoom_init_done = true;

                // Fall through into the first zoom step.
                self.perform_zoom_step(display);
            }
            DisplayOpCode::InZoom => {
                self.perform_zoom_step(display);
            }
            DisplayOpCode::NoBackgroundMove => {
                // Background is fine — entity rendering is handled
                // externally by game_session (render_entities_gpu etc.).
            }
            DisplayOpCode::Nothing => {
                // Nothing to do
            }
        }

        // Multi-selection rubber band is drawn host-side (host owns
        // the drag-box state and the renderer); engine no longer
        // dispatches it from draw().  View-cone, rain, trajectory
        // preview, and debug overlays are rendered externally by
        // game_session — see the `draw_over` docstring above.  Sound
        // listener / per-frame scroll-edge resets / sound-tick all
        // happen once per frame in [`Self::tick_display_state`].

        // Reset display op for next frame.
        if display.display_op != DisplayOpCode::InZoom {
            display.display_op = DisplayOpCode::NoBackgroundMove;
        }
    }

    // ─── View cone overlay ──────────────────────────────────────

    /// Check whether an admitted `DIES IRAE` action would consume the
    /// host's alt-hover gesture. This query is intentionally read-only; the
    /// corresponding mutation is admitted through the frame transaction.
    pub fn can_ezekiel_instakill(&self, id: EntityId) -> bool {
        if !self.ai.global.ezekiel_2517 {
            return false;
        }
        let Some(entity) = self.get_entity(id) else {
            return false;
        };
        entity.is_human()
    }

    /// Apply one admitted `DIES IRAE` action.
    pub fn try_ezekiel_instakill(&mut self, id: EntityId) -> bool {
        if !self.can_ezekiel_instakill(id) {
            return false;
        }
        // damage=10000 is a one-shot kill.
        let seq = crate::sequence::Sequence::single_damage(id, 10000, 0);
        self.launch_sequence(seq);
        true
    }

    /// Set the locker-mode follow target.
    ///
    /// `None` clears the follow target and disables locker mode.
    /// Non-`None` is ignored if the target is dead or unconscious.
    /// When accepted, centers the camera on the target if it's off-screen
    /// and enables locker tracking.
    pub(crate) fn select_follow_element(&mut self, seat: usize, entity_id: Option<EntityId>) {
        match entity_id {
            None => {
                self.players.seats[seat].locker_active = false;
            }
            Some(id) => {
                let Some(entity) = self.get_entity(id) else {
                    return;
                };
                // Reject dead / unconscious humans.
                let dead = entity.is_dead();
                let unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
                if dead || unconscious {
                    return;
                }

                // Snapshot target position before borrowing self mutably.
                let pos = entity.element_data().position_map();
                let target_pt = pos;
                self.players.seats[seat].follow_element = Some(id);

                // Compute `position_screen = (target - view) * zoom`. The
                // off-screen gate is an inside-box check in *map* space,
                // with the bottom-right clamped to director view size -
                // (0, PANNEL_HEIGHT) so the panel strip is excluded.
                let compute_screen = |view: MapPoint, zoom: f32| -> ScreenPoint {
                    ScreenPoint::new((target_pt.x - view.x) * zoom, (target_pt.y - view.y) * zoom)
                };
                let view = self.feedback.cutscene_camera.view_position;
                let zoom = self.feedback.cutscene_camera.zoom_factor;
                let box_tl = view;
                let screen = Self::director_camera_view_size();
                let box_br = MapPoint::new(
                    view.x + screen.x / zoom,
                    view.y + (screen.y - PANNEL_HEIGHT) / zoom,
                );
                let inside = target_pt.x >= box_tl.x
                    && target_pt.y >= box_tl.y
                    && target_pt.x <= box_br.x
                    && target_pt.y <= box_br.y;
                let anchor = if !inside {
                    self.center_on_point(seat, target_pt);
                    compute_screen(
                        self.feedback.cutscene_camera.view_position,
                        self.feedback.cutscene_camera.zoom_factor,
                    )
                } else {
                    compute_screen(view, zoom)
                };

                self.feedback.cutscene_camera.position_saved = anchor;
                self.players.seats[seat].locker_active = true;
                self.feedback.cutscene_camera.displacement_counter = 0;
            }
        }
    }

    /// Whether a quick-action macro is currently being recorded.
    ///
    /// Cursor, click, and titbit paths branch on this to record a macro
    /// step instead of immediately dispatching an action.
    pub fn is_recording_macro(&self) -> bool {
        !self.players.qa_recording_for.is_empty()
    }

    /// Build the `ViewParameters` for the currently-selected view element
    /// plus the 2D viewer (eye) position and, for NPCs, the RGB tint that
    /// should be applied to the darkening overlay.
    ///
    /// Returns `None` when any guard short-circuits: no selection, entity
    /// missing, inactive, dead, inside a building, eyes closed/unconscious,
    /// or the 3D eye point below ground.
    pub fn selected_view_cone_params(
        &self,
        selected_view_element: Option<EntityId>,
    ) -> Option<ViewConeParams> {
        use crate::element::EyeStatus;
        use crate::shadow_polygon::{ALPHA_DAY, NORMAL_HALF_APERTURE, sector_to_direction};

        let id = selected_view_element?;
        let entity = self.get_entity(id)?;
        if !entity.is_active() || entity.is_dead() {
            return None;
        }

        // Eyes-closed / eyes-unconscious short-circuit.
        // Only NPCs track eye status — PCs always have functioning eyes.
        if let Some(npc) = entity.npc_data()
            && npc.eye_status.is_blind()
        {
            return None;
        }

        // Skip when inside a building. Uses the entity's cached sector
        // (set during door-pass transitions) to check for building
        // membership.
        let edata = entity.element_data();
        if self.entity_building_sector(edata.sector()).is_some() {
            return None;
        }

        // 3D eye point: require `eye.z >= 0`. Only Human actors have a
        // posture-dependent eye point; for non-humans we accept
        // `position.z >= 0` directly (the guard is mostly there to skip
        // dead/teleported characters, which `is_dead()` already covers
        // for humans — this is belt-and-braces for animals / objects).
        let eye = entity.compute_eyes_point(None).unwrap_or(edata.position());
        if eye.z < 0.0 {
            return None;
        }

        let viewer = GroundPoint::new(eye.x, eye.y);

        // View direction and half-aperture.  For NPCs, use the
        // computed values from `refresh_view` (head turning, stare,
        // drunk wobble, etc.).  For PCs, fall back to the body
        // direction and `NORMAL_HALF_APERTURE`.
        let (dir, half_aperture) = if let Some(npc) = entity.npc_data() {
            (npc.view_direction, npc.real_half_aperture)
        } else {
            (sector_to_direction(edata.direction()), NORMAL_HALF_APERTURE)
        };

        // Radius: for NPCs use the computed view_radius (includes
        // longrange factor, drunk, stare modifiers from refresh_view).
        // For PCs, fall back to the engine's standard radius.
        let radius = if let Some(npc) = entity.npc_data() {
            npc.view_radius as f32
        } else if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius as f32
        } else {
            crate::shadow_polygon::RADIUS_DAY
        };

        // Alpha: for NPCs use the live `view_alpha_start` (fades on
        // death/unconsciousness). For PCs, use the static day/night value.
        let alpha = entity
            .npc_data()
            .map(|n| (n.view_alpha_start as u8).min(ALPHA_DAY))
            .unwrap_or(ALPHA_DAY);

        // `lean_out` is read from the live posture for NPCs; the
        // PC variant always treats lean_out as false.
        let is_pc = entity.pc_data().is_some();
        let lean_out = !is_pc && matches!(edata.posture, crate::element::Posture::LeaningOut);
        let params = ViewParameters {
            direction: dir,
            half_aperture,
            radius,
            alpha,
            lean_out,
            viewer_z: eye.z,
            projection_plane: entity.position_iface().get_plane().copied(),
            projection_obstacle: entity.position_iface().get_obstacle(),
        };

        // NPC tint from alert level — PCs use the default black overlay.
        // For NPCs we use the alert-status colour tables (see
        // `alert_colors::npc_tint`).
        let tint = entity.ai_controller().map(|ai| {
            let npc = entity.npc_data();
            let max_suspect = npc
                .map(|n| n.maximal_detection_suspect.max(ai.sorrow_level).min(1000))
                .unwrap_or(0);
            // `view_alert_status` already carries the IsForcedAttentive
            // soldier override (Green music ⇒ Yellow view) baked in by
            // `set_alert_status_with_flags`.
            crate::alert_colors::npc_tint(
                ai.view_alert_status,
                npc.map(|n| n.eye_status).unwrap_or(EyeStatus::LookForward),
                radius,
                self.ai.standard_view_polygon_radius.max(1) as f32,
                max_suspect,
                ai.sorrow_level,
            )
        });

        Some((viewer, params, tint))
    }

    /// Build view cone params for every visible NPC (for `--view-cones` mode).
    ///
    /// Same filtering as `selected_view_cone_params` but iterates all NPCs
    /// instead of just the selected view element.
    pub fn all_npc_view_cone_params(&self) -> Vec<ViewConeParams> {
        use crate::shadow_polygon::ALPHA_DAY;

        let mut result = Vec::new();
        for (_, entity) in self.world.entities.npcs() {
            if !entity.is_active() || entity.is_dead() {
                continue;
            }
            let Some(npc) = entity.npc_data() else {
                continue;
            };
            if npc.eye_status.is_blind() {
                continue;
            }
            let edata = entity.element_data();
            if self.entity_building_sector(edata.sector()).is_some() {
                continue;
            }
            let eye = entity.compute_eyes_point(None).unwrap_or(edata.position());
            if eye.z < 0.0 {
                continue;
            }

            let viewer = GroundPoint::new(eye.x, eye.y);
            let radius = npc.view_radius as f32;
            let alpha = (npc.view_alpha_start as u8).min(ALPHA_DAY);

            let params = ViewParameters {
                direction: npc.view_direction,
                half_aperture: npc.real_half_aperture,
                radius,
                alpha,
                lean_out: matches!(edata.posture, crate::element::Posture::LeaningOut),
                viewer_z: eye.z,
                projection_plane: entity.position_iface().get_plane().copied(),
                projection_obstacle: entity.position_iface().get_obstacle(),
            };

            let tint = entity.ai_controller().map(|ai| {
                let max_suspect = npc.maximal_detection_suspect.max(ai.sorrow_level).min(1000);
                // `view_alert_status` carries the IsForcedAttentive
                // override (Green music ⇒ Yellow view), set by
                // `set_alert_status_with_flags`.
                crate::alert_colors::npc_tint(
                    ai.view_alert_status,
                    npc.eye_status,
                    radius,
                    self.ai.standard_view_polygon_radius.max(1) as f32,
                    max_suspect,
                    ai.sorrow_level,
                )
            });

            result.push((viewer, params, tint));
        }
        result
    }

    /// Emit the selected-NPC AI log to the `ai_log` `tracing` target.
    ///
    /// Runs once per frame when the AI attribute-display cheat is on and
    /// the currently-selected view element is an NPC. The original game's
    /// on-screen overlay is replaced by `tracing::trace!` lines; callers
    /// filter via `RUST_LOG=robin_engine::ai=trace` (or
    /// `RUST_LOG=robin_engine[{ai_log}]=trace`) to surface the overlay
    /// output in the terminal.
    pub fn display_ai_log_for_selected(&self, selected_view_element: Option<EntityId>) {
        if !self.ai.global.attribute_display {
            return;
        }
        let Some(id) = selected_view_element else {
            return;
        };
        let Some(entity) = self.get_entity(id) else {
            return;
        };
        if !entity.is_npc() {
            return;
        }
        let Some(ai) = entity.ai_controller() else {
            return;
        };
        tracing::trace!(
            target: "ai_log",
            "--- AI log for NPC {:?} (frame {}) ---",
            id,
            self.control.frame_counter,
        );
        ai.display_log(self.control.frame_counter);
    }

    // ─── Display order sorting ──────────────────────────────────

    /// Compute a back-to-front draw order from the current entity table.
    ///
    /// Pure function of sim state — returns a fresh [`DrawOrder`] rather
    /// than mutating engine state, so it can be called from host code
    /// (before render + before input hit-test) without participating in
    /// the rollback hash.
    ///
    /// The algorithm has three phases:
    /// 1. Compute each entity's depth from `position.y` (or from a
    ///    reference entity for carried/attached entities).
    /// 2. Classify entities into "animations" (FX with masking polylines)
    ///    and "non-animations" (everything else), sort each group.
    /// 3. Merge animations into the sorted non-animation list using the
    ///    `is_element_behind_polyline` test (a two-pass merge).
    pub fn compute_display_order(&self) -> DrawOrder {
        // ── Phase 1: Compute depth per entity ────────────────────
        //
        // Free entities use `position.y` as their depth. Carried/attached
        // entities use `ref.depth ± 0.001` so they sort right next to the
        // entity they're attached to (sign chosen by `behind_display_order_ref`).
        //
        // Two-pass: first fill position.y for every entity, then resolve
        // the ref offset for carried/attached ones (which need the base
        // values computed first).
        let mut depths: HashMap<EntityId, f32> = HashMap::with_capacity(self.world.entities.len());
        for (id, e) in self.world.entities.occupied() {
            depths.insert(id, e.element_data().position().y);
        }

        for (id, entity) in self.world.entities.occupied() {
            let sprite = &entity.element_data().sprite;
            let Some(ref_id) = sprite.display_order_ref else {
                continue;
            };
            let Some(&ref_depth) = depths.get(&ref_id) else {
                continue;
            };
            let offset = if sprite.behind_display_order_ref {
                -0.001
            } else {
                0.001
            };
            depths.insert(id, ref_depth + offset);
        }

        // ── Phase 2: Classify and sort ───────────────────────────
        //
        // Two buckets:
        //   "Animations"     = FX-base entities with a non-empty display
        //                      polyline (these need the merge pass).
        //   "Non-animations" = everything else.
        //
        // FX with elevation==0 are drawn as background separately and do
        // not appear here.

        let mut animations: Vec<EntityId> = Vec::new();
        let mut non_animations: Vec<EntityId> = Vec::new();

        for (id, entity) in self.world.entities.occupied() {
            // RHEngine::AddElement puts elevation-zero FX-base elements only
            // in marrayBackgroundAnimations. They render once before the
            // sorted element pass and must not be drawn again over patches.
            if entity.is_background_animation() {
                continue;
            }

            // Scrolls whose current status is neither Visible nor
            // Opened are filtered out entirely — Invisible / Taken
            // scrolls don't render, and dropping them from the draw
            // order here also hides them from input hit-testing that
            // reuses `draw_order.ids` (e.g. `find_focusable_entity`).
            if matches!(entity, crate::element::Entity::Scroll(_)) {
                use super::scroll_reveal::ScrollStatus;
                if !matches!(
                    self.scroll_status(id),
                    ScrollStatus::Visible | ScrollStatus::Opened
                ) {
                    continue;
                }
            }
            let polyline = entity.display_polyline();
            if entity.is_fx() && !polyline.is_empty() {
                animations.push(id);
            } else {
                non_animations.push(id);
            }
        }

        let entities = &self.world.entities;

        // Sort non-animations by (depth, creation order). EntityId is
        // a monotonic slot index that's never reused, so it doubles as
        // the creation-order tiebreak.
        non_animations.sort_by(|a, b| {
            let da = depths.get(a).copied().unwrap_or(f32::MAX);
            let db = depths.get(b).copied().unwrap_or(f32::MAX);
            compare_display_depth(da, db, *a, *b)
        });

        // Sort animations by the minimum Y of their display polyline.
        animations.sort_by(|&a, &b| {
            let ya = entities
                .get(a)
                .map(|e| y_min_polyline(e.display_polyline()))
                .unwrap_or(f32::MAX);
            let yb = entities
                .get(b)
                .map(|e| y_min_polyline(e.display_polyline()))
                .unwrap_or(f32::MAX);
            compare_display_depth(ya, yb, a, b)
        });

        // ── Phase 3: Merge animations into non-animations ────────
        //
        // For each animation (sorted by Y-min), extract all active
        // non-animation entities that are "behind" it (using the polyline
        // test), append them, then append the animation itself.
        // Remaining non-animations go at the end.

        let ids = if animations.is_empty() {
            non_animations
        } else {
            let mut merged = Vec::with_capacity(animations.len() + non_animations.len());
            let mut consumed = vec![false; non_animations.len()];

            for &anim_id in &animations {
                let anim_entity = match entities.get(anim_id) {
                    Some(e) => e,
                    None => continue,
                };
                if !anim_entity.is_active() {
                    continue;
                }
                let polyline = anim_entity.display_polyline();

                for (i, &na_id) in non_animations.iter().enumerate() {
                    if consumed[i] {
                        continue;
                    }
                    let na_entity = match entities.get(na_id) {
                        Some(e) => e,
                        None => continue,
                    };
                    if !na_entity.is_active() {
                        continue;
                    }
                    let behind = is_element_behind_polyline(
                        polyline,
                        anim_entity.element_data().position_map(),
                        na_entity.element_data().position_map(),
                    );
                    if behind {
                        merged.push(na_id);
                        consumed[i] = true;
                    }
                }

                merged.push(anim_id);
            }

            for (i, &na_id) in non_animations.iter().enumerate() {
                if !consumed[i] {
                    merged.push(na_id);
                }
            }

            merged
        };

        DrawOrder { ids, depths }
    }

    // ─── Minimap dot info ───────────────────────────────────────
    //
    // Gather every property the minimap dot classifier needs for a
    // single entity. VIP status requires the profile manager, so we
    // take `LevelAssets`. Returns `None` for missing entity slots.
    pub fn minimap_dot_info(
        &self,
        id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> Option<crate::minimap::ElementDotInfo> {
        use crate::element::{Entity, Posture};
        use crate::minimap::{Camp as MmCamp, CustomDot, ElementDotInfo};

        let entity = self.get_entity(id)?;
        let elem = entity.element_data();

        let is_civilian_vip =
            matches!(entity, Entity::Civilian(_)) && self.is_entity_vip(assets, entity);
        let is_soldier_vip =
            matches!(entity, Entity::Soldier(_)) && self.is_entity_vip(assets, entity);

        let camp = if entity.camp() == crate::element::Camp::Lacklandists {
            MmCamp::Lacklandists
        } else {
            MmCamp::Other
        };

        Some(ElementDotInfo {
            custom_dot: CustomDot::from_u16(elem.custom_minimap_dot),
            is_active: entity.is_active(),
            is_human: entity.is_human(),
            is_object: entity.is_object(),
            is_projectile: entity.is_projectile(),
            is_scroll: matches!(entity, Entity::Scroll(_)),
            is_pc: entity.is_pc(),
            is_soldier: entity.is_soldier(),
            is_civilian: entity.is_civilian(),
            is_civilian_vip,
            is_vip: is_soldier_vip,
            is_blipped: elem.blipped,
            is_dead: entity.is_dead(),
            is_unconscious: entity.human_data().is_some_and(|h| h.unconscious),
            posture_lying: elem.posture == Posture::Lying,
            camp,
        })
    }

    // ─── Minimap sort order ─────────────────────────────────────
    //
    // Used by the minimap refresh loop to draw low-priority dots
    // (animals, NPCs) before high-priority ones (PCs, objects) so that
    // important markers end up on top. Unlike an in-place sort of the
    // engine's entity array, this returns a fresh ordering and leaves
    // engine state untouched.
    //
    // Returns every live entity; callers filter on `is_active()` /
    // `custom_minimap_dot`.
    pub fn sort_for_minimap(&self) -> Vec<EntityId> {
        let mut ids: Vec<EntityId> = self.world.entities.occupied().map(|(id, _)| id).collect();

        ids.sort_by(|&a, &b| {
            let ea = self.world.entities[a]
                .as_ref()
                .expect("entity present in sort input");
            let eb = self.world.entities[b]
                .as_ref()
                .expect("entity present in sort input");

            let pa = crate::minimap::element_priority(ea.is_object(), ea.is_pc(), ea.is_soldier());
            let pb = crate::minimap::element_priority(eb.is_object(), eb.is_pc(), eb.is_soldier());

            pa.cmp(&pb)
                .then_with(|| {
                    // Tiebreak by Y depth: position.y for free entities,
                    // ref.position.y ± 0.001 for carried ones (so minimap
                    // dots of carried objects sit right next to their
                    // carrier). Computed inline instead of via a cached
                    // sprite field.
                    let depth_of = |e: &Entity| -> f32 {
                        let sprite = &e.element_data().sprite;
                        if let Some(ref_id) = sprite.display_order_ref
                            && let Some(ref_entity) = self.world.entities.get(ref_id)
                        {
                            let base = ref_entity.element_data().position().y;
                            if sprite.behind_display_order_ref {
                                base - 0.001
                            } else {
                                base + 0.001
                            }
                        } else {
                            e.element_data().position().y
                        }
                    };
                    let da = depth_of(ea);
                    let db = depth_of(eb);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                // Final tiebreak by creation order: EntityId is a
                // monotonic slot index that's never reused.
                .then_with(|| a.index().cmp(&b.index()))
        });

        ids
    }
}

// ─── Display order helpers ──────────────────────────────────────────

/// Minimum Y coordinate of a display polyline.
fn y_min_polyline(polyline: &[MapPoint]) -> f32 {
    polyline.iter().map(|p| p.y).fold(f32::MAX, f32::min)
}

/// Totalizes Original's display comparator only for a NaN depth.
///
/// `RHElement::operator<` compares the raw display-order floats and uses
/// creation order only when `==`. A NaN therefore compares neither before nor
/// after any finite value. That violates the ordering contract of both C++
/// `std::sort` and Rust slice sorting; Original's unchecked zero-length combat
/// step-back can nevertheless put precisely such an entity in the
/// presentation array. Keep every ordinary comparison byte-for-byte
/// equivalent, and place only the unordered NaN case after ordered depths.
/// Creation order preserves deterministic input order between NaNs and equal
/// finite depths.
///
/// TODO(parity): if presentation draw order becomes a recorded field, verify
/// the exact shipped `std::sort` placement for multiple simultaneous NaNs.
fn compare_display_depth(
    left: f32,
    right: f32,
    left_id: EntityId,
    right_id: EntityId,
) -> std::cmp::Ordering {
    match (left.is_nan(), right.is_nan()) {
        (false, false) => left
            .partial_cmp(&right)
            .expect("non-NaN display depths must be ordered")
            .then_with(|| left_id.index().cmp(&right_id.index())),
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        (true, true) => left_id.index().cmp(&right_id.index()),
    }
}

/// Test whether an element is "behind" an FX animation's display polyline.
///
/// The polyline divides the map into "in front" and "behind" regions.
/// Returns `true` if the element at `position_map` is behind (should be
/// drawn before) the FX animation.
///
/// The test works as follows:
/// - If the element is left of the polyline's leftmost point, compare Y
///   against that endpoint.
/// - If right of the rightmost point, compare Y against that endpoint.
/// - Otherwise, find the segment that brackets the element's X coordinate
///   and use the 2D cross product (determinant) to determine which side
///   of the segment the point is on.
fn is_element_behind_polyline(
    polyline: &[MapPoint],
    fx_position_map: MapPoint,
    position_map: MapPoint,
) -> bool {
    let n = polyline.len();
    if n == 0 {
        // Defensive fallback: an FX with an empty display polyline
        // shouldn't reach the animation merge pass, but if it does we
        // compare the other element's Y against this FX's map-position
        // Y so neighbouring elements south of the FX still draw behind it.
        return position_map.y < fx_position_map.y;
    }

    // Left of polyline: compare against first point's Y.
    if position_map.x < polyline[0].x {
        return position_map.y < polyline[0].y;
    }

    // Right of polyline: compare against last point's Y.
    if position_map.x > polyline[n - 1].x {
        return position_map.y < polyline[n - 1].y;
    }

    // Find the segment that brackets position_map.x: walk from index 1
    // until polyline[current].x >= element.x.
    let mut current = 1;
    while current < n && polyline[current].x < position_map.x {
        current += 1;
    }
    if current >= n {
        return position_map.y < polyline[n - 1].y;
    }

    // 2D cross product (determinant) test against the segment:
    //   (polyline[current] - polyline[current-1]).Det(element - polyline[current-1]) < 0
    let seg_x = polyline[current].x - polyline[current - 1].x;
    let seg_y = polyline[current].y - polyline[current - 1].y;
    let to_x = position_map.x - polyline[current - 1].x;
    let to_y = position_map.y - polyline[current - 1].y;
    // Det(a, b) = a.x * b.y - a.y * b.x
    let det = seg_x * to_y - seg_y * to_x;
    det < 0.0
}

#[cfg(test)]
mod display_order_tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ElementData, ElementFx, ElementKind, Entity, FxData};

    fn fx_entity(
        position: WorldPoint3D,
        display_polyline: Vec<MapPoint>,
        patch_index: Option<crate::patch::PatchIndex>,
    ) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::Fx,
            active: true,
            ..Default::default()
        };
        element.set_position(position);
        Entity::Fx(ElementFx {
            element,
            fx: FxData {
                display_polyline,
                patch_index,
                ..Default::default()
            },
        })
    }

    #[test]
    fn elevation_zero_fx_render_only_in_background_pass() {
        let mut engine = EngineInner::new();
        let background = engine.add_entity(fx_entity(
            WorldPoint3D::new(10.0, 20.0, 0.0),
            Vec::new(),
            None,
        ));
        let elevated = engine.add_entity(fx_entity(
            WorldPoint3D::new(10.0, 30.0, 10.0),
            Vec::new(),
            None,
        ));

        assert_eq!(engine.bg_animation_ids(), vec![background]);
        assert_eq!(engine.compute_display_order().ids, vec![elevated]);
    }

    #[test]
    fn elevated_interior_fx_sort_behind_closing_patch() {
        let mut engine = EngineInner::new();
        let interior_fire = engine.add_entity(fx_entity(
            WorldPoint3D::new(100.0, 110.0, 10.0),
            Vec::new(),
            None,
        ));
        let closing_patch = engine.add_entity(fx_entity(
            WorldPoint3D::new(100.0, 210.0, 10.0),
            vec![MapPoint::new(0.0, 150.0), MapPoint::new(300.0, 150.0)],
            crate::patch::PatchIndex::new(0),
        ));

        assert_eq!(
            engine.compute_display_order().ids,
            vec![interior_fire, closing_patch]
        );
    }

    #[test]
    fn nan_display_depth_is_totalized_after_finite_entities() {
        let mut engine = EngineInner::new();
        let far = engine.add_entity(fx_entity(
            WorldPoint3D::new(10.0, 200.0, 10.0),
            Vec::new(),
            None,
        ));
        let unordered = engine.add_entity(fx_entity(
            WorldPoint3D::new(f32::NAN, f32::NAN, f32::NAN),
            Vec::new(),
            None,
        ));
        let near = engine.add_entity(fx_entity(
            WorldPoint3D::new(10.0, 100.0, 10.0),
            Vec::new(),
            None,
        ));

        assert_eq!(
            engine.compute_display_order().ids,
            vec![near, far, unordered]
        );
    }
}
