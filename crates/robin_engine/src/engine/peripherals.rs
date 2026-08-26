use crate::ai::Noise;
use crate::console::Console;
use crate::element::EntityId;
use crate::engine::types::{BackgroundTransform, DisplayOpCode};
use crate::macro_store::{
    MacroStore, NUMBER_OF_QA_MEMORY, QuickActionSlot, SHIFT_FALL_PER_REFRESH, SHIFT_STEP,
};
use crate::minimap::MinimapState;
use crate::shadow_polygon::ViewParameters;

/// One punctual noise being animated by the `noise_display` cheat.
#[derive(Clone, Debug)]
pub struct DisplayedNoise {
    /// Captured snapshot of the broadcast noise.
    pub noise: Noise,
    /// Innermost concentric-ring radius this frame.  Grows by
    /// `CIRCLE_SPEED + 3 * CIRCLE_DISTANCE` per draw until it passes
    /// the effective volume, at which point the entry is removed.
    pub start_radius: u16,
}

/// Engine-owned display-state machine for the shared script/director
/// camera.
///
/// This intentionally contains only the fields the engine camera
/// transition code needs.  Host-local presentation state such as the
/// minimap and macro UI lives in [`HostDisplayState`] and must not feed
/// back into the rollback-safe engine tick.
#[derive(
    Clone,
    Debug,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CameraDisplayState {
    pub background_transform: BackgroundTransform,
    pub display_op: DisplayOpCode,
    pub frame_scrolled: [bool; 4],
}

impl CameraDisplayState {
    pub(crate) fn to_host_display_state(&self) -> HostDisplayState {
        HostDisplayState {
            background_transform: self.background_transform.clone(),
            display_op: self.display_op,
            frame_scrolled: self.frame_scrolled,
            ..HostDisplayState::default()
        }
    }

    pub(crate) fn update_from_host_display_state(&mut self, display: &HostDisplayState) {
        self.background_transform = display.background_transform.clone();
        self.display_op = display.display_op;
        self.frame_scrolled = display.frame_scrolled;
    }
}

/// Host-owned display-state machine scratch for local presentation and
/// UI widgets.
///
/// This is deliberately outside [`super::EngineInner`]: it drives
/// presentation details such as minimap transitions, local UI animation, and
/// per-frame input scratch.  Script/director camera transition state lives in
/// [`CameraDisplayState`] instead so rollback replay never depends on host
/// viewport scratch.
#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct HostDisplayState {
    pub background_transform: BackgroundTransform,
    pub display_op: DisplayOpCode,
    pub frame_scrolled: [bool; 4],
    pub minimap: MinimapState,
    pub macro_ui: MacroUiState,
}

impl HostDisplayState {
    /// Read-only access to the minimap state (position, transitions,
    /// highlights). Host renderers use this to query hit boxes and
    /// display state.
    pub fn minimap(&self) -> &MinimapState {
        &self.minimap
    }

    pub fn display_minimap(&mut self, show: bool, restore_position: bool) {
        self.minimap.display_map(show, restore_position);
    }

    /// Resolve the deterministic camera target of a minimap mouse-up while
    /// all host-owned geometry and drag state are available. The returned
    /// point is recorded on `PlayerCommand::MinimapMouseUp`; command apply
    /// never re-reads this presentation state to decide an Engine mutation.
    pub fn resolve_minimap_center(
        &self,
        click_pt: crate::coordinates::ScreenPoint,
        on_minimap: bool,
        level_size: crate::coordinates::MapSize,
    ) -> Option<crate::coordinates::MapPoint> {
        if self.minimap.dragged || !on_minimap || !self.minimap.is_displayed() {
            return None;
        }
        let usable = crate::minimap::usable_area(&self.minimap.map_box);
        usable
            .contains_point(click_pt)
            .then(|| self.minimap.map_to_real(click_pt, level_size))
            .flatten()
    }

    /// One-shot level-load wiring for the minimap corner button:
    /// geometry and pixel hit mask.
    pub fn setup_minimap_widget(
        &mut self,
        position: crate::coordinates::ScreenPoint,
        corner_size: crate::coordinates::ScreenSize,
        button_hit_mask: Option<crate::minimap::HitMask>,
        screen_width: f32,
        screen_height: f32,
    ) {
        self.minimap
            .set_widget_position(position, corner_size, screen_width, screen_height);
        if let Some(mask) = button_hit_mask {
            self.minimap.button_hit_mask = Some(mask);
        }
    }

    /// Level-load wiring for the deployed-map bitmap: hit mask, size,
    /// and initial bounding box derived from the saved profile position.
    pub fn setup_minimap_map(
        &mut self,
        hit_mask: crate::minimap::HitMask,
        map_size: crate::coordinates::MinimapSize,
        saved_position: crate::coordinates::ScreenPoint,
        screen_width: f32,
        screen_height: f32,
    ) {
        self.minimap.map_hit_mask = Some(hit_mask);
        self.minimap.map_size = map_size;
        self.minimap
            .set_minimap_position(saved_position, screen_width, screen_height);
        // Suppress the dirty flag from the seeding call — only user
        // drags / resizes should trigger a profile write-back.
        self.minimap.position_dirty = false;
    }

    pub(crate) fn tick_macro_shift_phases(&mut self, pc_ids: &[EntityId], macros: &MacroStore) {
        self.macro_ui.tick_shift_phases(pc_ids, macros);
    }

    pub(crate) fn rearm_macro_tetris(
        &mut self,
        pc_ids: &[EntityId],
        macros: &MacroStore,
        slot_idx: usize,
    ) {
        self.macro_ui.rearm_tetris(pc_ids, macros, slot_idx);
    }

    pub(crate) fn blink_qa(&mut self, pc_id: EntityId, slot_idx: usize) {
        self.macro_ui.blink_qa(pc_id, slot_idx);
    }

    pub(crate) fn tick_macro_blink_phases(&mut self, pc_ids: &[EntityId]) {
        self.macro_ui.tick_blink_phases(pc_ids);
    }

    pub fn macro_shift_phase(&self, pc_id: EntityId, slot_idx: usize) -> f32 {
        self.macro_ui.shift_phase(pc_id, slot_idx)
    }

    pub fn macro_titbit_blink_hidden(&self, pc_id: EntityId, slot_idx: usize) -> bool {
        self.macro_ui.is_blink_hidden(pc_id, slot_idx)
    }
}

#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct MacroUiState {
    entries: Vec<(EntityId, PcMacroUiState)>,
}

#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
struct PcMacroUiState {
    shift_phase: [f32; NUMBER_OF_QA_MEMORY],
    last_step_count: [usize; NUMBER_OF_QA_MEMORY],
    blink_phase_counter: [u16; NUMBER_OF_QA_MEMORY],
    blink_phase_timer: [u16; NUMBER_OF_QA_MEMORY],
}

impl MacroUiState {
    fn get(&self, pc: EntityId) -> Option<&PcMacroUiState> {
        self.entries
            .iter()
            .find(|(id, _)| *id == pc)
            .map(|(_, s)| s)
    }

    fn get_or_insert(&mut self, pc: EntityId) -> &mut PcMacroUiState {
        if let Some(idx) = self.entries.iter().position(|(id, _)| *id == pc) {
            return &mut self.entries[idx].1;
        }
        self.entries.push((pc, PcMacroUiState::default()));
        &mut self.entries.last_mut().unwrap().1
    }

    fn tick_shift_phases(&mut self, pc_ids: &[EntityId], macros: &MacroStore) {
        for &pc_id in pc_ids {
            let Some(state) = macros.get(pc_id) else {
                continue;
            };
            let ui = self.get_or_insert(pc_id);
            for slot_idx in 0..NUMBER_OF_QA_MEMORY {
                let cur_len = state.slot(slot_idx).map(QuickActionSlot::len).unwrap_or(0);
                if ui.last_step_count[slot_idx] != cur_len {
                    ui.last_step_count[slot_idx] = cur_len;
                    ui.shift_phase[slot_idx] = SHIFT_STEP;
                }
                ui.shift_phase[slot_idx] =
                    (ui.shift_phase[slot_idx] - SHIFT_FALL_PER_REFRESH).max(0.0);
            }
        }
    }

    fn rearm_tetris(&mut self, pc_ids: &[EntityId], macros: &MacroStore, slot_idx: usize) {
        if slot_idx >= NUMBER_OF_QA_MEMORY {
            return;
        }
        for &pc_id in pc_ids {
            let Some(state) = macros.get(pc_id) else {
                continue;
            };
            let ui = self.get_or_insert(pc_id);
            for i in slot_idx..NUMBER_OF_QA_MEMORY {
                ui.shift_phase[i] = SHIFT_STEP;
                ui.last_step_count[i] = state.slot(i).map(QuickActionSlot::len).unwrap_or(0);
            }
        }
    }

    fn shift_phase(&self, pc_id: EntityId, slot_idx: usize) -> f32 {
        self.get(pc_id)
            .and_then(|s| s.shift_phase.get(slot_idx).copied())
            .unwrap_or(0.0)
    }

    fn blink_qa(&mut self, pc_id: EntityId, slot_idx: usize) {
        if slot_idx >= NUMBER_OF_QA_MEMORY {
            return;
        }
        let ui = self.get_or_insert(pc_id);
        ui.blink_phase_counter[slot_idx] = crate::macro_store::BLINK_PHASE_INIT;
        ui.blink_phase_timer[slot_idx] = crate::macro_store::BLINK_PHASE_LENGTH;
    }

    fn tick_blink_phases(&mut self, pc_ids: &[EntityId]) {
        for &pc_id in pc_ids {
            let ui = self.get_or_insert(pc_id);
            for slot_idx in 0..NUMBER_OF_QA_MEMORY {
                if ui.blink_phase_counter[slot_idx] > 0 {
                    ui.blink_phase_timer[slot_idx] -= 1;
                    if ui.blink_phase_timer[slot_idx] == 0 {
                        ui.blink_phase_counter[slot_idx] -= 1;
                        ui.blink_phase_timer[slot_idx] = if ui.blink_phase_counter[slot_idx] > 0 {
                            crate::macro_store::BLINK_PHASE_LENGTH
                        } else {
                            0
                        };
                    }
                }
            }
        }
    }

    fn is_blink_hidden(&self, pc_id: EntityId, slot_idx: usize) -> bool {
        self.get(pc_id)
            .and_then(|s| s.blink_phase_counter.get(slot_idx).copied())
            .map(|c| c > 0 && (c & 1) != 0)
            .unwrap_or(false)
    }
}

/// Developer / debug / cheat state that does **not** belong in the
/// deterministic simulation snapshot.
///
/// None of these fields affect gameplay outcomes — they control debug
/// overlays, developer console state, and cheat triggers. Extracting them
/// from [`super::EngineInner`] keeps the sim-state struct clean: anything that
/// *does* live on `EngineInner` is authoritative simulation state that
/// participates in rollback clone + serde.
///
/// Owned by the game session and passed into engine methods that need it.
#[derive(Clone, Default)]
pub struct DevState {
    /// Debug visualization toggles.
    pub debug: DebugFlags,

    /// Developer/cheat console.
    pub console: Console,

    /// Whether at least one QA test has been launched.
    pub at_least_one_qa_launched: bool,

    /// Cheat: free-floating shadow polygon position (developer debug).
    pub cheat_free_shadow_polygon_pos: Option<crate::coordinates::WorldPoint3D>,
    /// Cheat: view parameters for the free shadow polygon.
    pub cheat_free_shadow_polygon_params: ViewParameters,

    /// If set, rain projectiles of this type next frame then clear.
    /// Uses a raw i32 matching RHobjectType; -1 or RHOBJECT_NONE = inactive.
    pub projectile_cheat_rain: i32,

    /// Last NPC sent on "vacation" by `CheatHonolulu`.  The reactivate
    /// arm of the same cheat reads *this* id, not the current
    /// view-selected NPC, so the sequence
    /// "select A → HONOLULU → select B → HONOLULU" reactivates A
    /// rather than B.  Dev-only cheat state — never serialized.
    pub last_actor_in_honolulu: Option<crate::element::EntityId>,

    /// Active punctual-noise circles for the `noise_display` cheat.
    /// Animates expanding rings around each broadcast noise until the
    /// radius exceeds the effective volume, then drops the entry.
    pub displayed_noises: Vec<DisplayedNoise>,

    /// Rolling start-radius shared by the *PC footstep* rings.
    /// Advances by `CIRCLE_SPEED` every frame and wraps at
    /// `CIRCLE_DISTANCE` so the PC-noise rings scroll outward smoothly.
    pub noise_display_start_radius: u16,
}

impl DevState {
    pub fn new() -> Self {
        Self {
            projectile_cheat_rain: -1,
            ..Default::default()
        }
    }

    /// Queue one broadcast noise for the expanding-ring overlay.
    ///
    /// No-op when the cheat flag is off.
    pub fn add_noise_to_display(&mut self, noise: Noise) {
        if !self.debug.noise_display {
            return;
        }
        self.displayed_noises.push(DisplayedNoise {
            noise,
            start_radius: 0,
        });
    }

    /// Advance the ring animation by one frame.
    ///
    /// Rings whose start radius has outgrown the
    /// (volume × hearing-factor) envelope are retired; the rest step
    /// forward by `CIRCLE_SPEED + 3 * CIRCLE_DISTANCE`.
    ///
    /// The PC-footstep `start_radius` scrolls independently and wraps
    /// at `CIRCLE_DISTANCE`.
    pub fn tick_noise_display(&mut self, hearing_factor: f32) {
        const CIRCLE_SPEED: u16 = 7;
        const CIRCLE_DISTANCE: u16 = 20;

        // PC footstep scroll — always advance even when nothing is
        // queued; the static counter is incremented once per pass.
        self.noise_display_start_radius =
            self.noise_display_start_radius.wrapping_add(CIRCLE_SPEED);
        if self.noise_display_start_radius >= CIRCLE_DISTANCE {
            self.noise_display_start_radius -= CIRCLE_DISTANCE;
        }

        self.displayed_noises.retain_mut(|d| {
            let effective = d.noise.volume as f32 * hearing_factor;
            if (d.start_radius as f32) > effective {
                return false;
            }
            d.start_radius = d
                .start_radius
                .saturating_add(CIRCLE_SPEED + 3 * CIRCLE_DISTANCE);
            true
        });
    }
}

/// Debug visualization toggles — all default to `false`.
#[derive(Clone, Default, Debug)]
pub struct DebugFlags {
    pub door_display: bool,
    pub motion_obstacles_display: bool,
    pub motion_graph_display: bool,
    pub motion_graph_display_index: u16,
    /// `SURFACE` cheat — outline every walkable `MotionArea` polygon and
    /// highlight the area the selected character is currently on, plus
    /// the committed path waypoints colored by surface.
    pub surface_display: bool,
    pub elevation_display: bool,
    pub actor_info_display: bool,
    pub noise_display: bool,
    pub sound_source_display: bool,
    pub all_obstacles_display: bool,
    pub projection_areas_display: bool,
    pub railroad_display: bool,
    pub prob_display: bool,
    pub company_number_display: bool,
    pub free_shadow_polygon: bool,
    pub shadow_polygon_sphere: bool,
    pub display_animation_lines: bool,
    pub display_light_zones: bool,
    pub combat_energy_display: bool,
    pub pc_sight: bool,
    pub script_zone_display: bool,
    pub display_seek_points: bool,
    pub all_view_cones: bool,
    pub all_dialogues: bool,
    pub all_debriefings: bool,
    pub all_popup_texts: bool,
    /// `FPS` cheat — display the per-frame FPS counter overlay.
    pub fps_display: bool,
    /// Draw each entity's `EntityId` number centered below its feet.
    /// Dev overlay driven by the `/screenshot?entity_ids` HTTP flag.
    pub entity_ids: bool,
    /// Draw sprite occlusion-mask debug geometry for masked/flying sprites.
    /// Rust-only dev overlay for renderer parity investigations.
    pub sprite_masks_display: bool,
}
