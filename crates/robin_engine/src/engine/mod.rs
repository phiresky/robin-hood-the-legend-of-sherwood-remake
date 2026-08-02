//! Core game engine.
//!
//! This is the central game loop that drives everything: the state machine,
//! per-frame update tick (`perform_hourglass`), rendering dispatch (`draw`),
//! level initialization, camera/zoom control, and subsystem management.
//!
//! Entity/rendering calls are stubbed where systems are not yet ported —
//! this module captures the *architecture*: the data structures, control
//! flow, and state transitions.

mod ai;
mod ale;
mod animation;
pub(crate) mod anti_collision;
mod beggar;
mod camera;
mod combat;
mod commands;
mod console_dispatch;
mod corpse_intersection;
mod display_state;
pub use display_state::DrawOrder;
mod door_pass;
#[cfg(test)]
mod filter_ai_event_tests;
mod global_options;
pub mod input;
pub(crate) mod jump;
pub mod level_loading;
pub mod melee;
mod movement;
mod nets;
mod patch_effects;
pub mod peripherals;
mod posture_transitions;
mod purse;
mod refresh_seek;
mod reinforcement;
mod rollback_safe;
mod script;
mod scroll_reveal;
mod seat;
mod sector_motion;
mod selection;
#[cfg(test)]
mod send_message_tests;
mod sequence_runtime;
mod sequence_validity;
mod simulation_gate;
mod snapshot;
mod soldier_helpers;
mod special_motion;
pub(crate) mod state;
#[doc(hidden)]
pub use state::ScriptDomains;
pub mod target_interaction;
#[cfg(test)]
mod target_script_tests;
mod teleport;
#[cfg(test)]
mod tests;
mod tick;
mod titbit_sync;
mod transitions;
mod types;
mod wasp_nest;

pub(crate) use commands::command_action_distance_animation;
pub use commands::{coin_pickup_target, object_pickup_command};
pub use console_dispatch::ConsoleResponse;
pub use global_options::*;
pub(crate) use movement::{FailedPathRequest, PendingPathRequest, PendingPathRequestQueue};
pub(crate) use movement::{adapt_source_to_current_door, current_door_for_route_source};
pub use peripherals::{CameraDisplayState, DebugFlags, DevState, HostDisplayState};
pub use rollback_safe::{
    Engine, EngineArgs, GroundMarkSpriteData, LevelLoadArgs, MinimapWidgetSetup,
    SnapshotGridComponent, SnapshotRestoreError,
};
pub use scroll_reveal::{BeggarRemark, PendingScrollAmulet, ScrollStatus};
pub use seat::SeatState;
pub use selection::Stature;
pub use types::*;

use crate::ai::AiGlobalState;
use crate::element::{Entity, EntityId};
use crate::fast_find_grid::FastFindGrid;
use crate::markers::GroundMark;
use crate::messenger::{Message, MessageType, SimpleMessage};
use crate::mission_stat::MissionStat;
use crate::order::OrderType;
use crate::pathfinder::PathFinder;
use crate::profiles::MissionType;
use crate::short_briefings::ShortBriefings;
use simulation_gate::SimulationGateState;
use state::{
    AiRuntime, FeedbackRuntime, MissionDomain, OrderRuntime, PlayerRuntime, ScriptRuntime,
    SimulationControl, WorldState,
};

// ─── Constants ───────────────────────────────────────────────────────

/// Default scrolling start speed (pixels per frame).
const DEFAULT_SCROLLING_START: f32 = 6.0;
/// Scrolling acceleration factor.
const DEFAULT_SCROLLING_ACCELERATION: f32 = 1.05;
/// Maximum scrolling speed.
const DEFAULT_SCROLLING_LIMIT: f32 = 31.0;

/// Number of scrolling table entries.
const SCROLLING_TABLE_SIZE: usize = 32;

/// Square distance threshold for multi-selection.
pub const MULTI_SELECTION_THRESHOLD: f32 = 1600.0;
/// Group movement limits.
pub const GROUP_LIMIT_MAX: u16 = 70;
pub const GROUP_LIMIT_MIN: u16 = 10;

/// Camera slide speed in frames.
pub const CAMERA_COUNTER: u16 = 15;

/// Frame timing target: 40ms = 25fps.
pub const FRAME_TIME_MS: u32 = 40;
/// Slow-motion multiplier.
pub const SLOW_MOTION_FRAME_TIME_MS: u32 = 400;

/// Frames per game-second (scripts tick once per 25 frames).
const FRAMES_PER_SECOND: u32 = 25;

/// Victory condition check interval in game-seconds.
const VICTORY_CHECK_INTERVAL: u32 = 3;

/// Default forbid multiselect timer.
pub const DEFAULT_FORBID_MULTISELECT: u32 = 25;

/// Panel height in pixels (bottom UI bar).
pub const PANNEL_HEIGHT: f32 = 80.0;

/// Cost in ransom to pay a beggar for one scroll reveal.
pub const BEGGAR_SALARY: i32 = 50;

/// Number of zoom levels.
const ZOOM_LEVEL_COUNT: usize = 3;

/// The central game engine struct, passed explicitly rather than via a
/// global singleton.
///
/// Fields are grouped by subsystem and annotated with serialization status.
///
/// `Clone` is derived so rollback snapshots and the determinism test can
/// copy the whole world cheaply.
///
/// `Serialize`, `Deserialize`, and `StateHash` all follow the current nested
/// ownership layout. Multiplayer peers, rollback, and current-format replays
/// therefore observe the same deterministic state boundaries.
#[derive(Clone, robin_state_hash_derive::StateHash)]
pub struct EngineInner {
    /// Deterministic mission outcome, campaign, objective, and stats state.
    pub(crate) mission_domain: MissionDomain,

    /// Deterministic time, RNG, and global suspension/rate controls.
    pub(crate) control: SimulationControl,

    /// Deterministic global AI state and mission-configured vision defaults.
    pub(crate) ai: AiRuntime,

    /// Authoritative entities and the spatial state indexed alongside them.
    pub(crate) world: WorldState,

    /// Deterministic world-script domains borrowed by native dispatch.
    pub(crate) script_domains: state::ScriptDomains,

    /// Deterministic orders, sequences, timers, messages, and existing
    /// deferred-gameplay queues.
    pub(crate) orders: OrderRuntime,

    /// Deterministic mission VM state and script global variables.
    pub(crate) scripts: ScriptRuntime,

    // ── Ported subsystems (real Rust types) ─────────────────────
    /// Deterministic per-player selection, input-mode, and macro state.
    pub(crate) players: PlayerRuntime,

    /// Deterministic sound, marker, director-camera, and tick-output state.
    pub(crate) feedback: FeedbackRuntime,
    // (Deferred bg-blits live on `pending_side_effects.bg_blits` now;
    // load-once index tables live on `LevelAssets::{source_durations,
    // patch_entity_handles, scroll_entity_ids, all_soldier_entity_ids}`.)
}

/// Disjoint engine-owned state needed after the entity/coma phases of mission
/// teardown. The campaign remains borrowed in place from `MissionDomain` for
/// the entire update; this context owns nothing and needs no unwind repair.
struct QuitMissionContext<'a> {
    campaign: &'a mut crate::campaign::Campaign,
    mission_stat: &'a mut MissionStat,
    pending_side_effects: &'a mut SideEffects,
    frame_counter: u32,
}

impl QuitMissionContext<'_> {
    fn apply_won_updates(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &crate::profiles::ProfileManager,
        living: u32,
        dead: u32,
        tied_score: i32,
        difficulty: crate::player_profile::DifficultyLevel,
    ) {
        sync_mission_stats_to_campaign(self.mission_stat, self.campaign);

        self.add_campaign_value(crate::campaign::CampaignValue::Score, tied_score);

        let idx = self
            .campaign
            .current_mission_idx
            .expect("quit-mission updates: current mission disappeared");
        let mission_type = self.campaign.missions[idx].profile(profiles).mission_type;
        if mission_type != crate::profiles::MissionType::Ambush {
            self.add_campaign_value(crate::campaign::CampaignValue::Score, 1000);
        }

        // Original provenance: `original-code/RHengine.cpp:16381-16398`
        // applies difficulty to recruitment only after the score updates
        // above. The application resolves that difficulty into the command,
        // so replay and multiplayer execution cannot consult ambient state.
        let recruited = self
            .campaign
            .recruit_post_mission_peasants(sim, living, dead, difficulty, profiles);
        self.mission_stat.new_peasant_count = recruited;
        tracing::info!("Post-mission warcrime recruitment: {recruited} new peasants");

        self.campaign.consume_blazons_post_mission(profiles);
    }

    fn add_campaign_value(&mut self, name: crate::campaign::CampaignValue, amount: i32) {
        self.campaign.values[name] += amount;
        EngineInner::apply_value_add_side_effects(
            self.mission_stat,
            self.pending_side_effects,
            self.frame_counter,
            name,
            amount,
        );
    }
}

fn sync_mission_stats_to_campaign(
    mission_stat: &MissionStat,
    campaign: &mut crate::campaign::Campaign,
) {
    campaign.add_value(
        crate::campaign::CampaignValue::LivingSoldiers,
        mission_stat.living_soldier_count as i32,
    );
    campaign.add_value(
        crate::campaign::CampaignValue::DeadSoldiers,
        mission_stat
            .total_soldier_count
            .saturating_sub(mission_stat.living_soldier_count) as i32,
    );
}

/// Sample duration in sim frames (40 ms each), keyed by sound-source
/// sample id.  Populated host-side from the decoded WAV length in the
/// sound cache; consulted by [`EngineInner`] when an `Activate` /
/// `ResumeAll` dispatches to schedule a deterministic finish.
pub type SourceDurations = std::sync::Arc<std::collections::BTreeMap<u32, u32>>;

/// A queued persistent background decal update for an FX entity whose
/// patch just transitioned. `restore_only = true` removes the decal
/// without adding the current frame.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct PendingBgBlit {
    pub entity_id: EntityId,
    pub restore_only: bool,
    pub decal: Option<PendingBgBlitDecal>,
}

/// Exact sprite frame to keep as a persistent background decal.
///
/// The original `Patch::SwapBackground(true)` temporarily forces the patch
/// FX to the last transition frame, blits it to the map, then restores its
/// previous row/frame. The Rust engine computes that same transition-frame
/// result without mutating the live sprite; the hardware renderer consumes
/// the concrete frame id and destination later.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct PendingBgBlitDecal {
    pub bank_id: u32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub shadow_color: u16,
}

/// Build the typed stable ID for a known occupied entity-table slot.
pub(crate) fn entity_id_for_occupied_slot(index: u32, entity: &Entity) -> EntityId {
    EntityId::new(index, entity.entity_id_kind())
}

/// Resolve Original's actor `mpOrder` animation identity from the explicit
/// installed pointer mirror. A selected SequenceManager element is not a
/// substitute: selection and `Instruct`/`DoNextOrder` pointer publication are
/// observably separate boundaries in the Original.
fn resolve_actor_order_type(
    installed: Option<crate::element::InstalledActorOrder>,
) -> crate::order::OrderType {
    installed
        .map(|order| order.order_type)
        .unwrap_or(crate::order::OrderType::NonanimationEnd)
}

#[cfg(test)]
mod actor_order_type_tests {
    use super::resolve_actor_order_type;
    use crate::{element::InstalledActorOrder, order::OrderType};
    use std::num::NonZeroU32;

    #[test]
    fn installed_order_is_authoritative() {
        assert_eq!(
            resolve_actor_order_type(Some(InstalledActorOrder {
                order_id: NonZeroU32::new(7).unwrap(),
                order_type: OrderType::TransitionWaitingUprightBoredWaitingUpright,
            })),
            OrderType::TransitionWaitingUprightBoredWaitingUpright
        );
    }

    #[test]
    fn null_installed_pointer_exposes_original_nonanimation_sentinel() {
        assert_eq!(resolve_actor_order_type(None), OrderType::NonanimationEnd);
    }
}

impl EngineInner {
    /// Queue concrete speech sample resolutions produced by the logical sound
    /// manager after the preceding engine frame.
    #[doc(hidden)]
    pub fn queue_resolved_exclamations(
        &mut self,
        resolutions: Vec<crate::sound::ResolvedExclamation>,
    ) {
        assert!(
            self.feedback.sound_sim.resolved_exclamations.is_empty(),
            "resolved exclamations were not consumed before the next sound boundary"
        );
        self.feedback.sound_sim.resolved_exclamations = resolutions;
    }

    /// Cancel every logical callback for an actor when Original's
    /// `StopExclamation` removes its pending sound without calling
    /// `SoundIsFinished`.
    pub(super) fn cancel_exclamation_callbacks(&mut self, actor_id: u32) {
        let sound = &mut self.feedback.sound_sim;
        sound
            .pending_exclamations
            .retain(|pending| pending.actor_id != actor_id);
        sound
            .resolved_exclamations
            .retain(|resolved| resolved.actor_id != actor_id);
        sound
            .playing_exclamations
            .retain(|playing| playing.actor_id != actor_id);
        sound
            .finished_exclamations
            .retain(|(finished_actor, _)| *finished_actor != actor_id);
    }

    pub(crate) fn engine_locked(&self) -> bool {
        self.control.engine_locked()
    }

    pub(crate) fn set_engine_locked(&mut self, locked: bool) {
        self.control.set_engine_locked(locked);
    }

    pub(crate) fn actors_frozen(&self) -> bool {
        self.control.actors_frozen()
    }

    pub(crate) fn set_actors_frozen(&mut self, frozen: bool) {
        self.control.set_actors_frozen(frozen);
    }

    #[cfg(test)]
    pub(crate) fn fade_freeze_frames_remaining(&self) -> u32 {
        self.control.fade_freeze_frames_remaining()
    }

    pub(crate) fn set_fade_freeze_frames_remaining(&mut self, frames: u32) {
        self.control.set_fade_freeze_frames_remaining(frames);
    }

    pub(crate) fn consume_fade_freeze_frame(&mut self) -> bool {
        self.control.consume_fade_freeze_frame()
    }

    pub(crate) fn pc_description_index_for_pc_data(
        &self,
        pc_data: &crate::element::PcData,
    ) -> Option<usize> {
        let campaign = &self.mission_domain.campaign;
        let Some(raw_index) = pc_data.campaign_description_index else {
            tracing::warn!(
                "PC profile {} has no campaign description identity",
                pc_data.profile_index
            );
            return None;
        };
        let idx = raw_index as usize;
        let Some(description) = campaign.characters.get(idx) else {
            tracing::error!(
                "PC campaign description index {raw_index} is outside campaign character table of length {}",
                campaign.characters.len()
            );
            return None;
        };
        if description.character_profile_idx != Some(pc_data.profile_index) {
            tracing::error!(
                "PC campaign description index {raw_index} has profile {:?}, entity has profile {}",
                description.character_profile_idx,
                pc_data.profile_index
            );
            return None;
        }
        // Original RHElementActorPC keeps mpDescription/mpStatus as aliases
        // into RHCampaign and serializes that description pointer separately.
        // mubListIndex is independent actor/UI storage and is never used to
        // resolve the campaign status. Profiles are not unique in the
        // campaign character table, so retaining this exact index is required.
        Some(idx)
    }

    pub(crate) fn pc_description_for_pc_data(
        &self,
        pc_data: &crate::element::PcData,
    ) -> Option<&crate::campaign::PcDescription> {
        let idx = self.pc_description_index_for_pc_data(pc_data)?;
        self.mission_domain.campaign.characters.get(idx)
    }

    pub(crate) fn attach_preflighted_level_assets(&mut self, assets: &LevelAssets) {
        self.world.attach_preflighted_level_assets(assets);
        self.scripts.attach_preflighted_level_assets(assets);
    }

    /// Test fixture constructor. Production construction always supplies the
    /// concrete campaign through [`Self::new_with_campaign`].
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::new_with_campaign(crate::campaign::Campaign::default())
    }

    /// Create the deterministic kernel for a live mission. Downstream crates
    /// construct through the [`Engine`] facade, so every production path must
    /// supply the campaign up front.
    pub(crate) fn new_with_campaign(campaign: crate::campaign::Campaign) -> Self {
        // Engine starts with canonical seat 0. This is not "the local
        // player"; every peer has the same seat table, and joined peers
        // add deterministic seats via `ConnectSeat`.
        //
        Self {
            mission_domain: MissionDomain::new(campaign),
            // Original: the `__TEST` path in
            // `original-code/launcher.cpp:762-766` calls `srand(0)`.
            // `Engine::new` replaces this bare-engine test seed with the
            // replay/match seed before level setup draws.
            control: SimulationControl::new(0, SimConfig::default()),
            ai: AiRuntime::new(),
            world: WorldState::new(),
            script_domains: state::ScriptDomains::default(),
            orders: OrderRuntime::new(),

            scripts: ScriptRuntime::new(),

            players: PlayerRuntime::new(),
            feedback: FeedbackRuntime::new(),
        }
    }

    /// Post-load initialization: scripts, AI, animation preloading.
    ///
    /// Called from `Engine::new` after level loading is complete.
    pub(crate) fn initialize(&mut self, assets: &mut LevelAssets) {
        self.with_simulation_context(|engine, sim| engine.initialize_inner(assets, sim));
    }

    /// Run non-tick simulation work against the engine's authoritative RNG.
    ///
    /// This is also used by focused tests that invoke a normally tick-owned
    /// subsystem directly. The capability remains tied to this engine's one
    /// serialized stream and cannot be omitted by a downstream caller.
    pub(crate) fn with_simulation_context<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &crate::sim_rng::SimulationContext) -> R,
    ) -> R {
        let sim = self.control.simulation_context();
        f(self, &sim)
    }

    fn initialize_inner(
        &mut self,
        assets: &mut LevelAssets,
        sim: &crate::sim_rng::SimulationContext,
    ) {
        // Called from `Engine::new` after the motion stage
        // has built out `fast_grid` (grid size + map bbox + motion
        // lines) and loaded the pathfinder graph.  Everything the
        // downstream initialization steps (scroll randomization,
        // pathfinder state init, AI init's `TestIfPathIsFine` checks)
        // need is in place.

        // Validate actor placement against the motion grid: fatal if an
        // actor sits on a layer past `fast_grid.special_layer`, warn if
        // its move-box intersects an obstacle.  Shipped data never trips
        // these, but a malformed mission file would otherwise slide
        // through silently and leave actors in unreachable positions.
        self.validate_actor_placement();

        // Pathfinder obstacle states now that the graph is loaded.
        if !assets.pathfinder_graph.static_data.move_layers.is_empty() {
            self.world
                .pathfinder
                .initialize_from_graph(assets.pathfinder_graph.as_ref(), &mut self.world.fast_grid);
        }

        // Original RHEngine::Initialize runs IEngineScript::Initialize(0)
        // before RHArtificialIntelligence::InitAI. This ordering is required
        // now that InitOneAI's typed SetState calls synchronously dispatch
        // FilterAIEvent through the bound actor VMs.
        if self.scripts.mission.is_some() {
            self.initialize_mission_script_with(sim, assets, 0, &assets.hiking_paths);
        }

        // The original initializes scrolls immediately after the engine
        // script and before AI. ForceRandomSpriteFrame is the remaining
        // entity-side half of that step.
        self.initialize_all_scrolls(sim);

        // Notify UI to update stature display
        self.orders
            .messenger
            .send(Message::new(MessageType::Simple(SimpleMessage::Stature)));

        // Initialize AI for all NPCs and global AI state. Runs here —
        // not pre-bitmap — because `init_one_ai`'s `TestIfPathIsFine`
        // reads `fast_grid.map_bbox` + motion lines, and the
        // pathfinder's `move_box_half_diagonals` table must already be
        // populated so `spawn_soldier`'s move_box ends up at the real
        // profile-sized pathfinder box instead of the `(-1,-1,1,1)`
        // fallback.
        self.init_ai(sim, assets);

        // Original closes mission loading by centering on and selecting the
        // playable PC with the greatest character-profile priority. This is
        // authoritative selection state even before the first input frame.
        let initial_pc = self
            .world
            .pc_ids
            .iter()
            .copied()
            .filter_map(|pc_id| {
                let Entity::Pc(pc) = self.world.entities.get(pc_id)? else {
                    return None;
                };
                if !pc.pc.playable {
                    return None;
                }
                let priority = assets
                    .profile_manager
                    .get_character(pc.pc.profile_index)?
                    .priority;
                Some((pc_id, priority))
            })
            .max_by_key(|&(_, priority)| priority)
            .map(|(pc_id, _)| pc_id);
        if let Some(pc_id) = initial_pc {
            assert!(
                self.is_pc_selectable(assets, pc_id),
                "highest-priority playable PC {pc_id:?} is not selectable after mission initialization"
            );
            self.select_pc(assets, 0, pc_id, false, false);
            assert_eq!(
                self.players.seats[0].selection.as_slice(),
                &[pc_id],
                "initial PC selection message did not select its target"
            );
        }

        // Update player's ears position.
        self.update_sound_listener_position();

        tracing::info!("EngineInner: initialization complete");
    }

    /// Verify every actor's mission-start placement is legal.
    ///
    /// * Out-of-range layer (`GetLayer() > GetSpecialLayer()`) — shipped
    ///   data never trips it.  We use `tracing::error!` (no panic) so a
    ///   bad mission file still boots while yelling in the logs.
    /// * Move-box colliding with an obstacle is a non-fatal warn.
    fn validate_actor_placement(&self) {
        let special_layer = self.world.fast_grid.level.special_layer;
        for (_, entity) in self.world.entities.actors() {
            let elem = entity.element_data();
            let layer = elem.layer();
            if layer == 0xFFFF {
                continue;
            }
            let pos = elem.position_map();
            if layer > special_layer {
                tracing::error!(
                    "Actor at ({:.1},{:.1}) lies on out-of-range layer {} \
                     (special_layer={})",
                    pos.x,
                    pos.y,
                    layer,
                    special_layer,
                );
                continue;
            }
            let move_box = elem.sprite.position_iface.get_move_box_map();
            if !self.world.fast_grid.is_position_authorized(move_box, layer) {
                tracing::warn!(
                    "Actor at ({:.1},{:.1}) lies inside an obstacle on layer {}",
                    pos.x,
                    pos.y,
                    layer,
                );
            }
        }
    }

    /// Mission-start wakeup for every scroll entity.
    ///
    /// Walks every scroll entity and runs its initialization: the
    /// scroll's script `Initialize` (pending the scroll script
    /// subsystem port) and then `ForceRandomSpriteFrame` so every
    /// scroll starts on a random frame of its fluttering animation
    /// instead of all waving in lockstep.
    fn initialize_all_scrolls(&mut self, sim: &crate::sim_rng::SimulationContext) {
        for (_, scroll) in self.world.entities.scrolls_mut() {
            // Original: `RHElementScroll::Initialize` in
            // `original-code/RHElementScroll.cpp:153-171` calls
            // `ForceRandomSpriteFrame` after script initialization.
            scroll
                .element
                .sprite
                .force_random_sprite_frame(sim, crate::sim_rng::RngSite::ScrollInitialFrame);
        }
    }

    // ─── Timer management ────────────────────────────────────────

    /// Add an anonymous countdown timer.
    ///
    /// The sequence element reference lets us fire `element_terminated`
    /// when the timer elapses.
    pub(crate) fn add_timer(
        &mut self,
        remaining_frames: u32,
        element_ref: crate::sequence::SequenceElementRef,
    ) {
        self.orders.timer_elements.push(TimerEntry {
            remaining: remaining_frames,
            element_ref,
        });
    }

    /// Terminate the currently-tracked camera sequence element (if any)
    /// and clear the slot. Called before latching a new camera command
    /// onto [`CameraState::sequence_element`]; the previous element is
    /// transitioned to `Terminated` and the slot nulled.
    pub(super) fn terminate_prev_camera_sequence_element(&mut self) {
        if let Some(r) = self.feedback.cutscene_camera.sequence_element.take() {
            self.orders
                .sequence_manager
                .element_terminated(r.sequence_id, r.element_index);
        }
    }

    // ─── Mission control ─────────────────────────────────────────

    /// Get the current mission's type from the campaign, if available.
    pub fn mission_type(&self, profiles: &crate::profiles::ProfileManager) -> Option<MissionType> {
        let campaign = &self.mission_domain.campaign;
        let idx = campaign.current_mission_idx?;
        Some(campaign.missions.get(idx)?.profile(profiles).mission_type)
    }

    /// Signal that the mission has been won.
    ///
    /// `show_window` controls whether the "leave mission" UI message is
    /// displayed.  For ambush/tactical missions, this is `false`.
    ///
    /// Both flags are written unconditionally on every call, so a script
    /// sequence like `Win(false)` then `Win(true)` (or any second call
    /// via [`EngineCommand::Win`]) re-toggles `mission_won_first_time`.
    /// When `show_window == false`, the Sherwood start/quit-mission
    /// widgets are flipped via
    /// [`SideEffects::pending_silent_win_widget_swap`].
    pub(crate) fn win(&mut self, show_window: bool) {
        self.mission_domain.state.mission_won_first_time = show_window;
        self.mission_domain.state.mission_won = true;

        if !show_window {
            self.feedback
                .pending_side_effects
                .pending_silent_win_widget_swap = true;
        }
    }

    /// Clean up and signal mission quit.
    pub(crate) fn quit_mission(&mut self) {
        tracing::info!("EngineInner: mission quit");
    }

    /// Apply end-of-mission updates.
    ///
    /// Marks the mission done, counts soldiers, resets PC comas, and —
    /// if won — awards score bonuses, recruits peasants, and consumes
    /// blazons.  Called from the game session loop when the engine tick
    /// signals mission end, before the debriefing is shown.
    ///
    pub(crate) fn apply_quit_mission_updates(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        exit_code: crate::game_operation::GameCode,
        difficulty: crate::player_profile::DifficultyLevel,
    ) {
        let won = exit_code == crate::game_operation::GameCode::LevelSucceeded;

        let profiles = &assets.profile_manager;
        let campaign = self
            .mission_domain
            .required_campaign_mut("quit-mission updates");
        if campaign.current_mission_idx.is_some() {
            campaign.set_mission_done(won, None, profiles);
        }

        let (living, dead) = self.count_soldiers_at_quit();

        self.reset_all_pc_comas(assets);

        if won
            && self
                .mission_domain
                .required_campaign("quit-mission updates")
                .current_mission_idx
                .is_some()
        {
            // The LIVING/DEAD/SCORE `AddValue` calls are gated on
            // `mission_won` — a lost mission must NOT accumulate these
            // totals onto the campaign.
            let tied_score = self.score_tied_unconscious_soldiers();
            self.quit_mission_context()
                .apply_won_updates(sim, profiles, living, dead, tied_score, difficulty);
        } else {
            // Explicitly zero on the lost path.
            self.mission_domain.mission_stat.new_peasant_count = 0;
        }
    }

    /// Split-borrow the engine owners used by the campaign-only tail of
    /// mission teardown. This deliberately cannot expose `&mut EngineInner`.
    fn quit_mission_context(&mut self) -> QuitMissionContext<'_> {
        let Self {
            mission_domain,
            control,
            feedback,
            ..
        } = self;
        let (campaign, mission_stat) =
            mission_domain.required_campaign_and_stat("quit-mission updates");
        QuitMissionContext {
            campaign,
            mission_stat,
            pending_side_effects: &mut feedback.pending_side_effects,
            frame_counter: control.frame_counter,
        }
    }

    /// Compute score bonus for living enemy soldiers that are tied or
    /// unconscious: iterates all Lacklandist soldiers, adds
    /// `SCORE_SOLDIER_TIED_AND_UNCONSCIOUS` (70) for each living soldier
    /// that is tied or unconscious.
    ///
    pub fn score_tied_unconscious_soldiers(&self) -> i32 {
        use crate::element::{Actor as _, Camp, Human as _};
        const SCORE_SOLDIER_TIED_AND_UNCONSCIOUS: i32 = 70;

        let mut score = 0;
        for (_, s) in self.world.entities.soldiers() {
            if s.camp() == Camp::Lacklandists
                && s.life_points() > 0
                && (s.is_tied() || s.is_unconscious())
            {
                score += SCORE_SOLDIER_TIED_AND_UNCONSCIOUS;
            }
        }
        score
    }

    /// Count living and dead Lacklandist soldiers by iterating entities.
    ///
    /// Counts at quit time rather than reading pre-accumulated stats,
    /// ensuring accuracy.  Also populates
    /// `mission_stat.living_soldier_count` and
    /// `mission_stat.total_soldier_count`.
    pub(crate) fn count_soldiers_at_quit(&mut self) -> (u32, u32) {
        use crate::element::{Camp, Human as _};

        let mut living = 0u32;
        let mut dead = 0u32;
        for (_, s) in self.world.entities.soldiers() {
            if s.camp() == Camp::Lacklandists {
                if s.life_points() > 0 {
                    living += 1;
                } else {
                    dead += 1;
                }
            }
        }
        // The living-soldier increment runs inside the per-soldier
        // loop, accumulating onto whatever was previously in the stat
        // rather than overwriting.  Match the additive semantics so any
        // earlier writer's contribution survives.  `total_soldier_count`
        // is kept in lockstep.
        self.mission_domain.mission_stat.living_soldier_count = self
            .mission_domain
            .mission_stat
            .living_soldier_count
            .saturating_add(living);
        self.mission_domain.mission_stat.total_soldier_count = self
            .mission_domain
            .mission_stat
            .total_soldier_count
            .saturating_add(living + dead);
        (living, dead)
    }

    /// Reset coma state on all PCs at mission end.
    ///
    /// Iterates all PCs and calls ResetComa on any that are in coma
    /// (amulet death-save).
    pub(crate) fn reset_all_pc_comas(&mut self, assets: &LevelAssets) {
        let coma_pc_ids: Vec<EntityId> = {
            let campaign = self
                .mission_domain
                .required_campaign("quit-mission updates");
            self.world
                .pc_ids
                .iter()
                .copied()
                .filter(|&pc_id| match self.world.entities.get(pc_id) {
                    Some(Entity::Pc(pc)) => campaign
                        .characters
                        .get(usize::from(pc.pc.profile_index))
                        .map(|desc| desc.status.in_coma)
                        .unwrap_or(false),
                    _ => false,
                })
                .collect()
        };
        for pc_id in coma_pc_ids {
            self.reset_coma(assets, pc_id);
        }
    }

    // ─── Fast forward ────────────────────────────────────────────

    pub fn is_fast_forward(&self) -> bool {
        self.control.fast_forward
    }

    pub(crate) fn set_fast_forward(&mut self) {
        self.control.fast_forward = true;
        if self.feedback.cutscene_camera.is_sliding() {
            self.feedback.cutscene_camera.view_position =
                self.feedback.cutscene_camera.camera_slide;
        }
        self.feedback.cutscene_camera.stop_slide();
    }

    /// Effective alt state (physical Alt held OR the lock toggle is on).
    pub fn is_alt_effective(&self, input: &InputState) -> bool {
        input.is_alt || self.players.seats[0].is_lock_alt
    }

    /// The persistent alt-lock flag on its own, ignoring the transient
    /// physical-alt state.  The sight HUD button reads this to draw
    /// itself as latched.
    pub fn is_lock_alt(&self) -> bool {
        self.players.seats[0].is_lock_alt
    }

    // ─── State changes ───────────────────────────────────────────

    /// Handle a state change request.
    #[allow(clippy::collapsible_match)]
    pub(crate) fn change_state(
        &mut self,
        display: &mut HostDisplayState,
        seat: usize,
        request: EngineStateRequest,
    ) -> bool {
        match request {
            EngineStateRequest::LockerOn => {
                self.players.seats[seat].locker_active = true;
                true
            }
            EngineStateRequest::LockerOff => {
                self.players.seats[seat].locker_active = false;
                true
            }
            EngineStateRequest::ZoomingUp => {
                // Display transition state is host-owned and supplied by
                // the caller, so it never enters the simulation snapshot.
                if self.is_zoom_possible_for_seat(display, seat)
                    && self.is_zoom_up_possible_for_seat(seat)
                {
                    display.background_transform.required_zoom_up = false;
                    // Every MSG_ZOOM_UP receipt rewrites
                    // `mechanized_zoom` from the message value;
                    // user-initiated paths (keyboard/HUD/pad) pass value 0.
                    // Script-initiated zooms set `mechanized_zoom = true`
                    // separately via the `desired_zoom_factor` dispatch
                    // (`perform_director_work`) / `SetZoomLevel` script
                    // native, which execute before `ChangeState` fires.
                    self.feedback.cutscene_camera.mechanized_zoom = false;
                    // Can only initiate zoom when not scrolling
                    if display.background_transform.current_x_scrolling_level == 0
                        && display.background_transform.current_y_scrolling_level == 0
                        && display.display_op != DisplayOpCode::InitZoom
                        && display.display_op != DisplayOpCode::InZoom
                    {
                        if display.background_transform.current_zoom_level < 2 {
                            display.background_transform.current_zoom_level += 1;
                            display.background_transform.zoom_to_up = true;
                            self.set_operation(display, DisplayOpCode::InitZoom);
                        }
                    } else {
                        // Defer zoom until scrolling finishes
                        display.background_transform.required_zoom_up = true;
                        display.background_transform.required_zoom_down = false;
                    }
                    true
                } else {
                    false
                }
            }
            EngineStateRequest::ZoomingDown => {
                if self.is_zoom_possible_for_seat(display, seat)
                    && self.is_zoom_down_possible_for_seat(seat)
                {
                    display.background_transform.required_zoom_down = false;
                    // See ZoomingUp for the rationale on resetting
                    // `mechanized_zoom` from the message value.
                    self.feedback.cutscene_camera.mechanized_zoom = false;
                    if display.background_transform.current_x_scrolling_level == 0
                        && display.background_transform.current_y_scrolling_level == 0
                        && display.display_op != DisplayOpCode::InitZoom
                        && display.display_op != DisplayOpCode::InZoom
                    {
                        if display.background_transform.current_zoom_level > 0 {
                            display.background_transform.current_zoom_level -= 1;
                            display.background_transform.zoom_to_down = true;
                            self.set_operation(display, DisplayOpCode::InitZoom);
                        }
                    } else {
                        display.background_transform.required_zoom_up = false;
                        display.background_transform.required_zoom_down = true;
                    }
                    true
                } else {
                    false
                }
            }
            EngineStateRequest::EnterMenu => {
                // EnterMenu is a no-op that just returns true.
                true
            }
            _ => {
                // Returns false for every other state — night dimish /
                // night colour are set once at init and only readable
                // via `GetState`, and the other variants (locker, zoom,
                // beacon, …) are toggled by dedicated code paths rather
                // than through `ChangeState`.
                false
            }
        }
    }

    // ─── Script globals ──────────────────────────────────────────

    //
    // Backs the `InitScriptGlobal` script native. Will become live once
    // that native is wired in `crates/robin_engine/src/natives/`;
    // currently exercised only by `engine::tests::script_globals`.
    #[allow(dead_code)] // port-in-progress: awaiting `IInitGlobal` native plumbing
    pub(crate) fn init_script_global(&mut self, id: usize, value: i32) {
        // Resize the array to `id + 16` when `id` is out of range,
        // giving scripts a 16-slot slack window of valid reads/writes
        // past the last initialised index. Any script that pokes within
        // this window sees `0` defaults.
        if id + 16 > self.scripts.globals.len() {
            self.scripts.globals.resize(id + 16, 0);
        }
        self.scripts.globals[id] = value;
    }

    //
    // Backs the `SetScriptGlobal` script native. Will become live once
    // that native is wired in `crates/robin_engine/src/natives/`;
    // currently exercised only by `engine::tests::script_globals` /
    // `script_global_set_out_of_range_panics`.
    #[allow(dead_code)] // port-in-progress: awaiting `ISetGlobal` native plumbing
    pub(crate) fn set_script_global(&mut self, id: usize, value: i32) {
        if id < self.scripts.globals.len() {
            self.scripts.globals[id] = value;
        } else {
            panic!(
                "Script global ID {} out of range (max {})",
                id,
                self.scripts.globals.len()
            );
        }
    }

    /// Get a script global variable.
    pub fn get_script_global(&self, id: usize) -> i32 {
        self.scripts
            .globals
            .get(id)
            .copied()
            .unwrap_or_else(|| panic!("Script global ID {} out of range", id))
    }

    /// Check if a script global ID is valid.
    pub fn is_valid_script_global_id(&self, id: usize) -> bool {
        id < self.scripts.globals.len()
    }

    // ─── Entity management ──────────────────────────────────────

    /// Add an entity to the world. Returns its EntityId.
    pub(crate) fn add_entity(&mut self, mut entity: Entity) -> EntityId {
        let id = entity_id_for_occupied_slot(self.world.entities.len() as u32, &entity);

        if let Entity::Pc(pc) = &mut entity {
            let position = pc.element.position_map();
            pc.actor.produced_noise = Some(crate::ai::Noise {
                origin: crate::ai::Position {
                    x: position.x,
                    y: position.y,
                    sector: pc.element.sector(),
                    level: pc.element.layer(),
                },
                noise_type: crate::ai::NoiseType::Off,
                volume: 0,
                elevation: pc.element.sprite.position_iface.get_elevation() as u16,
                element_id: u16::try_from(id.index()).unwrap_or_else(|_| {
                    panic!(
                        "PC legacy slot {} exceeds noise element-id range",
                        id.index()
                    )
                }),
            });
        }

        // Initialise outline colours based on entity kind.  For
        // soldiers, route the VIP flag (cached on `EnemyAi.is_vip` from
        // the soldier profile at level load) so VIP soldiers get the
        // purple `OC_NPC_VIP_*` outline scheme rather than the standard
        // red enemy scheme.
        let is_vip = match &entity {
            Entity::Soldier(s) => s.npc.ai_brain.enemy().map(|ai| ai.is_vip).unwrap_or(false),
            _ => false,
        };
        entity.element_data_mut().init_outline_colors(is_vip);

        // Override the Hidden/Default/Target outline-colour slots with
        // the VIP palette when the civilian is a VIP, applied here after
        // the base civilian colours are written.
        if let Entity::Civilian(c) = &entity
            && c.civilian.cached_civilian_type == crate::profiles::CivilianType::Vip
        {
            use crate::element::OutlineColorName as N;
            use crate::element_kinds::outline_colors::*;
            let colors = &mut entity.element_data_mut().outline_colors;
            colors[N::Hidden as usize] = npc_vip_hidden();
            colors[N::Default as usize] = npc_vip_default();
            colors[N::Target as usize] = npc_vip_target();
        }

        // Track kind lists that carry ordering semantics. Other views
        // are derived from the entity store.
        match &entity {
            Entity::Pc(_) => {
                self.world.pc_ids.push(id);
                self.world.original_pc_registry_ids.push(id);
            }
            Entity::Soldier(_) => {}
            Entity::Civilian(_) => {}
            Entity::Fx(_) => {}
            Entity::Target(_) | Entity::Net(_) | Entity::Scroll(_) | Entity::Projectile(_) => {}
            Entity::Bonus(_) => {}
        }

        self.world.entities.push(Some(entity));
        self.world.assign_next_original_creation_order(id);
        id
    }

    /// Get a reference to an entity by ID.
    pub fn get_entity<I: Into<EntityId>>(&self, id: I) -> Option<&Entity> {
        self.world.entities.get(id)
    }

    /// Return the authoritative Original `RHElement::mulCreationOrder` for
    /// an entity.
    ///
    /// This is the stable cross-engine identity used by parity tooling and
    /// legacy-save fixups. Rust entity-table slots are not equivalent:
    /// Original mobile masters consume creation orders without occupying a
    /// Rust entity slot, and the Rust loader constructs authored categories
    /// in a different order.
    pub fn original_creation_order<I: Into<EntityId>>(&self, id: I) -> u32 {
        self.world.original_creation_order(id.into())
    }

    /// Resolve a legacy raw entity-table index to the typed ID variant for
    /// the entity currently stored in that slot.
    pub fn entity_id_for_index(&self, index: u32) -> Option<EntityId> {
        self.world.entities.id_at_legacy_slot(index)
    }

    /// Resolve a script actor handle to the typed ID variant for the entity
    /// currently stored in that slot.
    pub(crate) fn entity_id_for_actor_handle(&self, handle: i32) -> Option<EntityId> {
        crate::natives::ScriptHandleCodec::actor_handle_index(handle)
            .and_then(|idx| self.entity_id_for_index(idx as u32))
    }

    /// Resolve a legacy raw entity-table index and panic when the slot is not
    /// present.  Use this for script/AI boundaries that are expected to carry
    /// live entity handles; missing slots indicate corrupted sim state or an
    /// incomplete port rather than an ordinary false condition.
    pub(crate) fn expect_entity_id_for_index(&self, index: u32, context: &str) -> EntityId {
        self.entity_id_for_index(index)
            .unwrap_or_else(|| panic!("{context}: missing entity for raw entity index {index}"))
    }

    /// The command of the actor's currently-executing sequence element,
    /// falling back to `Command::Wait` when no element is `InProgress`.
    /// Used as the authoritative "is this actor idle?" signal — the
    /// `ActorData::action_state` proxy can disagree (e.g. a `WaitTimer`
    /// element drives `action_state = Waiting` but the actor reports
    /// the actual command, not WAIT).
    pub fn actor_command(&self, actor: EntityId) -> crate::element::Command {
        match self
            .orders
            .sequence_manager
            .current_element_for_actor(actor)
        {
            Some((seq_id, idx)) => self
                .orders
                .sequence_manager
                .get_element(seq_id, idx)
                .map(|e| e.command)
                .unwrap_or(crate::element::Command::Wait),
            None => crate::element::Command::Wait,
        }
    }

    /// Value corresponding to Original `RHElementActor::mulWaitTime`.
    ///
    /// Rust keeps the seek-refresh countdown separate from ordinary command
    /// waits, while the Original reuses `mulWaitTime` for both. Debug/parity
    /// consumers need this isomorphic view rather than comparing the Rust
    /// storage layout literally.
    pub fn actor_legacy_wait_time(&self, actor: EntityId) -> u32 {
        let entity = self
            .get_entity(actor)
            .unwrap_or_else(|| panic!("actor_legacy_wait_time: missing actor {actor:?}"));
        let data = entity
            .actor_data()
            .unwrap_or_else(|| panic!("actor_legacy_wait_time: non-actor {actor:?}"));

        // A live WAIT_TIMER owns Original's overloaded scalar even when the
        // actor retains a seek target/post-seek continuation. Rust's split
        // seek-refresh copy is dormant during this command and must not mask
        // the timer that Actor::Hourglass just decremented.
        if self.actor_command(actor) == crate::element::Command::WaitTimer {
            return data.wait_time;
        }

        if let Some(sequence_id) = data.active_movement.sequence_id
            && let Some(element) = self
                .orders
                .sequence_manager
                .get_element(sequence_id, data.active_movement.element_index)
            && let crate::sequence::SequenceElementData::Movement { flags, element, .. } =
                &element.data
            && flags.contains(crate::sequence::MoveFlags::SEEK)
            && element.is_some()
        {
            return data.seek_refresh_wait;
        }

        // Original leaves the overloaded seek countdown untouched when the
        // post-seek interaction takes over. Rust has already cleared
        // `active_movement` at that boundary, but the retained post-seek
        // sequence identifies the seek copy as the isomorphic value. This
        // explicit ownership test also handles loaded saves, where adoption
        // seeds both split fields from the serialized scalar.
        if data.post_seek_sequence.is_some() && data.seek_target.is_some() {
            return data.seek_refresh_wait;
        }

        data.wait_time
    }

    /// Current animation/order type for parity diagnostics.
    pub fn actor_order_type(&self, actor: EntityId) -> Option<crate::order::OrderType> {
        self.get_entity(actor)
            .and_then(|entity| entity.actor_data())
            .map(|actor| resolve_actor_order_type(actor.installed_order))
    }

    /// Mirror an Original boundary that assigns `mpOrder` from the selected
    /// sequence element's current order. Callers must invoke this only where
    /// the C++ source performs that assignment (Hourglass, accepted Instruct,
    /// or corrected movement retranslation), never as a read-time fallback.
    pub(crate) fn publish_selected_order_as_installed(&mut self, actor: EntityId) {
        let installed_order = self
            .orders
            .sequence_manager
            .current_order_for_actor(actor)
            .map(|(_, _, order)| crate::element::InstalledActorOrder {
                order_id: order.order_id,
                order_type: order.order_type,
            });
        self.get_entity_mut(actor)
            .and_then(Entity::actor_data_mut)
            .expect("mpOrder publication owner lost actor data")
            .installed_order = installed_order;
    }

    /// Apply Actor::Instruct's `mpOrder` publication only when the instructed
    /// owner actually uses the Actor implementation. `InstructOwner` also
    /// carries valid non-actor owners, whose Original classes have no
    /// `mpOrder`; a missing owner remains an invariant failure.
    pub(crate) fn publish_selected_order_for_instruct_owner(&mut self, owner: EntityId) {
        let is_actor = self
            .get_entity(owner)
            .unwrap_or_else(|| panic!("InstructOwner publication owner {owner:?} vanished"))
            .actor_data()
            .is_some();
        if is_actor {
            self.publish_selected_order_as_installed(owner);
        }
    }

    /// Original `RHElementActor::GetAnimation()`: the live sequence order,
    /// falling back to the sprite-driven animation while no order is selected.
    ///
    /// `ActorData::old_action` is not this value. It only retains the previous
    /// animation for the next `ActionChange(new, old)` callback and may remain
    /// `Invalid` throughout an otherwise visible animation.
    pub(crate) fn live_actor_animation(&self, actor: EntityId) -> Option<crate::order::OrderType> {
        self.actor_order_type(actor).or_else(|| {
            self.get_entity(actor)
                .filter(|entity| entity.kind().is_actor())
                .map(|entity| entity.sprite().last_action)
        })
    }

    pub(crate) fn actor_is_in_sword_recovery(&self, actor: EntityId) -> bool {
        use crate::order::OrderType as OT;
        self.live_actor_animation(actor).is_some_and(|animation| {
            matches!(
                animation,
                OT::BeingHitSword
                    | OT::ExtractingArrowSword
                    | OT::DyingSword
                    | OT::BeingDeadSword
                    | OT::FallingBackSword
                    | OT::BeingUnconsciousSword
                    | OT::BeingDeadFallenBackSword
                    | OT::StandingUpSword
            )
        })
    }

    /// Render-time gate for the unconscious-stars titbit.
    ///
    /// Invoked from the titbit renderer to decide whether the stars
    /// sprite should appear above `entity_id` *this frame*.  Checks the
    /// sprite's currently driven animation, not the sequence manager's
    /// front order — during queued damage/push transitions those can
    /// diverge, so use `Sprite::last_action` here.
    pub fn can_have_unconscious_stars(&self, entity_id: EntityId) -> bool {
        let Some(entity) = self.get_entity(entity_id) else {
            return false;
        };
        matches!(
            entity.sprite().last_action,
            crate::order::OrderType::BeingUnconscious
                | crate::order::OrderType::BeingUnconsciousBow
                | crate::order::OrderType::BeingUnconsciousSword
        )
    }

    /// Build a sequence-priority resolver keyed on the engine's entity
    /// table.  Calls `owner.DeterminePriority(elem)` when the priority
    /// is still unset; for non-actor / missing owners falls back to
    /// `Normal`.
    ///
    /// Takes the entity slice by reference so callers can split-borrow
    /// this alongside `&mut self.orders.sequence_manager`.
    pub(crate) fn priority_resolver(
        entities: &crate::entities::Entities,
    ) -> impl Fn(&crate::sequence::SequenceElement) -> crate::sequence::SequencePriority + '_ {
        move |elem| {
            // Sequence-manager registration short-circuits elements
            // whose `executed_immediately` is true — they're dispatched
            // synchronously and never reach `Instruct` /
            // `DeterminePriority`.  Mirror that here so commands like
            // `SEND_MESSAGE` don't fall into the actor_branch default.
            if elem.executed_immediately() {
                return crate::sequence::SequencePriority::Normal;
            }
            let owner_entity = elem.owner.and_then(|id| entities.get(id));
            match owner_entity {
                Some(entity) if entity.kind().is_actor() => {
                    let is_unconscious =
                        entity.human_data().map(|h| h.unconscious).unwrap_or(false);
                    crate::element_priority::determine_priority(
                        crate::element_priority::ActorPriorityContext {
                            kind: entity.kind(),
                            is_dead: entity.is_dead(),
                            is_unconscious,
                        },
                        elem,
                    )
                }
                // No owner or non-actor owner — fall back to Normal.
                _ => crate::sequence::SequencePriority::Normal,
            }
        }
    }

    /// Resolve `elem.priority` via [`Self::priority_resolver`] if it is
    /// still `NotYetSet`.  Eager `DeterminePriority` runs when a new
    /// sequence element is handed to an actor.
    fn resolve_element_priority(&self, elem: &mut crate::sequence::SequenceElement) {
        if elem.priority == crate::sequence::SequencePriority::NotYetSet {
            let resolver = Self::priority_resolver(&self.world.entities);
            elem.priority = resolver(elem);
        }
    }

    /// Register one element through Original's `LaunchSequenceElement`
    /// boundary.
    ///
    /// Ordinary owner work remains untouched until the later sequence-
    /// manager Hourglass calls `Go -> Instruct`. Priority, transition stamps,
    /// generated orders, and arbitration therefore observe actor state at
    /// instruction time, after every entity has completed its current
    /// Hourglass slot. `SequenceManager` separately routes the two Original
    /// registration-time exceptions: explicit `RHPRIORITY_WAIT` work and the
    /// `ExecutedImmediately` command whitelist.
    pub(crate) fn launch_element(
        &mut self,
        elem: crate::sequence::SequenceElement,
    ) -> crate::sequence::SequenceId {
        self.orders.sequence_manager.launch_element(elem)
    }

    /// Register an owned element without running its `Instruct` boundary
    /// inline.
    ///
    /// Actor `Execute` callbacks call `LaunchSequenceElement`, which only
    /// appends to the Original sequence-manager queue. The actor may still
    /// finish its current order before the manager instructs the registered
    /// element. Its unresolved priority is deliberately preserved here:
    /// resolving a wait-priority element would route it through Rust's
    /// synchronous wait queue and expose its `Instruct` before the derived
    /// Human/NPC tail. This explicit name remains at older actor-execute call
    /// sites; the ordinary [`Self::launch_element`] wrapper now has the same
    /// deferred semantics for every `LaunchSequenceElement` call.
    pub(crate) fn register_owned_element_deferred(
        &mut self,
        elem: crate::sequence::SequenceElement,
    ) -> crate::sequence::SequenceId {
        assert!(
            elem.owner.is_some(),
            "register_owned_element_deferred requires an actor owner"
        );
        self.orders.sequence_manager.launch_element(elem)
    }

    /// Direct `Instruct`-equivalent for an already-admitted owned element:
    /// resolve priority, launch via the sequence manager, stamp the actor's
    /// current posture / action state onto the element, then arbitrate
    /// against the actor's currently-executing element. This is not the
    /// implementation of Original `LaunchSequenceElement`, which merely
    /// registers ordinary work for the manager Hourglass; use it only when
    /// the caller is already modelling an `Instruct` boundary directly.
    ///
    /// Caller invariant: `elem.owner` is `Some`.  The returned
    /// `SequenceId` is for the freshly-minted single-element sequence;
    /// the element sits at index 0.
    pub(crate) fn launch_element_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mut elem: crate::sequence::SequenceElement,
    ) -> crate::sequence::SequenceId {
        let parity_debug_stage_timing = std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some();
        debug_assert!(
            elem.owner.is_some(),
            "launch_element_for_owner requires elem.owner"
        );
        let mut owner = elem.owner.expect("owner present");
        if parity_debug_stage_timing {
            eprintln!(
                "parity launch: launch_element_for_owner enter owner={owner:?} command={:?}",
                elem.command
            );
        }

        // PC on a carrier's shoulders, receiving a Move-to-jump command,
        // delegates the move to the carrier (with the TO_JUMP + SEEK
        // flags stripped).  The net effect: the carrier walks to the
        // jump point and the PC rides along on their shoulders.
        self.redirect_move_to_jump_if_carried(&mut elem, &mut owner);

        // Unfreeze actor on any incoming command, so a
        // `FreezeExecution`'d actor can be resumed by dispatching
        // a new element (e.g. scripted Wait on a held PC).
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.execution_frozen = false;
        }

        // `Command::Null` short-circuits to Terminated without running
        // priority / transition / translate.
        if elem.command == crate::element::Command::Null {
            let seq_id = self.orders.sequence_manager.launch_element(elem);
            self.orders.sequence_manager.element_terminated(seq_id, 0);
            return seq_id;
        }

        // RHElementActorHuman::Instruct retains repeated PC bow shots before
        // delegating to Actor::Instruct. Keep the registered element wholly
        // untouched: priority resolution, transition stamps, generated
        // orders, and ordinary arbitration all belong to the later retry.
        if self.pc_should_hold_shoot_bow(owner, elem.command) {
            let seq_id = self.orders.sequence_manager.launch_element(elem);
            self.orders
                .sequence_manager
                .hold_deferred_element(seq_id, 0);
            self.queue_pc_shoot_bow(owner, crate::sequence::SequenceElementRef::new(seq_id, 0));
            return seq_id;
        }

        if parity_debug_stage_timing {
            eprintln!(
                "parity launch: before resolve priority owner={owner:?} command={:?}",
                elem.command
            );
        }
        self.resolve_element_priority(&mut elem);
        if parity_debug_stage_timing {
            eprintln!(
                "parity launch: after resolve priority owner={owner:?} command={:?} priority={:?}",
                elem.command, elem.priority
            );
        }
        let seq_id = self.orders.sequence_manager.launch_element(elem);
        let elem_idx = 0;
        if parity_debug_stage_timing {
            eprintln!("parity launch: registered owner={owner:?} seq={seq_id:?} idx={elem_idx}");
        }

        // Stamp posture / action-state as the after-transition defaults
        // before any priority or transition logic runs.  See
        // `stamp_element_transition_state` for the rationale.
        if parity_debug_stage_timing {
            eprintln!("parity launch: before stamp owner={owner:?} seq={seq_id:?}");
        }
        self.stamp_element_transition_state(owner, seq_id, elem_idx);
        if parity_debug_stage_timing {
            eprintln!("parity launch: after stamp owner={owner:?} seq={seq_id:?}");
        }

        // NonInterruptable current short-circuit: postpone new (or mark
        // IMPOSSIBLE for PASS_DOOR+MOVE) without running
        // GenerateTransition.
        if parity_debug_stage_timing {
            eprintln!(
                "parity launch: before non_interruptable_guard owner={owner:?} seq={seq_id:?}"
            );
        }
        if self.non_interruptable_guard(owner, seq_id, elem_idx) {
            if parity_debug_stage_timing {
                eprintln!(
                    "parity launch: non_interruptable_guard accepted owner={owner:?} seq={seq_id:?}"
                );
            }
            return seq_id;
        }
        if parity_debug_stage_timing {
            eprintln!(
                "parity launch: after non_interruptable_guard owner={owner:?} seq={seq_id:?}"
            );
        }

        // Auto-insert the exit / posture / enter transition sub-orders
        // before the command's own Translate runs.  Returning false
        // means no valid transition exists — set the element Impossible
        // and skip arbitration.
        if parity_debug_stage_timing {
            eprintln!("parity launch: before generate_transition owner={owner:?} seq={seq_id:?}");
        }
        if !self.generate_transition(sim, assets, owner, seq_id, elem_idx) {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return seq_id;
        }
        if parity_debug_stage_timing {
            eprintln!("parity launch: after generate_transition owner={owner:?} seq={seq_id:?}");
            eprintln!("parity launch: before arbitrate owner={owner:?} seq={seq_id:?}");
        }

        self.arbitrate_instruct(seq_id, elem_idx);
        if parity_debug_stage_timing {
            eprintln!("parity launch: after arbitrate owner={owner:?} seq={seq_id:?}");
            eprintln!(
                "parity launch: launch_element_for_owner exit owner={owner:?} seq={seq_id:?}"
            );
        }
        seq_id
    }

    /// Engine-side wrapper for
    /// [`SequenceManager::launch_single_order_sequence_unchecked`] that
    /// runs the synchronous `Instruct`-equivalent — posture stamp +
    /// priority arbitration — before the element is promoted to
    /// `InProgress`.  This is the blessed path for owner-carrying
    /// single-order sequences; a grep for
    /// `launch_single_order_sequence_unchecked` should turn up only
    /// this wrapper, making it obvious in review when a future change
    /// bypasses the stamp / arbitration.
    ///
    /// The blessed pattern for `BeginSwordfight` / `QuitSwordfight` /
    /// `process_pending_ai_orders` where the order must be visible to
    /// same-frame consumers (animation driver,
    /// `current_order_for_actor`).  If arbitration rejects the element
    /// (Abandon / Postpone), the `InProgress` promotion is skipped —
    /// the element carries the correct terminal state and downstream
    /// scanners ignore it.
    pub(crate) fn launch_single_order_sequence_stamped(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        command: crate::element::Command,
        order: crate::order::Order,
    ) -> crate::sequence::SequenceId {
        self.launch_single_order_sequence_stamped_ex(sim, assets, owner, command, order, true)
    }

    /// Like [`launch_single_order_sequence_stamped`] but with an
    /// explicit toggle for the auto-insert `generate_transition` pass.
    ///
    /// `with_transitions = false` is reserved for synthetic prebuilt-order
    /// lowerings whose Original command path has already selected the exact
    /// transition order. Ordinary `LaunchSequence` calls still reach Actor
    /// `Instruct` and generate transitions at the later manager boundary.
    pub(crate) fn launch_single_order_sequence_stamped_ex(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        command: crate::element::Command,
        order: crate::order::Order,
        with_transitions: bool,
    ) -> crate::sequence::SequenceId {
        self.launch_single_order_sequence_stamped_ex_configured(
            sim,
            assets,
            owner,
            command,
            order,
            with_transitions,
            |_| {},
        )
        .0
    }

    /// Configured single-order launch that installs element properties before
    /// priority arbitration and reports whether the owner was instructed.
    ///
    /// The callback supports synthetic commands that need to author element
    /// properties before the direct arbitration boundary.
    pub(crate) fn launch_single_order_sequence_stamped_ex_configured(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        command: crate::element::Command,
        order: crate::order::Order,
        with_transitions: bool,
        configure: impl FnOnce(&mut crate::sequence::SequenceElement),
    ) -> (crate::sequence::SequenceId, bool) {
        use crate::sequence::SequenceState;

        // Unfreeze actor on any incoming command.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.execution_frozen = false;
        }

        // `Command::Null` short-circuits to Terminated.  Launch the
        // element so downstream callers see a terminated sequence
        // (return-true semantics).
        if command == crate::element::Command::Null {
            let seq_id = self
                .orders
                .sequence_manager
                .launch_single_order_sequence_unchecked(owner, command);
            self.orders.sequence_manager.element_terminated(seq_id, 0);
            return (seq_id, false);
        }

        // Launch an EMPTY element so `generate_transition`'s auto-
        // inserted exit/posture/enter transitions get pushed first,
        // then append the pre-baked single order.  Order:
        // GenerateTransition populates the queue with transitions
        // BEFORE Translate pushes the command's own order, so those
        // transitions play before the command's main animation.
        let seq_id = self
            .orders
            .sequence_manager
            .launch_single_order_sequence_unchecked(owner, command);
        let elem_idx = 0;
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
        {
            configure(elem);
            let resolver = Self::priority_resolver(&self.world.entities);
            if elem.priority == crate::sequence::SequencePriority::NotYetSet {
                elem.priority = resolver(elem);
            }
        }
        self.stamp_element_transition_state(owner, seq_id, elem_idx);

        // NonInterruptable guard — see `launch_element_for_owner` for
        // details.
        if self.non_interruptable_guard(owner, seq_id, elem_idx) {
            return (seq_id, false);
        }

        // Auto-insert exit / posture / enter transition orders before
        // the command runs.  If the transition is impossible, mark the
        // element Impossible and skip both arbitration and the
        // InProgress promotion below. Skipped only by synthetic lowering
        // paths that have already chosen their exact transition order.
        if with_transitions && !self.generate_transition(sim, assets, owner, seq_id, elem_idx) {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return (seq_id, false);
        }

        // NOW append the pre-baked command order — transitions are
        // already in front of it (when enabled).
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);

        let accepted = self.arbitrate_instruct(seq_id, elem_idx);
        // Synchronously promote to `InProgress` so same-frame consumers
        // (animation driver, `current_order_for_actor`) see the
        // attached order without waiting for the next hourglass pass.
        // Skip when arbitration rejected the element (Abandon /
        // Postpone) — downstream scanners filter on state.
        let state = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.state);
        let mut instructed = false;
        if accepted && matches!(state, Some(SequenceState::Todo)) {
            // After `DecidePriorities` runs an `INTERRUPT_CURRENT`
            // cascade, re-check the actor's current element pointer.  A
            // cascade may have started a postponed successor, in which
            // case the pointer no longer matches this new element and
            // the synchronous InProgress promotion must be skipped.
            let still_current = match self.current_sequence_element_for_actor(owner) {
                Some((cur_seq, cur_idx)) => cur_seq == seq_id && cur_idx == elem_idx,
                None => true, // no current — we're free to promote
            };
            if still_current {
                self.orders
                    .sequence_manager
                    .element_in_progress(seq_id, elem_idx);
                instructed = true;
                // Mirror the original actor lifecycle flag when the element
                // transitions to InProgress.
                if let Some(entity) = self.world.entities.get_mut(owner)
                    && let Some(actor) = entity.actor_data_mut()
                {
                    actor.sequence_element_started = true;
                }
                // Actor::Instruct publishes the selected order through
                // mpOrder before returning.  The prebuilt-order fast path
                // bypasses the normal InstructOwner dispatcher, so mirror
                // that publication here as well; same-frame GetAnimation()
                // calls must see this order even though Execute will not run
                // it until the actor's next Hourglass slot.
                self.publish_selected_order_for_instruct_owner(owner);
            }
        }
        (seq_id, instructed)
    }

    /// Register a direct FaceTo-style Turn without instructing its owner yet.
    /// Cross-owner patrol coordination runs before the member's entity slot;
    /// Original SequenceManager::Hourglass arbitrates the Turn only after that
    /// slot, allowing Halt's retained movement exit transition to execute once.
    pub(crate) fn launch_turn_sequence_deferred_no_transitions(
        &mut self,
        owner: EntityId,
        command: crate::element::Command,
        explicit_direction: Option<i16>,
        target_x: f32,
        target_y: f32,
        retained_movement_goal: Option<crate::coordinates::MapPoint>,
    ) -> crate::sequence::SequenceId {
        let seq_id = self
            .orders
            .sequence_manager
            .launch_single_order_sequence_unchecked(owner, command);
        if let Some(element) = self.orders.sequence_manager.get_element_mut(seq_id, 0) {
            if let Some(direction) = explicit_direction {
                element.set_property(
                    crate::sequence::Field::Direction,
                    crate::sequence::FieldValue::Integer(direction as u32),
                );
            } else {
                element.set_property(
                    crate::sequence::Field::CameraPoint,
                    crate::sequence::FieldValue::GeoPoint2D {
                        x: target_x,
                        y: target_y,
                    },
                );
            }
            if let Some(goal) = retained_movement_goal {
                element.set_property(
                    crate::sequence::Field::RetainedMovementGoal,
                    crate::sequence::FieldValue::GeoPoint2D {
                        x: goal.x,
                        y: goal.y,
                    },
                );
            }
        }
        seq_id
    }

    /// Stamp the actor's current posture / action-state onto the new
    /// sequence element as `posture_after_transition` /
    /// `action_state_after_transition`.  Downstream Translate arms read
    /// these to gate posture-specific animation branches —
    /// ENTER_ATTENTIVE_MODE plays the lean-forward transition only when
    /// `posture_after_transition == Upright`, which is why an
    /// un-stamped element (leaving the field at `Posture::Undefined`)
    /// would cause the alerted transition animation to silently not
    /// fire.
    fn stamp_element_transition_state(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let (actor_posture, actor_action_state) = self
            .get_entity(owner)
            .map(|e| {
                let posture = e.element_data().posture;
                let action_state = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                (posture, action_state)
            })
            .unwrap_or_default();
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
        {
            elem.posture_after_transition = actor_posture;
            elem.action_state_after_transition = actor_action_state;
        }
    }

    /// PC-on-shoulders MoveToJump redirect.
    ///
    /// When a PC is riding another PC's shoulders and a Move command
    /// with `TO_JUMP` fires, hand the element off to the carrier with
    /// `TO_JUMP` and `SEEK` cleared — the carrier walks to the jump
    /// point with the rider in tow.
    fn redirect_move_to_jump_if_carried(
        &self,
        elem: &mut crate::sequence::SequenceElement,
        owner: &mut EntityId,
    ) {
        use crate::element::{Command, Posture};
        use crate::sequence::{MoveFlags, SequenceElementData};

        if elem.command != Command::Move {
            return;
        }
        let entity = match self.get_entity(*owner) {
            Some(e) => e,
            None => return,
        };
        if !entity.is_pc() {
            return;
        }
        if entity.element_data().posture != Posture::OnShoulders {
            return;
        }
        let Some(carrier_id) = entity.human_data().and_then(|h| h.carrier) else {
            // The carrier is expected to be present here; if the
            // posture claims OnShoulders but no carrier is tracked,
            // leave the element on the PC and log.
            tracing::warn!(
                ?owner,
                "redirect_move_to_jump_if_carried: OnShoulders posture but no carrier"
            );
            return;
        };
        let SequenceElementData::Movement { flags, .. } = &mut elem.data else {
            return;
        };
        if !flags.contains(MoveFlags::TO_JUMP) {
            return;
        }
        // Strip TO_JUMP and SEEK before handing to the carrier.
        *flags &= !(MoveFlags::TO_JUMP | MoveFlags::SEEK);
        elem.owner = Some(carrier_id);
        *owner = carrier_id;
    }

    /// Apply the PC-on-shoulders movement redirect when the registered
    /// element reaches `Instruct`, matching `RHElementActorPC::Instruct`.
    fn redirect_queued_move_to_jump_if_carried(
        &mut self,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) -> EntityId {
        use crate::element::{Command, Posture};
        use crate::sequence::{MoveFlags, SequenceElementData};

        let should_redirect = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .is_some_and(|element| {
                element.command == Command::Move
                    && matches!(
                        &element.data,
                        SequenceElementData::Movement { flags, .. }
                            if flags.contains(MoveFlags::TO_JUMP)
                    )
            });
        if !should_redirect {
            return owner;
        }
        let Some(rider) = self.get_entity(owner) else {
            return owner;
        };
        if !rider.is_pc() || rider.element_data().posture != Posture::OnShoulders {
            return owner;
        }
        let carrier = rider
            .human_data()
            .and_then(|human| human.carrier)
            .unwrap_or_else(|| panic!("PC {owner:?} is OnShoulders without the required carrier"));

        let element = self
            .orders
            .sequence_manager
            .get_element_mut(sequence_id, element_index)
            .expect("queued shoulder movement disappeared before Instruct");
        let SequenceElementData::Movement { flags, .. } = &mut element.data else {
            unreachable!("TO_JUMP redirect element changed data kind")
        };
        *flags &= !(MoveFlags::TO_JUMP | MoveFlags::SEEK);
        self.orders
            .sequence_manager
            .reassign_element_owner(sequence_id, element_index, carrier);
        carrier
    }

    /// Non-interruptable postpone guard.  Runs *before* GenerateTransition
    /// so a command issued on top of a NonInterruptable current element
    /// skips the transition check entirely and either postpones the new
    /// command or rejects a MOVE issued before a freshly-instructed
    /// PASS_DOOR has executed. Returns `true` when the guard consumed the
    /// element (caller should skip generate_transition + arbitrate);
    /// `false` otherwise.
    fn non_interruptable_guard(
        &mut self,
        owner: EntityId,
        new_seq: crate::sequence::SequenceId,
        new_idx: usize,
    ) -> bool {
        use crate::element::Command;
        use crate::sequence::SequencePriority;

        // RHElementActorPC::Instruct owns these early-return commands before
        // it delegates to the base Actor method containing this guard.
        if self.pc_instruct_early_completion(owner, new_seq, new_idx) {
            return true;
        }

        let Some((cur_seq, cur_idx)) = self.current_sequence_element_for_actor(owner) else {
            return false;
        };
        let Some(cur_elem) = self.orders.sequence_manager.get_element(cur_seq, cur_idx) else {
            return false;
        };
        if cur_elem.priority != SequencePriority::NonInterruptable {
            return false;
        }
        let cur_command = cur_elem.command;
        let cur_started = self
            .get_entity(owner)
            .and_then(|e| e.actor_data())
            .map(|a| a.sequence_element_started)
            .unwrap_or(false);

        // Ensure new element has a resolved priority before postponing.
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(new_seq, new_idx)
            && elem.priority == SequencePriority::NotYetSet
        {
            let resolver = Self::priority_resolver(&self.world.entities);
            elem.priority = resolver(elem);
        }

        let new_command = self
            .orders
            .sequence_manager
            .get_element(new_seq, new_idx)
            .map(|e| e.command)
            .unwrap_or(Command::Null);

        if cur_started && cur_command == Command::PassDoor && new_command == Command::Move {
            // The move will be invalid after this newly-instructed door
            // pass executes. Once Execute has run, the lifecycle flag is
            // cleared and later moves are postponed normally.
            self.orders
                .sequence_manager
                .element_impossible(new_seq, new_idx);
        } else {
            // `new.Postpone(current)` — current is the blocker, new is
            // the waiter.
            self.engine_postpone(cur_seq, cur_idx, new_seq, new_idx);
        }
        true
    }

    /// Launch a 1-frame idle `Command::Wait` owned element at
    /// `SequencePriority::Wait`.  Used to park an actor in idle after
    /// a cross-entity state change (drop corpse, post-tie, post-combat)
    /// so its AI re-enters the default loop instead of continuing the
    /// pre-event command.
    pub(crate) fn actor_wait(&mut self, owner: EntityId) -> crate::sequence::SequenceId {
        let mut wait_elem =
            crate::sequence::SequenceElement::new(1, crate::element::Command::Wait, Some(owner));
        wait_elem.priority = crate::sequence::SequencePriority::Wait;
        self.launch_element(wait_elem)
    }

    /// Freeze an actor's execution and cascade-interrupt the
    /// currently-executing element.  Sets `execution_frozen = true`,
    /// then if the actor has a current sequence element, sets that
    /// element's state to `Interrupted` with `NEXT_LEVEL` cascade so a
    /// postponed successor can resume after the freeze lifts.
    ///
    /// Callers previously wrote `actor.execution_frozen = true` by hand,
    /// which left any in-progress element in `InProgress` state; when
    /// the freeze was later cleared, the animation driver re-read a
    /// stale InProgress element instead of the postponed successor.
    pub(crate) fn actor_freeze_execution(&mut self, owner: EntityId) {
        use crate::sequence::CascadeFlags;

        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.execution_frozen = true;
        }
        if let Some((cur_seq, cur_idx)) = self.current_sequence_element_for_actor(owner) {
            // Movement-element interrupt runs `MaybeCancelPathRequest`
            // before delegating to the base SetState.  Without this, a
            // `MoveWaiting` element's pathfinder request and
            // failed-path retry entry leak past the freeze, and the
            // 100-frame retry queue can fire `element_impossible` /
            // hero-speech on an actor that has been frozen / killed.
            self.stop_owner_active_mechanics(owner);
            self.orders.sequence_manager.element_interrupted(
                cur_seq,
                cur_idx,
                CascadeFlags::NEXT_LEVEL,
            );
        }
    }

    /// Drain deferred hero-speech triggers queued from
    /// `arbitrate_instruct`.  The speech is fired as soon as the
    /// Instruct-equivalent completes for `SpeakHeroReachDestination` /
    /// `SpeakVipsAreForRobin`, but the arbitrate path doesn't carry
    /// `&LevelAssets`, so we accumulate and drain here (called at the
    /// top of `perform_hourglass` alongside the other `drain_pending_*`
    /// helpers).
    pub(crate) fn drain_pending_hero_speeches(&mut self, assets: &crate::engine::LevelAssets) {
        let queued = std::mem::take(&mut self.orders.pending_hero_speeches);
        for (pc_id, expression) in queued {
            self.hero_speaking(assets, pc_id, expression);
        }
    }

    /// Run the PC `Instruct` arms which return before delegating to
    /// `RHElementActorHuman::Instruct` / `RHElementActor::Instruct`.
    ///
    /// Arrival speech must terminate here before base Actor's
    /// non-interruptable-current guard can postpone it. Otherwise parallel
    /// movement can win the postponed-chain priority comparison, abandon the
    /// speech, and cascade `Impossible` into later posture recovery work.
    fn pc_instruct_early_completion(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> bool {
        if !self.get_entity(owner).is_some_and(Entity::is_pc) {
            return false;
        }
        let command = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|element| element.command);
        let expression = match command {
            Some(crate::element::Command::SpeakHeroReachDestination) => {
                crate::engine::melee::HERO_DONE_COMMAND
            }
            Some(crate::element::Command::SpeakVipsAreForRobin) => {
                crate::engine::melee::HERO_PROVOKE_VIP
            }
            _ => return false,
        };
        self.orders
            .sequence_manager
            .element_terminated(seq_id, elem_idx);
        self.orders.pending_hero_speeches.push((owner, expression));
        true
    }

    /// Register a prebuilt sequence for the manager hourglass.
    ///
    /// Original `RHSequenceManager::LaunchSequence` does not call
    /// `DeterminePriority`; owned elements resolve it later at their ordered
    /// `Instruct` boundary. Keeping `NotYetSet` here also preserves dynamic
    /// Wait priority when the owner dies or becomes unconscious between
    /// registration and dispatch.
    pub(crate) fn launch_sequence(
        &mut self,
        seq: crate::sequence::Sequence,
    ) -> crate::sequence::SequenceId {
        self.orders.sequence_manager.launch_sequence(seq)
    }

    /// Find the actor's currently-executing sequence element.  An
    /// actor's "current" element is the single `InProgress`-state
    /// element owned by that actor; priority-based arbitration in
    /// `Instruct` maintains the one-at-a-time invariant across all
    /// sequences.
    ///
    /// Returns `None` when the actor is idle (no in-progress element).
    fn current_sequence_element_for_actor(
        &self,
        actor: EntityId,
    ) -> Option<(crate::sequence::SequenceId, usize)> {
        self.orders
            .sequence_manager
            .current_element_for_actor(actor)
    }

    /// Arbitrate a new sequence-element dispatch against the actor's
    /// currently-executing element.
    ///
    /// Called synchronously from [`Self::launch_element_for_owner`] (the
    /// default launch path for owned elements) so arbitration fires
    /// inline with the launch.  Also called idempotently from the
    /// hourglass pre-pass as a safety net for any owned element that
    /// might slip through an un-refactored code path.
    /// The four outcomes:
    ///
    /// - [`PriorityDecision::Abandon`]: the new element becomes
    ///   `Impossible`.  Caller skips the dispatch entirely.
    /// - [`PriorityDecision::Postpone`]: the new element waits behind
    ///   the current one (state → `Postponed`, linked via
    ///   `cross_postponed`).  Caller skips the dispatch.
    /// - [`PriorityDecision::PostponeCurrent`]: the current element
    ///   gets postponed behind the new one, and the new one proceeds.
    /// - [`PriorityDecision::InterruptCurrent`]: the current element is
    ///   marked `Interrupted` (cascades via `set_element_state`), and
    ///   the new one proceeds.
    ///
    /// Returns `true` if the caller should proceed to dispatch the new
    /// element; `false` if it was abandoned or postponed.
    pub(crate) fn arbitrate_instruct(
        &mut self,
        new_seq: crate::sequence::SequenceId,
        new_idx: usize,
    ) -> bool {
        use crate::element::Command;
        use crate::sequence::{PriorityDecision, SequenceState};

        let Some(new_elem) = self.orders.sequence_manager.get_element(new_seq, new_idx) else {
            return false;
        };
        let Some(owner) = new_elem.owner else {
            // No owner: nothing to arbitrate against, let it through.
            return true;
        };
        // Idempotency guard.  Owned launches now arbitrate
        // synchronously inside `launch_element_for_owner`, but legacy
        // callsites that explicitly arbitrate after an owned launch still hit
        // `arbitrate_instruct` explicitly after `launch_element`.  The
        // second call must be a safe no-op: if the first call already
        // resolved the element, return the matching bool without
        // repeating the decision (which would double-postpone / double-
        // interrupt on cascading priorities).
        match new_elem.state {
            SequenceState::Todo => { /* fall through — normal case */ }
            SequenceState::InProgress => {
                // Element is already the actor's current (e.g.
                // `launch_single_order_sequence_stamped` promoted it
                // after arbitration). Accept it without comparing the
                // element against itself: a postponed element can retain its
                // original manager registration and gain a second one when
                // its blocker releases it, so duplicate InstructOwner
                // actions are possible in the same drain.
                if self.current_sequence_element_for_actor(owner) == Some((new_seq, new_idx)) {
                    return true;
                }
            }
            SequenceState::Impossible
            | SequenceState::Postponed
            | SequenceState::Interrupted
            | SequenceState::Terminated
            | SequenceState::Done => {
                tracing::trace!(
                    ?owner,
                    ?new_seq,
                    new_idx,
                    command = ?new_elem.command,
                    state = ?new_elem.state,
                    "arbitrate_instruct skipped a non-pending element"
                );
                return false;
            }
        }
        let new_priority = new_elem.priority;
        let new_command = new_elem.command;

        // Every recipient of `Instruct` is unconditionally unfrozen
        // before the arbitration / dispatch logic runs.  Without this
        // clear, a freeze imposed via paths other than `DropDone`
        // (which clears it inline) would persist past the next Instruct.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.execution_frozen = false;
        }

        // The posture / action-state stamp now runs at *launch* time
        // via `launch_element_for_owner` and the stamped
        // single-order-sequence wrapper, synchronous with the
        // launch → Instruct flow.  By the time arbitration runs, the
        // stamp is already on the element.

        // ── Subclass Instruct overrides ─────────────────────────
        //
        // Civilian Instruct refuses everything except RECEIVE_PURSE /
        // BEGGAR_SHOW_FACE / WAIT when the civilian is a beggar.
        if self.beggar_rejects_command(owner, new_command) {
            self.orders
                .sequence_manager
                .element_impossible(new_seq, new_idx);
            return false;
        }

        // Some direct callers enter arbitration without the ordinary
        // base-Actor admission wrapper. Preserve the PC derived-class early
        // return for those paths as well.
        if self.pc_instruct_early_completion(owner, new_seq, new_idx) {
            return false;
        }

        // PC Instruct intercepts the remaining commands before falling
        // through to the Human path.
        if let Some(entity) = self.get_entity(owner)
            && entity.is_pc()
        {
            match new_command {
                // CROUCH_UP / CROUCH_DOWN: reject when swordfighting.
                // When the PC is doing a non-movement sequence element,
                // first Stop(PREFERENCE) so the posture change can take
                // over cleanly.
                Command::CrouchUp | Command::CrouchDown => {
                    let swordfighting =
                        entity.human_data().is_some_and(|h| !h.opponents.is_empty());
                    if swordfighting {
                        // Forward `MSG_STATURE_CHANGE_END` so the
                        // stature-HUD latch (focus standing-up /
                        // crouching-down) clears even though the command
                        // is being rejected.  Without this the stature
                        // arrow stays visually pressed until some other
                        // actor's stature changes.
                        self.orders.messenger.send(crate::messenger::Message::new(
                            crate::messenger::MessageType::Simple(
                                crate::messenger::SimpleMessage::StatureChangeEnd,
                            ),
                        ));
                        self.orders
                            .sequence_manager
                            .element_impossible(new_seq, new_idx);
                        return false;
                    }
                    // `is_part_of_movement` covers
                    // Move/MoveOk/Seek/PassDoor/Jump/AssertPosition;
                    // use it instead of `data.is_movement()` (which only
                    // covers the `Movement` data variant —
                    // Move/MoveOk/Seek/PassDoor) so a mid-Jump or
                    // mid-AssertPosition crouch toggle doesn't trigger
                    // a spurious `Stop(PREFERENCE)`.
                    let cur_is_movement = self
                        .current_sequence_element_for_actor(owner)
                        .and_then(|(s, i)| self.orders.sequence_manager.get_element(s, i))
                        .map(|e| e.command.is_part_of_movement())
                        .unwrap_or(true);
                    if !cur_is_movement {
                        self.stop_owner(owner, crate::sequence::SequencePriority::Preference);
                    }
                }
                _ => {}
            }
        }

        // Human Instruct refuses almost every command when
        // dead / unconscious / stuck under a net — only damage-receive
        // / WAIT / GET_KILLED_AT_BOTTOM (and RECEIVE_NET at a specific
        // stuck-counter) go through.
        if let Some(entity) = self.get_entity(owner)
            && entity.is_human()
        {
            let is_dead = entity.is_dead();
            let is_unconscious = entity.human_data().is_some_and(|h| h.unconscious);
            let stuck_ctr = entity
                .human_data()
                .map(|h| h.stuck_under_nets_counter)
                .unwrap_or(0);
            let is_stuck_under_net = stuck_ctr > 0;
            if is_dead || is_unconscious || is_stuck_under_net {
                let allowed = matches!(
                    new_command,
                    Command::ReceiveHitDamage
                        | Command::ReceiveSwordDamage
                        | Command::ReceiveArrowDamage
                        | Command::ReceiveDamage
                        | Command::ReceiveMobileDamage
                        | Command::Wait
                        | Command::GetKilledAtBottom
                ) || (new_command == Command::ReceiveNet
                    && !is_dead
                    && !is_unconscious
                    && stuck_ctr == 1);
                if !allowed {
                    self.orders
                        .sequence_manager
                        .element_impossible(new_seq, new_idx);
                    return false;
                }
            }
        }

        let Some((cur_seq, cur_idx)) = self.current_sequence_element_for_actor(owner) else {
            // Idle actor — new element takes over unconditionally.
            return true;
        };

        let cur_priority = self
            .orders
            .sequence_manager
            .get_element(cur_seq, cur_idx)
            .map(|e| e.priority)
            .unwrap_or(crate::sequence::SequencePriority::None);

        let decision = crate::sequence::decide_priorities(cur_priority, new_priority);

        tracing::trace!(
            ?owner,
            ?cur_seq,
            cur_idx,
            ?cur_priority,
            ?new_seq,
            new_idx,
            ?new_priority,
            ?decision,
            "arbitrate_instruct"
        );

        match decision {
            PriorityDecision::Abandon => {
                // Hand the new element's postponed successor (if any)
                // over to the current element before marking new
                // Impossible, so the successor doesn't get orphaned.
                self.orders
                    .sequence_manager
                    .take_over_postponed(cur_seq, cur_idx, new_seq, new_idx);
                self.orders
                    .sequence_manager
                    .element_impossible(new_seq, new_idx);
                false
            }
            PriorityDecision::Postpone => {
                // `new.Postpone(current)` — may recurse when the target
                // already has a postponed chain.
                self.engine_postpone(cur_seq, cur_idx, new_seq, new_idx);
                false
            }
            PriorityDecision::PostponeCurrent => {
                assert!(
                    self.orders
                        .sequence_manager
                        .can_interrupt_now(cur_seq, cur_idx),
                    "Original CanInterruptNow is unconditional"
                );
                // `current.Postpone(new)` — postpone current behind new.
                // Current is in-progress, so we first tear down its
                // active machinery before flipping it to Postponed.
                self.stop_owner_active_mechanics(owner);
                self.engine_postpone(new_seq, new_idx, cur_seq, cur_idx);
                true
            }
            PriorityDecision::InterruptCurrent => {
                assert!(
                    self.orders
                        .sequence_manager
                        .can_interrupt_now(cur_seq, cur_idx),
                    "Original CanInterruptNow is unconditional"
                );
                // In Original, Instruct installs the incoming Turn as
                // `mpSequenceElement` before interrupting the outgoing
                // movement. Its synchronous condolence therefore sees
                // that it is no longer selected and leaves the sprite's
                // movement goal intact. Rust clears active mechanics
                // before the incoming element begins executing, so carry
                // that selected-owner fact explicitly to the Turn.
                if matches!(new_command, Command::Turn | Command::TurnFast)
                    && self
                        .orders
                        .sequence_manager
                        .get_element(cur_seq, cur_idx)
                        .is_some_and(|element| element.data.is_movement())
                {
                    let retained_goal = self
                        .world
                        .entities
                        .get(owner)
                        .map(|entity| entity.position_iface().map_goal());
                    if let (Some(goal), Some(element)) = (
                        retained_goal,
                        self.orders
                            .sequence_manager
                            .get_element_mut(new_seq, new_idx),
                    ) && element
                        .get_property(crate::sequence::Field::RetainedMovementGoal)
                        .is_none()
                    {
                        element.set_property(
                            crate::sequence::Field::RetainedMovementGoal,
                            crate::sequence::FieldValue::GeoPoint2D {
                                x: goal.x,
                                y: goal.y,
                            },
                        );
                    }
                }
                // New takes over current's postponed chain, current
                // becomes Interrupted.
                self.orders
                    .sequence_manager
                    .take_over_postponed(new_seq, new_idx, cur_seq, cur_idx);
                self.stop_owner_active_mechanics(owner);
                self.orders
                    .sequence_manager
                    .element_interrupted_after_replacement_selected(
                        cur_seq,
                        cur_idx,
                        crate::sequence::CascadeFlags::NEXT_LEVEL,
                    );
                true
            }
        }
    }

    /// Walk every actor whose sprite reported `MotionState::Done` this
    /// tick and flip `done = true` on the actor's currently-dispatched
    /// order, then clear `last_motion_state` on every sprite so the
    /// field is fresh for the next tick.
    ///
    /// Sprite advancement is split across the live owner coordinator and
    /// the remaining specialized arms (`tick_actor_animation_for`, active
    /// jumps, melee, bow, and abilities); each one funnels through
    /// [`Sprite::record_motion_state`](crate::sprite::Sprite), which
    /// stashes the result in [`Sprite::last_motion_state`].  This pass
    /// runs once per frame after every per-system tick has completed,
    /// recovering the "single Done observer" semantics without forcing
    /// each per-system tick to know about the order-completion flag.
    ///
    /// The corresponding read site is the postpone-race guard in
    /// [`Self::engine_postpone`]: when a postpone target's last order
    /// is already `done`, the postpone short-circuits to TERMINATED
    /// instead of installing the cross-element link.
    pub(super) fn propagate_done_to_current_orders(&mut self) {
        let done_actors: Vec<(crate::element::EntityId, u32)> = self
            .world
            .entities
            .actors()
            .filter_map(|(entity_id, entity)| {
                matches!(
                    entity.element_data().sprite.last_motion_state,
                    Some(crate::sprite::MotionState::Done)
                )
                .then_some((
                    entity_id.into(),
                    entity.element_data().sprite.last_processed_order_id,
                ))
            })
            .collect();

        for (entity_id, processed_order_id) in done_actors {
            let Some((seq_id, elem_idx)) = self
                .orders
                .sequence_manager
                .current_element_for_actor(entity_id)
            else {
                continue;
            };
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
                && let Some(order) = elem.orders.front_mut()
                && order.order_id.get() == processed_order_id
            {
                order.done = true;
            }
        }

        // Reset every sprite's transient last_motion_state so the next
        // tick starts clean, regardless of whether the slot was an
        // actor or had an order to mark.
        for (_, entity) in self.world.entities.occupied_mut() {
            entity.element_data_mut().sprite.last_motion_state = None;
        }
    }

    /// Postpone element `waiter` behind element `blocker` on the same
    /// actor.  When the blocker already has a postponed successor,
    /// arbitrate between the existing successor and the new waiter —
    /// may recurse, swap, or interrupt deeper in the chain.
    fn engine_postpone(
        &mut self,
        blocker_seq: crate::sequence::SequenceId,
        blocker_idx: usize,
        waiter_seq: crate::sequence::SequenceId,
        waiter_idx: usize,
    ) {
        self.engine_postpone_with_debug_depth(blocker_seq, blocker_idx, waiter_seq, waiter_idx, 0);
    }

    fn engine_postpone_with_debug_depth(
        &mut self,
        blocker_seq: crate::sequence::SequenceId,
        blocker_idx: usize,
        waiter_seq: crate::sequence::SequenceId,
        waiter_idx: usize,
        depth: usize,
    ) {
        use crate::sequence::PriorityDecision;

        assert_ne!(
            (blocker_seq, blocker_idx),
            (waiter_seq, waiter_idx),
            "engine_postpone cannot postpone a sequence element behind itself"
        );

        let parity_debug_stage_timing =
            std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some() && depth <= 64;
        if parity_debug_stage_timing {
            eprintln!(
                "parity launch: engine_postpone enter depth={depth} blocker=({blocker_seq:?},{blocker_idx}) waiter=({waiter_seq:?},{waiter_idx})"
            );
        }

        // If blocker already has a postponed successor, arbitrate
        // between that existing successor and the new waiter.
        let existing_postponed = self
            .orders
            .sequence_manager
            .get_element(blocker_seq, blocker_idx)
            .and_then(|e| e.cross_postponed);
        if let Some((existing_seq, existing_idx)) = existing_postponed {
            if parity_debug_stage_timing {
                eprintln!(
                    "parity launch: engine_postpone existing depth={depth} blocker=({blocker_seq:?},{blocker_idx}) existing=({existing_seq:?},{existing_idx})"
                );
            }
            let existing_priority = self
                .orders
                .sequence_manager
                .get_element(existing_seq, existing_idx)
                .map(|e| e.priority)
                .unwrap_or(crate::sequence::SequencePriority::None);
            let waiter_priority = self
                .orders
                .sequence_manager
                .get_element(waiter_seq, waiter_idx)
                .map(|e| e.priority)
                .unwrap_or(crate::sequence::SequencePriority::None);

            let decision = crate::sequence::decide_priorities(existing_priority, waiter_priority);
            match decision {
                PriorityDecision::Abandon => {
                    // existing wins — take over waiter's postponed
                    // chain and abandon waiter.
                    self.orders.sequence_manager.take_over_postponed(
                        existing_seq,
                        existing_idx,
                        waiter_seq,
                        waiter_idx,
                    );
                    self.orders
                        .sequence_manager
                        .element_impossible(waiter_seq, waiter_idx);
                    return;
                }
                PriorityDecision::Postpone => {
                    // waiter queues behind existing — recurse.
                    self.engine_postpone_with_debug_depth(
                        existing_seq,
                        existing_idx,
                        waiter_seq,
                        waiter_idx,
                        depth + 1,
                    );
                    return;
                }
                PriorityDecision::PostponeCurrent => {
                    // existing becomes postponed behind waiter.  Keep
                    // blocker→waiter link (set below after the fall-
                    // through) and install existing behind waiter.
                    // First detach existing from blocker's slot so we
                    // don't leave a dangling link while recursing.
                    if let Some(b) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(blocker_seq, blocker_idx)
                    {
                        b.cross_postponed = None;
                    }
                    self.engine_postpone_with_debug_depth(
                        waiter_seq,
                        waiter_idx,
                        existing_seq,
                        existing_idx,
                        depth + 1,
                    );
                    // Fall through to install waiter in blocker's slot.
                }
                PriorityDecision::InterruptCurrent => {
                    // waiter inherits existing's postponed chain;
                    // existing becomes Interrupted.  Then install
                    // waiter in blocker's slot.
                    self.orders.sequence_manager.take_over_postponed(
                        waiter_seq,
                        waiter_idx,
                        existing_seq,
                        existing_idx,
                    );
                    self.orders.sequence_manager.element_interrupted(
                        existing_seq,
                        existing_idx,
                        crate::sequence::CascadeFlags::NEXT_LEVEL,
                    );
                    if let Some(b) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(blocker_seq, blocker_idx)
                    {
                        b.cross_postponed = None;
                    }
                }
            }
        }

        // When the waiter already has orders and its last order is
        // done, just terminate it instead of postponing.  Otherwise
        // install it in the blocker's postponed slot.
        let should_terminate_instead = self
            .orders
            .sequence_manager
            .get_element(waiter_seq, waiter_idx)
            .map(|e| {
                e.command != crate::element::Command::MoveOk
                    && e.orders.back().is_some_and(|o| o.done)
            })
            .unwrap_or(false);

        if should_terminate_instead {
            if let Some(e) = self
                .orders
                .sequence_manager
                .get_element_mut(waiter_seq, waiter_idx)
            {
                e.orders.clear();
            }
            self.orders
                .sequence_manager
                .element_terminated(waiter_seq, waiter_idx);
            return;
        }

        if let Some(b) = self
            .orders
            .sequence_manager
            .get_element_mut(blocker_seq, blocker_idx)
        {
            b.cross_postponed = Some((waiter_seq, waiter_idx));
        }

        // RHSequenceElementMovement::SetState(RHSEQ_POSTPONED) restores a
        // translated movement element to its untranslated command before the
        // common sequence-state transition runs. A resumed element is sent
        // through Instruct/Translate again, so retaining MoveWaiting or MoveOk
        // here would either strand the old pathfinder state or bypass path
        // translation entirely.
        //
        // Original provenance:
        // original-code/RHSequenceElementMovement.cpp:1395-1426.
        let postponed_movement = self
            .orders
            .sequence_manager
            .get_element(waiter_seq, waiter_idx)
            .and_then(|element| {
                matches!(
                    element.command,
                    crate::element::Command::MoveWaiting | crate::element::Command::MoveOk
                )
                .then_some((element.owner, element.command))
            });
        if let Some((owner, command)) = postponed_movement {
            if command == crate::element::Command::MoveWaiting {
                let owner = owner.unwrap_or_else(|| {
                    panic!(
                        "MoveWaiting element {waiter_seq:?}[{waiter_idx}] has no actor owner while being postponed"
                    )
                });
                self.world.pathfinder.cancel_requests_for(owner);
                self.orders.pending_path_requests.cancel_for_owner(owner);
                self.orders
                    .failed_path_requests
                    .retain(|request| request.owner != owner);
            }
            self.orders
                .sequence_manager
                .get_element_mut(waiter_seq, waiter_idx)
                .expect("postponed movement element disappeared")
                .command = crate::element::Command::Move;
        }

        if let Some(w) = self
            .orders
            .sequence_manager
            .get_element_mut(waiter_seq, waiter_idx)
        {
            w.orders.clear();
        }
        self.orders
            .sequence_manager
            .postpone_element(waiter_seq, waiter_idx);
        if parity_debug_stage_timing {
            eprintln!(
                "parity launch: engine_postpone exit depth={depth} blocker=({blocker_seq:?},{blocker_idx}) waiter=({waiter_seq:?},{waiter_idx})"
            );
        }
    }

    /// Cancel any active pathfinder request / active-movement / active-
    /// melee on `owner`, used when arbitration interrupts or postpones
    /// the actor's current element.  Subset of the StopMovement /
    /// MaybeCancelPathRequest cleanup we need before a state
    /// transition.
    fn stop_owner_active_mechanics(&mut self, owner: EntityId) {
        self.world.pathfinder.cancel_requests_for(owner);
        self.orders.pending_path_requests.cancel_for_owner(owner);
        // `MaybeCancelPathRequest` fires from both
        // `SetState(Interrupted)` *and* `SetState(Postponed)`, and
        // drops stale retry entries for the actor.  Mirror that here so
        // cross-postpone (higher-priority blocker) also evicts pending
        // failed-path retries — otherwise the entry would stay in the
        // queue until the element resumes or times out.
        self.orders
            .failed_path_requests
            .retain(|r| r.owner != owner);
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.active_movement.clear();
            // Original's lateral/circle victim list and angles are
            // human-owned members, not sequence-owned state. They survive an
            // interrupted strike and are cleared only when a sweep genuinely
            // terminates or a later action-done point reinitializes them.
            actor.pending_push_swordfight.clear();
            // Order-chain cleanup happens implicitly: interrupted
            // elements drop their `orders` in `Sequence::set_element_state`,
            // which invalidates `current_order_for_actor`.  Non-
            // interruptable elements (dying / corpse idle / rolling)
            // keep running — arbitration prevents the interrupt
            // dispatch from reaching them.
        }
    }

    /// Stop all active / pending sequence elements owned by `owner`,
    /// rewriting any in-progress movement element's current order to
    /// the matching waiting-transition animation (shortened to ~10
    /// units) and cancelling pending pathfinder requests.
    ///
    /// This is the full `Stop()` path — combining the actor stop, the
    /// sequence-manager not-yet-launched stop, the movement-element
    /// StopMovement, and MaybeCancelPathRequest.  Callers that
    /// previously invoked `self.orders.sequence_manager.stop_owner(...)`
    /// directly should use this wrapper so the actor's movement doesn't
    /// keep running on a stale path.
    pub(crate) fn stop_owner(
        &mut self,
        owner: EntityId,
        stop_priority: crate::sequence::SequencePriority,
    ) {
        let parity_debug_stage_timing = std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some();
        if parity_debug_stage_timing {
            eprintln!(
                "parity stop: engine stop_owner enter owner={owner:?} priority={stop_priority:?}"
            );
        }
        let owner_pos = self
            .get_entity(owner)
            .map(|e| e.element_data().position_map())
            .unwrap_or_default();
        if tracing::enabled!(target: "parity_owner_handoff", tracing::Level::TRACE) {
            let selected = self
                .orders
                .sequence_manager
                .current_element_for_actor(owner);
            let selected_state = selected.and_then(|(seq_id, elem_idx)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| {
                        (
                            element.command,
                            element.state,
                            element.priority,
                            element
                                .orders
                                .front()
                                .map(|order| (order.order_type, order.order_id)),
                        )
                    })
            });
            let (active_movement, goal) = self
                .get_entity(owner)
                .map(|entity| {
                    let active_movement = entity.actor_data().map(|actor| {
                        (
                            actor.active_movement.sequence_id,
                            actor.active_movement.element_index,
                        )
                    });
                    (active_movement, entity.position_iface().map_goal())
                })
                .unwrap_or_default();
            tracing::trace!(
                target: "parity_owner_handoff",
                frame = self.control.frame_counter,
                ?owner,
                ?stop_priority,
                ?selected,
                ?selected_state,
                ?active_movement,
                ?goal,
                "stop_owner before movement and sequence stop"
            );
        }
        let pathfinder = &mut self.world.pathfinder;
        let next_order_id = &mut self.orders.next_order_id;
        let resolver = Self::priority_resolver(&self.world.entities);
        let selected_movement_before_stop = self
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .map(|(seq_id, elem_idx, order)| (seq_id, elem_idx, order.order_id));
        if parity_debug_stage_timing {
            eprintln!("parity stop: before stop_movement_for_owner owner={owner:?}");
        }
        self.orders.sequence_manager.stop_movement_for_owner(
            owner,
            owner_pos,
            stop_priority,
            &resolver,
            next_order_id,
            &mut |id| {
                pathfinder.cancel_requests_for(id);
            },
        );
        let rewritten_selected_order = if let Some((before_seq, before_idx, before_id)) =
            selected_movement_before_stop
            && let Some((after_seq, after_idx, after_order)) =
                self.orders.sequence_manager.current_order_for_actor(owner)
            && after_seq == before_seq
            && after_idx == before_idx
            && after_order.order_id != before_id
        {
            // RHSequenceElementMovement::StopMovement mutates the first
            // RHOrder's action and calls NewID in place. mpOrder still points
            // at that same object, so update Rust's explicit pointer mirror
            // only when the selected element survived with a rewritten ID.
            Some(crate::element::InstalledActorOrder {
                order_id: after_order.order_id,
                order_type: after_order.order_type,
            })
        } else {
            None
        };
        if parity_debug_stage_timing {
            eprintln!("parity stop: after stop_movement_for_owner owner={owner:?}");
        }
        // `MaybeCancelPathRequest` pairs path-request cancellation with
        // failed-path-retry removal whenever a movement element
        // transitions out of MOVE_WAITING.  Mirror that here so a
        // `stop_owner` tear-down also evicts any stale retry entries
        // for this actor — otherwise the 100-frame timeout would fire
        // `element_impossible` / hero-speech on a sequence that no
        // longer cares.
        self.orders
            .failed_path_requests
            .retain(|r| r.owner != owner);
        self.orders.pending_path_requests.cancel_for_owner(owner);
        if parity_debug_stage_timing {
            eprintln!("parity stop: before sequence stop_owner owner={owner:?}");
        }
        self.orders
            .sequence_manager
            .stop_owner(owner, stop_priority, &resolver);
        drop(resolver);
        if let Some(installed_order) = rewritten_selected_order {
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("rewritten StopMovement owner lost actor data")
                .installed_order = Some(installed_order);
        }
        if parity_debug_stage_timing {
            eprintln!("parity stop: after sequence stop_owner owner={owner:?}");
            eprintln!(
                "parity stop: engine stop_owner exit owner={owner:?} priority={stop_priority:?}"
            );
        }
    }

    /// Returns `true` when the actor's posture is one of
    /// `Flying / OnLadder / OnWall`, or when the actor's currently
    /// in-progress sequence element is a `PassDoor` or `Fall` command.
    /// An actor in either state cannot accept a fresh AI movement
    /// order without tearing down the in-flight posture-transition or
    /// door-pass sequence, so the engine holds `AILOCK_BUSY` for the
    /// duration via the per-tick edge detector in
    /// [`Self::tick_npc_busy_edge_detect_for_npc`].
    pub fn is_very_very_busy(&self, owner: EntityId) -> bool {
        use crate::element::Posture;
        let Some(entity) = self.get_entity(owner) else {
            return false;
        };
        let posture = entity.element_data().posture;
        if matches!(
            posture,
            Posture::Flying | Posture::OnLadder | Posture::OnWall
        ) {
            return true;
        }
        self.orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(sid, eidx)| self.orders.sequence_manager.get_element(sid, eidx))
            .is_some_and(|el| {
                matches!(
                    el.command,
                    crate::element::Command::PassDoor | crate::element::Command::Fall
                )
            })
    }

    /// Per-tick AILOCK_BUSY edge detector for every NPC.
    ///
    /// ```text
    /// if  !was_busy && is_very_very_busy()  → non_script_lock(BUSY)
    /// elif was_busy && !is_very_very_busy() → non_script_unlock(BUSY)
    /// was_busy = is_very_very_busy()
    /// ```
    ///
    /// The `was_busy = true` writes inside
    /// [`Self::soldier_helpers`]'s `EventCouldntReachPoint` arm and
    /// inside `ai_friendly::return_to_duty` are *one-way* sets — there
    /// is no symmetric unlock.  Without this scan an NPC that crossed
    /// into the busy gate via either site would stay locked forever.
    /// The per-tick edge detect closes the loop and also covers the
    /// `Command::PassDoor | Command::Fall` arm of `is_very_very_busy`,
    /// which neither caller checks.
    pub(super) fn tick_npc_busy_edge_detect_for_npc(&mut self, npc_id: EntityId) {
        let busy = self.is_very_very_busy(npc_id);
        let entity = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| panic!("busy-edge NPC {} disappeared", npc_id.index()));
        let ai = entity
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("busy-edge NPC {} has no AI controller", npc_id.index()));
        if !ai.was_busy && busy {
            ai.non_script_lock(crate::ai::AiLockFlags::BUSY);
        } else if ai.was_busy && !busy {
            ai.non_script_unlock(crate::ai::AiLockFlags::BUSY);
        }
        ai.was_busy = busy;
    }

    /// Launch the actor's pending post-seek sequence, if any.  Stops a
    /// PC seek target, clears the seek-target field, terminates the
    /// seek element, and launches the stored sequence at info priority.
    pub(crate) fn start_post_seek_sequence(
        &mut self,
        owner: EntityId,
        seek_element: Option<(crate::sequence::SequenceId, usize)>,
    ) -> bool {
        let (target, post_seek) = {
            let Some(entity) = self.get_entity_mut(owner) else {
                return false;
            };
            let Some(actor) = entity.actor_data_mut() else {
                return false;
            };
            let target = actor.seek_target;
            // Original retains the single overloaded `mulWaitTime` value
            // when StartPostSeekSequence replaces the seek with its
            // interaction. Fold Rust's seek-specific copy back into the
            // ordinary slot before dropping the state that identifies which
            // split field currently owns that legacy scalar.
            actor.wait_time = actor.seek_refresh_wait;
            actor.seek_target = None;
            (target, actor.post_seek_sequence.take())
        };
        let Some(post_seek) = post_seek else {
            return false;
        };

        // `SetState(Terminated)` synchronously calls the selected actor's
        // SendCondolationCard in Original, which clears the old movement goal
        // before `StartPostSeekSequence` launches the interaction. Rust
        // defers that callback to avoid a re-entrant borrow, so perform this
        // selected-seek cleanup at the same handoff boundary.
        if seek_element.is_some() {
            self.get_entity_mut(owner)
                .unwrap_or_else(|| panic!("post-seek owner {owner:?} disappeared"))
                .position_iface_mut()
                .set_map_goal(crate::coordinates::MapPoint::ZERO);
        }

        if let Some(target_id) = target
            && self.get_entity(target_id).is_some_and(|e| e.is_pc())
        {
            self.stop_owner(target_id, crate::sequence::SequencePriority::Normal);
        }
        if let Some((seq_id, elem_idx)) = seek_element {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }
        self.launch_sequence(*post_seek);
        true
    }

    /// Halt an NPC: stop the actor at `Preference` priority while
    /// flagging that the stop cascade is happening "inside Halt".
    ///
    /// ```text
    /// inside_halt_method = true;
    /// stop_owner(Preference);
    /// inside_halt_method = false;
    /// ```
    ///
    /// Sets `AiController::inside_halt_method` on the target NPC and
    /// flips the sequence manager's `halt_pending` marker while the
    /// `stop_owner(Preference)` cascade runs, so any `PendingCondolation`
    /// queued while the sequence is being torn down is tagged
    /// `from_halt=true`. The downstream `SendCondolationCard` handler
    /// checks that tag to suppress the `Think(EVENT_DONE)` /
    /// `Think(EVENT_IMPOSSIBLE)` / `Think(EVENT_COULDNT_REACHPOINT)`
    /// dispatches that should not fire from a halt.
    ///
    /// Called from the AI-order drain in
    /// [`EngineInner::process_pending_ai_orders`] whenever a movement
    /// order arrives without `GotoFlags::NO_HALT`.
    pub(crate) fn halt_actor(&mut self, owner: EntityId) {
        // Snapshot the actor-base selected element before Stop tears down the
        // sequence-manager identity. Original SendCondolationCard clears the
        // sprite goal synchronously when this exact selected element is
        // interrupted, regardless of whether it is a movement element. Rust
        // delivers that card later, by which point replacement work may
        // already be selected and obscure the relationship. This notably
        // matters when a second FaceTo halts a Turn whose front order is a
        // running-to-waiting transition.
        let selected_element = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .filter(|&(sequence, element)| {
                self.orders
                    .sequence_manager
                    .get_element(sequence, element)
                    .is_some_and(|element| {
                        element.state == crate::sequence::SequenceState::InProgress
                    })
            });

        if let Some(entity) = self.get_entity_mut(owner)
            && let Some(ai) = entity.ai_controller_mut()
        {
            ai.inside_halt_method = true;
        }
        self.orders.sequence_manager.set_halt_pending(true);

        self.stop_owner(owner, crate::sequence::SequencePriority::Preference);
        // Stop can retain the selected movement element by replacing its
        // current walking order with a transition-to-waiting order. Original
        // has not called Actor::SendCondolationCard in that case, so the
        // sprite's live movement goal remains owned by the retained element
        // until the transition finishes. Clear it synchronously only when the
        // stop actually detached the element selected before Halt. The same
        // test covers selected non-movement work, which Stop normally detaches
        // outright and whose Actor-base condolence clears the goal as well.
        let selected_element_was_detached = selected_element.is_some_and(|selected| {
            let remains_live = self
                .orders
                .sequence_manager
                .get_element(selected.0, selected.1)
                .is_some_and(|element| element.state == crate::sequence::SequenceState::InProgress);
            !remains_live
        });
        if selected_element_was_detached && let Some(entity) = self.get_entity_mut(owner) {
            entity
                .position_iface_mut()
                .set_map_goal(crate::coordinates::MapPoint::ZERO);
        }

        // `MaybeCancelPathRequest` fires from movement-element
        // interrupt.  When halt interrupts the actor's Move element,
        // any failed-path retry entry for that actor must be dropped —
        // otherwise the retry pass would try to pathfind for an
        // element that no longer exists (survives the `retryable`
        // guard only briefly, but eager cleanup avoids the one-tick lag
        // that could e.g. fire `HERO_UNABLE_TO_DO_SOMETHING` for a Move
        // the player already cancelled).  Also drops the pending intent
        // so a newly-arriving Move doesn't race with a stale enqueue.
        self.orders
            .failed_path_requests
            .retain(|r| r.owner != owner);
        // AI Move intents are the Rust pre-sequence form of Original
        // not-yet-launched Move elements. Halt/Stop(PREFERENCE) cancels those
        // elements before a later GoTo is registered; retaining an older
        // intent here would let the synchronous SetAIState barrier mistake it
        // for the causal SeekArea Move. Keep this at the Halt boundary: the
        // lower-level active-mechanics cleanup also serves postponement paths,
        // where deleting a not-yet-launched intent would be too broad.
        self.orders
            .pending_move_requests
            .retain(|(request_owner, _)| *request_owner != owner);

        self.orders.sequence_manager.set_halt_pending(false);
        if let Some(entity) = self.get_entity_mut(owner)
            && let Some(ai) = entity.ai_controller_mut()
        {
            ai.inside_halt_method = false;
        }
    }

    /// Apply only an AI controller's pending synchronous `StopAll` barrier.
    ///
    /// Some original AI routines stop existing work and then immediately
    /// launch replacement work inside one call. Their Rust counterparts must
    /// consume the halt before launching that replacement, without draining
    /// unrelated queued actor effects at the nested boundary.
    pub(crate) fn apply_pending_ai_halt(&mut self, owner: EntityId) {
        let take_halt = self
            .get_entity_mut(owner)
            .unwrap_or_else(|| panic!("pending-halt owner {} disappeared", owner.index()))
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("pending-halt owner {} has no AI controller", owner.index()))
            .outbox
            .actor
            .take_halt();
        if take_halt {
            self.halt_actor(owner);
        }
    }

    /// Launch a one-shot damage sequence; wraps
    /// [`Self::launch_sequence`] so the damage element's priority is
    /// resolved eagerly.
    pub(crate) fn launch_damage(
        &mut self,
        actor: EntityId,
        hp: u16,
        concussion: u16,
    ) -> crate::sequence::SequenceId {
        self.launch_sequence(crate::sequence::Sequence::single_damage(
            actor, hp, concussion,
        ))
    }

    // ─── Read-only accessors for host renderer / input ───────────

    /// Iterate over all live entities (skipping `None` slots).
    pub fn entities_iter(&self) -> impl Iterator<Item = &Entity> + '_ {
        self.world.entities.occupied().map(|(_, entity)| entity)
    }

    /// Iterate over all live entities together with their typed table IDs.
    ///
    /// Diagnostic tools that compare two independently-built simulations use
    /// this to construct an isomorphism between entity tables; callers must
    /// not assume that the returned IDs have meaning outside this engine.
    pub fn entities_with_ids_iter(&self) -> impl Iterator<Item = (EntityId, &Entity)> + '_ {
        self.world.entities.occupied()
    }

    /// Active entity positions for debug overlays.
    pub fn active_entity_positions(
        &self,
    ) -> impl Iterator<Item = (EntityId, crate::coordinates::MapPoint)> + '_ {
        self.world.entities.occupied().filter_map(|(id, entity)| {
            entity
                .is_active()
                .then_some((id, entity.element_data().position_map()))
        })
    }

    /// All player characters (portrait order).
    pub fn pc_ids(&self) -> &[EntityId] {
        &self.world.pc_ids
    }

    /// All NPCs (soldiers + civilians).
    pub fn npc_ids(&self) -> Vec<EntityId> {
        self.world.entities.npc_ids().collect()
    }

    /// Currently selected PC ids for the [`PlayerId::HOST`] seat.
    ///
    /// Single-player host code (HUD, renderer, input translation)
    /// always reads this accessor — there's only one seat in
    /// single-player and it's the host.  Multi-seat callers should use
    /// [`Self::seat_selection`] with their own
    /// [`crate::player_command::PlayerId`].
    pub fn selected_pc_ids(&self) -> &[EntityId] {
        &self.players.seats[0].selection
    }

    /// Selection for a specific seat, or `&[]` if the seat hasn't
    /// joined yet.  Multi-seat read path.
    pub fn seat_selection(&self, player_id: crate::player_command::PlayerId) -> &[EntityId] {
        self.players
            .seats
            .get(player_id.0 as usize)
            .map(|s| s.selection.as_slice())
            .unwrap_or(&[])
    }

    /// Look up [`SeatState`] for a `PlayerId`.  `None` when the seat
    /// hasn't materialised — happens before the seat's first
    /// `ConnectSeat` (or, for non-host seats, before its first
    /// command of any kind).
    pub fn seat(&self, player_id: crate::player_command::PlayerId) -> Option<&SeatState> {
        self.players.seats.get(player_id.0 as usize)
    }

    /// All currently-existing seats (connected or disconnected) in
    /// `PlayerId` order.  Renderer uses this to walk every seat for
    /// the portrait "controlled by" overlay; transport uses it to
    /// drive seat-list UI.
    pub fn seats(&self) -> &[SeatState] {
        &self.players.seats
    }

    /// Iterate over `(PlayerId, &SeatState)` pairs for every seat
    /// that's currently active — i.e. the host seat (always) plus
    /// any peer seat with `connected = true`.  Disconnected peers
    /// are filtered out so the renderer doesn't draw stale
    /// "controlled by" labels.
    pub fn active_seats(
        &self,
    ) -> impl Iterator<Item = (crate::player_command::PlayerId, &SeatState)> {
        self.players.seats.iter().enumerate().filter_map(|(i, s)| {
            if s.is_active(i) {
                Some((crate::player_command::PlayerId(i as u8), s))
            } else {
                None
            }
        })
    }

    /// Ensure a seat exists for `player_id`, growing `self.players.seats` with
    /// default [`SeatState`]s as needed, and return its index.
    ///
    /// New seats start empty (no selection, no hotgroups) — they only
    /// pick up state once the player issues commands.  This is the
    /// drop-in/drop-out hook: a peer that joins mid-mission gets a
    /// fresh seat, and a peer that leaves keeps its slot (their
    /// last-issued selection survives so the PCs stay where they
    /// were left, on autopilot).
    pub fn ensure_seat(&mut self, player_id: crate::player_command::PlayerId) -> usize {
        let idx = player_id.0 as usize;
        if idx >= self.players.seats.len() {
            self.players.seats.resize_with(idx + 1, SeatState::default);
        }
        idx
    }

    /// `true` if at least one selected PC currently has its rotating
    /// selection circle drawn this frame — i.e. the per-PC posture /
    /// in-building filter lets at least one PC through.
    ///
    /// Used host-side to gate `SelectionMark::tick` so the ping-pong
    /// animation freezes whenever no circle would be drawn — the
    /// frame counter advance originally lived inside `DrawAt`, so
    /// non-drawing periods naturally paused the animation.
    pub fn any_selected_pc_drawing_selection_mark(&self) -> bool {
        for &pc_id in &self.players.seats[0].selection {
            if self.pc_draws_selection_mark(pc_id) {
                return true;
            }
        }
        false
    }

    /// Check whether the entity's cached sector (set during door-pass
    /// transitions) is a building sector.
    ///
    /// Takes the entity's `element.sector` sector number and returns
    /// the same handle when the sector has the BUILDING flag, so
    /// callers can also compare "same building".
    pub(crate) fn entity_building_sector(
        &self,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> Option<crate::position_interface::SectorHandle> {
        let sector_num = sector?;
        let raw = u16::from(sector_num);
        let gs = self.grid_sector_by_number(crate::sector::SectorNumber::new(raw as i16))?;
        gs.sector_type.is_building().then_some(sector_num)
    }

    /// `true` when the rotating ground selection circle should be drawn
    /// for `pc_id`.
    pub fn pc_draws_selection_mark(&self, pc_id: EntityId) -> bool {
        let Some(entity) = self.get_entity(pc_id) else {
            return false;
        };
        if !entity.is_active() {
            return false;
        }

        let elem = entity.element_data();
        if elem.posture == crate::element::Posture::Flying
            || elem.hidden_in_building
            || elem.is_in_door_transit()
        {
            return false;
        }

        self.entity_building_sector(elem.sector()).is_none()
    }

    /// `true` if `pc_id` has any queued `Command::ShootBow` sequence
    /// element.  Used by the right-click `Bow` arm to decide whether to
    /// drain the shoot-list (queue non-empty) or cancel the Bow action
    /// (queue empty).
    pub fn pc_has_pending_shoot_bow(&self, pc_id: EntityId) -> bool {
        self.get_entity(pc_id)
            .and_then(|entity| entity.human_data())
            .is_some_and(|human| !human.pending_shoots.is_empty())
            || self
                .orders
                .sequence_manager
                .queued_element_exists(pc_id, crate::element::Command::ShootBow)
    }

    pub(in crate::engine) fn pc_should_hold_shoot_bow(
        &self,
        owner: EntityId,
        command: crate::element::Command,
    ) -> bool {
        use crate::order::OrderType;
        command == crate::element::Command::ShootBow
            && self.get_entity(owner).is_some_and(|entity| entity.is_pc())
            && self.get_entity(owner).is_some_and(|entity| {
                matches!(
                    entity.sprite().last_action,
                    OrderType::ShootingWithBow
                        | OrderType::ShootingWithBowUp
                        | OrderType::TransitionLoadingBow
                        | OrderType::TransitionRaisingBow
                        | OrderType::TransitionEquipBow
                )
            })
    }

    pub(in crate::engine) fn queue_pc_shoot_bow(
        &mut self,
        owner: EntityId,
        element_ref: crate::sequence::SequenceElementRef,
    ) {
        let human = self
            .world
            .entities
            .get_mut(owner)
            .and_then(|entity| entity.human_data_mut())
            .unwrap_or_else(|| panic!("shoot-list owner {} is not human", owner.index()));
        if !human.pending_shoots.contains(&element_ref) {
            human.pending_shoots.push(element_ref);
        }
    }

    /// Drop the retained Human::Instruct shoot FIFO without altering the
    /// sequence elements themselves, matching C++ `ClearShootList`.
    pub(in crate::engine) fn clear_pc_shoot_list(&mut self, owner: EntityId) -> bool {
        let human = self
            .world
            .entities
            .get_mut(owner)
            .and_then(|entity| entity.human_data_mut())
            .unwrap_or_else(|| panic!("shoot-list owner {} is not human", owner.index()));
        let had_entries = !human.pending_shoots.is_empty();
        human.pending_shoots.clear();
        had_entries
    }

    /// Original `RHElementActorHuman::ProcessShootList`: retry the oldest
    /// retained element synchronously, and remove it only when Instruct
    /// accepts it. The sprite animation gate is deliberately exact.
    pub(crate) fn process_shoot_list_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let entity = self.world.entities.get(owner).unwrap_or_else(|| {
            panic!(
                "shoot-list owner {} disappeared from its legacy slot",
                owner.index()
            )
        });
        assert!(
            entity.human_data().is_some(),
            "shoot-list owner {} is not human",
            owner.index()
        );
        use crate::order::OrderType;
        if !matches!(
            entity.sprite().last_action,
            OrderType::AimingWithBow | OrderType::AimingWithBowUp
        ) {
            return;
        }
        let Some(element_ref) = entity
            .human_data()
            .and_then(|human| human.pending_shoots.first().copied())
        else {
            return;
        };
        let accepted = self.instruct_held_shoot_bow(sim, assets, owner, element_ref);
        if accepted {
            let human = self
                .world
                .entities
                .get_mut(owner)
                .and_then(|entity| entity.human_data_mut())
                .expect("validated human shoot-list owner disappeared");
            assert_eq!(human.pending_shoots.first(), Some(&element_ref));
            human.pending_shoots.remove(0);
        }
    }

    /// Background animation entity ids.
    pub fn bg_animation_ids(&self) -> Vec<EntityId> {
        self.world
            .entities
            .occupied()
            .filter_map(|(id, entity)| entity.is_background_animation().then_some(id))
            .collect()
    }

    /// Quick-select group `idx` (0 = group 1, 8 = group 9).
    pub fn quick_select_group(&self, idx: usize) -> &[EntityId] {
        &self.players.seats[0].quick_select_groups[idx]
    }

    /// Floating indicator manager (titbits: stars, emoticons, smoke, splashes).
    /// The host reads it every frame to drive the titbit renderer; scripts
    /// and input handlers add new titbits through [`EngineInner::titbit_manager_mut`].
    pub fn titbit_manager(&self) -> &crate::titbit::TitbitManager {
        &self.feedback.titbit_manager
    }

    /// Install the titbit renderer's per-row frame counts.  Called at
    /// level load and whenever the ambience shadow colour changes (the
    /// titbit atlas is rebuilt host-side and hands fresh counts back).
    /// Safe to call mid-tick: `titbit_manager.row_frame_counts` is
    /// level renderer metadata and not part of the rollback hash.
    pub(crate) fn set_titbit_row_frame_counts(&mut self, counts: Vec<u16>) {
        self.feedback.titbit_manager.set_row_frame_counts(counts);
    }

    /// Current dotted-chain animation phase, advanced by the engine
    /// tick.  Host renderers read this to chain dotted line segments
    /// within a frame; they do not write it back — next frame's
    /// `perform_hourglass` re-advances it via
    /// `TitbitManager::prepare_refresh`.
    pub fn titbit_dotted_start(&self) -> f32 {
        self.feedback.titbit_manager.dotted_start()
    }

    /// Global AI state (alert levels, seek points, …). Read-only.
    pub fn ai_global(&self) -> &AiGlobalState {
        &self.ai.global
    }

    /// Read-only access to the per-PC quick-action macro store.  Host
    /// renderers use this to iterate slots for the portrait strip.
    pub fn macro_store(&self) -> &crate::macro_store::MacroStore {
        &self.players.macro_store
    }

    /// Remove all titbits owned by `pc` at QA slot `slot`.  Resolves
    /// the titbit id from the PC's per-slot titbit-id table, then
    /// drops every titbit whose id matches.  Returns `true` iff at
    /// least one titbit was removed (also `false` when the slot is
    /// empty).
    pub fn remove_quick_action_titbits_for(&mut self, pc: EntityId, slot: u8) -> bool {
        let Some(state) = self.players.macro_store.get(pc) else {
            return false;
        };
        let Some(titbit_id) = state.get_slot_titbit(slot as usize) else {
            return false;
        };
        self.feedback
            .titbit_manager
            .remove_quick_action_titbits_by_id(titbit_id)
    }

    /// Does `pc` have a recorded macro in `slot`?
    pub fn has_quick_action(&self, pc: EntityId, slot: u8) -> bool {
        self.players
            .macro_store
            .get(pc)
            .map(|s| s.has_macro(slot as usize))
            .unwrap_or(false)
    }

    /// Abort the macro at `(pc, slot)`: drop the slot's titbit and clear
    /// the slot's recorded steps + stored titbit id.  Returns `true` iff
    /// the slot had a macro before the call.
    ///
    pub fn abort_quick_action(&mut self, pc: EntityId, slot: u8) -> bool {
        if !self.has_quick_action(pc, slot) {
            return false;
        }
        self.remove_quick_action_titbits_for(pc, slot);
        if let Some(state) = self.players.macro_store.get_mut(pc) {
            state.clear_slot(slot as usize);
        }
        let saved_pc = self
            .get_entity_mut(pc)
            .and_then(|entity| entity.pc_data_mut())
            .unwrap_or_else(|| panic!("quick-action owner {pc:?} is not a PC"));
        let slot = slot as usize;
        saved_pc.quick_action_types[slot] = crate::element_kinds::QuickAction::None;
        saved_pc.quick_action_sequences[slot] = None;
        saved_pc.quick_seek_sequences[slot] = None;
        saved_pc.quick_action_special_counts[slot] = 0;
        saved_pc.quick_action_buttons[slot] = 0;
        saved_pc.quick_action_interactors[slot] = None;
        saved_pc.titbits[slot] = u32::MAX;
        true
    }

    /// Tetris-shift slot `slot..NUMBER_OF_QA_MEMORY` on every PC.
    /// Called once all PCs have successfully launched their slot-`slot`
    /// macros — see `apply_start_macro` which drives the call.
    pub(crate) fn do_tetris_macro(&mut self, display: &mut HostDisplayState, slot: u8) {
        let pcs = self.world.pc_ids.clone();
        for pc in pcs {
            if let Some(state) = self.players.macro_store.get_mut(pc) {
                state.do_tetris(slot as usize);
            }
            let saved_pc = self
                .get_entity_mut(pc)
                .and_then(|entity| entity.pc_data_mut())
                .unwrap_or_else(|| panic!("quick-action owner {pc:?} is not a PC"));
            let first = slot as usize;
            for index in first..crate::macro_store::NUMBER_OF_QA_MEMORY - 1 {
                saved_pc.quick_action_types[index] = saved_pc.quick_action_types[index + 1];
                saved_pc.quick_action_sequences[index] =
                    saved_pc.quick_action_sequences[index + 1].clone();
                saved_pc.quick_seek_sequences[index] =
                    saved_pc.quick_seek_sequences[index + 1].clone();
                saved_pc.titbits[index] = saved_pc.titbits[index + 1];
                saved_pc.quick_action_interactors[index] =
                    saved_pc.quick_action_interactors[index + 1];
                saved_pc.quick_action_buttons[index] = saved_pc.quick_action_buttons[index + 1];
                saved_pc.portrait.quick_icons[index] = saved_pc.portrait.quick_icons[index + 1];
            }
            let last = crate::macro_store::NUMBER_OF_QA_MEMORY - 1;
            saved_pc.quick_action_types[last] = crate::element_kinds::QuickAction::None;
            saved_pc.quick_action_sequences[last] = None;
            saved_pc.quick_seek_sequences[last] = None;
            saved_pc.titbits[last] = u32::MAX;
            saved_pc.quick_action_interactors[last] = None;
            saved_pc.quick_action_buttons[last] = 0;
        }
        display.rearm_macro_tetris(&self.world.pc_ids, &self.players.macro_store, slot as usize);
    }

    /// Enable or disable the `--goldeneye` cheat (NPCs can't see the player).
    /// Set once at startup from CLI args.
    pub(crate) fn set_golden_eye_mode(&mut self, on: bool) {
        self.ai.global.golden_eye_mode = on;
    }

    /// Whether the `--goldeneye` cheat is active.  Used by the PC
    /// refresh path to render every PC sprite at 50% alpha.
    pub fn get_golden_eye_mode(&self) -> bool {
        self.ai.global.golden_eye_mode
    }

    /// Weather / ambiance state (night colour, rain, fog, …).
    pub fn weather(&self) -> &WeatherState {
        &self.world.weather
    }

    /// Shield protection state (for the "Immortality" cheat).
    pub fn shield(&self) -> &ShieldState {
        &self.world.shield
    }

    /// Spatial acceleration grid (sectors, masks, jump lines, doors).
    pub fn fast_grid(&self) -> &FastFindGrid {
        &self.world.fast_grid
    }

    /// Canonical door selected by the same click-polygon fallback used by
    /// group movement when no fast-grid sector polygon contains the point.
    pub fn group_move_door_at(&self, point: crate::coordinates::MapPoint) -> Option<u32> {
        movement::door_click_polygon_at(&self.script_domains.interactables.doors, point)
    }

    /// A* waypoint pathfinder.
    pub fn pathfinder(&self) -> &PathFinder {
        &self.world.pathfinder
    }

    /// Committed path waypoints for an actor's active movement, if any.
    ///
    /// Returns the `(target_x, target_y)` of each remaining (non-`done`)
    /// order on the actor's currently-executing sequence element, in
    /// execution order.  Used by the surface debug overlay to draw the
    /// path the character will follow.  Returns `None` when the actor
    /// has no active movement element.
    pub fn actor_path_waypoints(
        &self,
        actor: EntityId,
    ) -> Option<Vec<crate::coordinates::MapPoint>> {
        let entity = self.get_entity(actor)?;
        let actor_data = entity.actor_data()?;
        let seq_id = actor_data.active_movement.sequence_id?;
        let elem_idx = actor_data.active_movement.element_index;
        let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
        Some(
            elem.orders
                .iter()
                .filter(|o| !o.done)
                .map(|o| crate::coordinates::MapPoint::new(o.target_x, o.target_y))
                .collect(),
        )
    }

    /// Destination markers drawn on the ground.
    pub fn ground_mark(&self) -> &GroundMark {
        &self.feedback.ground_mark
    }

    /// Populate ground-mark sprite data at resource-load time (host-side
    /// call). Runtime writes into `ground_mark` are command-driven engine
    /// mutations; per-seat trajectory preview marks live on `Host`.
    pub(crate) fn set_ground_mark_sprite_data(
        &mut self,
        half_w: f32,
        half_h: f32,
        frame_sizes: Vec<(u16, u16)>,
        per_frame_offsets: Vec<(i16, i16)>,
    ) {
        self.feedback
            .ground_mark
            .set_sprite_data(half_w, half_h, frame_sizes, per_frame_offsets);
    }

    /// Combined static + dynamic sight obstacles. Static come from
    /// `LevelAssets::static_sight_obstacles` (Arc-shared, populated at
    /// level load); dynamic are this frame's shields. Returns a
    /// `ObstacleList` view that exposes the flat global indexing used
    /// by patches and per-actor obstacle references.
    pub fn sight_obstacles<'a>(
        &'a self,
        assets: &'a LevelAssets,
    ) -> crate::sight_obstacle::ObstacleList<'a> {
        crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        }
    }

    /// Mutator for the runtime active flag on a static sight obstacle.
    /// Out-of-range indices (including dynamic obstacles) silently no-op
    /// — dynamic obstacles are always implicitly active.
    pub(crate) fn set_sight_obstacle_active(&mut self, idx: u32, active: bool) {
        if let Some(slot) = self
            .world
            .static_sight_obstacle_active
            .get_mut(idx as usize)
        {
            *slot = active;
        }
    }

    /// Short mission briefing entries (read-only, drained by host UI).
    pub fn short_briefings(&self) -> &ShortBriefings {
        &self.mission_domain.short_briefings
    }

    /// Read the accumulated mission statistics (money, score, kills,
    /// recruitment, …).  Written by script natives during the tick and
    /// rolled up at mission end by [`EngineInner::apply_quit_mission_updates`].
    pub fn mission_stat(&self) -> &MissionStat {
        &self.mission_domain.mission_stat
    }

    /// Whether the camera is locked to follow an entity.
    pub fn locker_active(&self) -> bool {
        self.players.seats[0].locker_active
    }

    /// Original messenger view-lock, distinct from camera-follow locker mode.
    pub fn view_locked(&self) -> bool {
        self.players.view_locked
    }

    /// Whether the player has the engine "user-locked" (alt-lock UI).
    pub fn user_locked(&self) -> bool {
        self.players.user_locked
    }

    /// Enqueue a `SimpleMessage` onto the engine's messenger.
    ///
    /// Host-side producers of messenger events (console overlay,
    /// switch-task handler, alt-tab watchdog) use this instead of
    /// touching `self.orders.messenger` directly — the field is `pub(crate)`
    /// to keep the drain loop authoritative over which variants are
    /// observed.
    pub fn send_simple_message(&mut self, msg: crate::messenger::SimpleMessage) {
        self.orders.messenger.send(crate::messenger::Message::new(
            crate::messenger::MessageType::Simple(msg),
        ));
    }

    /// Whether `pc` is part of the currently-armed recording set.
    pub fn is_qa_recording_for(&self, pc: EntityId) -> bool {
        self.players.qa_recording_for.contains(&pc)
    }

    /// Stop the in-progress quick-action macro recording (host-side
    /// portrait-click handler).  Idempotent.
    pub(crate) fn stop_recording_macro(&mut self) {
        let slot = self.players.qa_recording_slot as usize;
        let recording = self.players.qa_recording_for.clone();
        for pc_id in recording {
            let (has_macro, titbit) = self
                .players
                .macro_store
                .get(pc_id)
                .map(|state| (state.has_macro(slot), state.get_slot_titbit(slot)))
                .unwrap_or((false, None));
            let icon = if has_macro {
                let titbit = titbit.unwrap_or_else(|| {
                    panic!("recorded quick-action PC {pc_id:?} slot {slot} has no titbit")
                });
                crate::element::PcPortraitQuickIconState {
                    titbit_id: u32::from(self.feedback.titbit_manager.get_phase(titbit)),
                    running: self.feedback.titbit_manager.is_running_for_qa(titbit),
                }
            } else {
                Default::default()
            };
            if let Some(state) = self.players.macro_store.get_mut(pc_id) {
                state.stop_recording();
            }
            let pc = self
                .get_entity_mut(pc_id)
                .and_then(|entity| entity.pc_data_mut())
                .unwrap_or_else(|| panic!("quick-action recording target {pc_id:?} is not a PC"));
            pc.portrait.quick_icons[slot] = icon;
        }
        self.players.qa_recording_for.clear();
    }

    /// Re-target the in-flight macro recording after the selection has
    /// changed.  Forwarded on every
    /// MSG_SELECT_CHARACTER / MSG_SELECT_ADD_CHARACTER /
    /// MSG_UNSELECT_CHARACTER.
    ///
    /// End recording on PCs that left the selection and start it on PCs
    /// that entered the selection, keeping the slot index stable.  If
    /// no recording is in flight this is a no-op.
    ///
    /// Post-process emitter for the `MSG_SELECT_CHARACTER[_WITH_ECHO]`,
    /// `MSG_SELECT_ADD_CHARACTER[_WITH_ECHO]`, and
    /// `MSG_UNSELECT_CHARACTER` arms: broadcast `MSG_STATURE`, nudge any
    /// in-flight macro recording to re-target the current selection via
    /// `MSG_UPDATE_RECORDING_MACRO`, and drop the
    /// "restore-on-stop-recording" snapshot so a later
    /// `MSG_STOP_RECORDING_MACRO` doesn't rearm a stale action.
    pub(crate) fn emit_character_selection_followups(&mut self) {
        self.orders.messenger.send(crate::messenger::Message::new(
            crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
        ));
        self.orders.messenger.send(crate::messenger::Message::pc(
            crate::messenger::PcMessage::UpdateRecordingMacro,
            None,
        ));
        self.players.action_before_recording_macro = crate::profiles::Action::NoAction;
    }

    pub(crate) fn update_recording_after_selection_change(&mut self) {
        if self.players.qa_recording_for.is_empty() {
            return;
        }
        let slot = self.players.qa_recording_slot;
        let selected: Vec<EntityId> = self.players.seats[0].selection.clone();
        let current = self.players.qa_recording_for.clone();
        for pc_id in &current {
            if !selected.contains(pc_id)
                && let Some(state) = self.players.macro_store.get_mut(*pc_id)
            {
                state.stop_recording();
            }
        }
        for pc_id in &selected {
            if !current.contains(pc_id) {
                self.players
                    .macro_store
                    .get_or_insert(*pc_id)
                    .begin_recording(slot);
            }
        }
        self.players.qa_recording_for = selected;
    }

    /// Request the PC-info hover overlay to show (`Some(pc_id)`) or hide
    /// (`None`).  The host writes into this via its per-frame mouse
    /// handler; the renderer reads the overlay after the tick drains
    /// [`SideEffects::overlay`] into [`Host::pc_info_overlay`].
    ///
    /// Backed by the `MSG_SHOW_PC_INFORMATION` /
    /// `MSG_HIDE_PC_INFORMATION` messenger pair — the messenger
    /// indirection exists for engine-internal sites, but the host just
    /// writes the overlay directly because there's nothing else
    /// listening.
    ///
    /// Both show and hide handlers early-out unless we're in Sherwood,
    /// so the popup only ever appears in the Sherwood (HQ) mission.
    pub(crate) fn request_pc_info_overlay(
        &mut self,
        assets: &LevelAssets,
        focus: Option<EntityId>,
    ) {
        if !self.is_sherwood(&assets.profile_manager) {
            return;
        }
        self.feedback.pending_side_effects.overlay = Some(match focus {
            Some(pc_id) => OverlayChange::Show { pc_id },
            None => OverlayChange::Hide,
        });
    }

    /// `true` when the current mission is the Sherwood (HQ) hideout.
    pub fn is_sherwood(&self, profiles: &crate::profiles::ProfileManager) -> bool {
        self.is_sherwood_mission(&self.mission_domain.campaign, profiles)
    }

    /// Build a fresh `Order` (via `alloc_order_id` for the id) and push
    /// it. Shorthand for the common engine-side pattern of allocating a
    /// unique id, building an Order at `(x, y)` with `order_type`, and
    /// pushing it onto the given element.
    pub(crate) fn push_new_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
        x: f32,
        y: f32,
    ) -> std::num::NonZeroU32 {
        let id = self.orders.allocate_order_id();
        self.orders.sequence_manager.push_order_on(
            seq_id,
            elem_idx,
            crate::order::Order::new(order_type, x, y, id),
        );
        id
    }

    /// Advance a sequence element to its next order, or terminate when
    /// the order list is exhausted.  Pops the front order; if a new
    /// front exists, the element keeps running with that order;
    /// otherwise the element terminates and `EventDone` fires up the
    /// chain.
    ///
    /// This runs whenever an order's animation completes with the
    /// default [`OrderCompletion::AdvanceElement`] hook.  When the
    /// queue drains for a non-wait element, we terminate it. The actor
    /// Hourglass installs a fresh wait element at its next entry only when no
    /// synchronous condolence/AI callback instructed a real successor.
    ///
    /// The BORED ↔ BORED_RANDOM idle cycle does NOT route through here
    /// — its Execute arm consumes the event in
    /// `dispatch_arm_completion` (`engine/animation.rs`) and mutates
    /// the front order in place without popping.
    pub(crate) fn do_next_order(&mut self, seq_id: crate::sequence::SequenceId, elem_idx: usize) {
        if tracing::enabled!(target: "parity_owner_handoff", tracing::Level::TRACE) {
            let element_state = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .map(|element| {
                    (
                        element.owner,
                        element.command,
                        element.state,
                        element
                            .orders
                            .front()
                            .map(|order| (order.order_type, order.order_id)),
                        element.orders.len(),
                    )
                });
            let owner_state = element_state
                .and_then(|(owner, _, _, _, _)| owner)
                .map(|owner| {
                    let selected = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(owner);
                    let (active_movement, goal) = self
                        .get_entity(owner)
                        .map(|entity| {
                            let active_movement = entity.actor_data().map(|actor| {
                                (
                                    actor.active_movement.sequence_id,
                                    actor.active_movement.element_index,
                                )
                            });
                            (active_movement, entity.position_iface().map_goal())
                        })
                        .unwrap_or_default();
                    (selected, active_movement, goal)
                });
            tracing::trace!(
                target: "parity_owner_handoff",
                frame = self.control.frame_counter,
                ?seq_id,
                elem_idx,
                ?element_state,
                ?owner_state,
                "do_next_order before popping front order"
            );
        }
        // Pop the just-completed front order, capture context.
        let Some((owner, next_order)) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .map(|elem| {
                if !elem.orders.is_empty() {
                    let popped = elem.pop_current_order();
                    if tracing::enabled!(tracing::Level::TRACE) {
                        let remaining: Vec<(crate::order::OrderType, f32, f32)> = elem
                            .orders
                            .iter()
                            .map(|o| (o.order_type, o.target_x, o.target_y))
                            .collect();
                        tracing::trace!(
                            owner = ?elem.owner,
                            ?popped,
                            ?remaining,
                            "do_next_order: popped front order"
                        );
                    }
                }
                let next_order =
                    elem.current_order()
                        .map(|order| crate::element::InstalledActorOrder {
                            order_id: order.order_id,
                            order_type: order.order_type,
                        });
                (elem.owner, next_order)
            })
        else {
            return;
        };

        if let Some(next_order) = next_order {
            // Original DoNextOrder assigns mpOrder = Proceed() immediately.
            if let Some(owner) = owner {
                self.world
                    .entities
                    .get_mut(owner)
                    .and_then(Entity::actor_data_mut)
                    .expect("next-order owner disappeared before mpOrder publication")
                    .installed_order = Some(next_order);
            }
            return;
        }

        // Queue exhausted. RHElementActor::SendCondolationCard clears the
        // current element's map goal before dropping mpSequenceElement and
        // mpOrder. `do_next_order` is only called for the actor's selected
        // current order, so perform that owner-side cleanup at the same
        // terminal boundary before the Rust sequence registry removes it.
        if let Some(owner) = owner {
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("exhausted-order owner disappeared before mpOrder clear")
                .installed_order = None;
            if tracing::enabled!(target: "parity_owner_handoff", tracing::Level::TRACE) {
                let selected = self
                    .orders
                    .sequence_manager
                    .current_element_for_actor(owner);
                let (active_movement, goal) = self
                    .get_entity(owner)
                    .map(|entity| {
                        let active_movement = entity.actor_data().map(|actor| {
                            (
                                actor.active_movement.sequence_id,
                                actor.active_movement.element_index,
                            )
                        });
                        (active_movement, entity.position_iface().map_goal())
                    })
                    .unwrap_or_default();
                tracing::trace!(
                    target: "parity_owner_handoff",
                    frame = self.control.frame_counter,
                    ?seq_id,
                    elem_idx,
                    ?owner,
                    ?selected,
                    ?active_movement,
                    ?goal,
                    "do_next_order before exhausted-order goal clear"
                );
            }
            self.world
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| {
                    panic!("current order owner {owner:?} disappeared before terminal goal cleanup")
                })
                .element_data_mut()
                .sprite
                .position_iface
                .set_map_goal(crate::coordinates::MapPoint::new(0.0, 0.0));
        }

        // Terminate the element. Do not eagerly install Wait here:
        // RHElementActor::DoNextOrder calls SetState(TERMINATED), whose
        // SendCondolationCard callback can synchronously instruct a real
        // successor. The actor's next Hourglass entry supplies Wait only if
        // that stack unwinds without one.
        self.orders
            .sequence_manager
            .element_terminated(seq_id, elem_idx);
    }

    /// Guarantee that `entity_id` has a live `Command::Wait` sequence
    /// element running at `SequencePriority::Wait`.  Launches a fresh
    /// wait element whenever the actor has no current order to
    /// execute.  No-op when a wait element already exists for this
    /// actor.
    ///
    /// Called by the null-order guard at the start of an actor Hourglass.
    /// Exhausting the final order does not call this again in the same slot:
    /// Original leaves `mpOrder` null through ActionChange and creates the
    /// fallback Wait on the actor's next frame.
    pub(crate) fn ensure_wait_element(&mut self, entity_id: EntityId) {
        use crate::sequence::{SequenceElement, SequencePriority};

        // Original Actor::Hourglass installs Wait whenever the actor has no
        // current order. Future Todo/Postponed elements do not count: they may
        // sit behind an ownerless Timer while this actor idles. A concurrently
        // InProgress element is the only state that corresponds to the
        // Original's live `mpSequenceElement`/`mpOrder`.
        if self
            .orders
            .sequence_manager
            .current_element_for_actor(entity_id)
            .is_some()
        {
            return;
        }

        let lift_idle = self
            .get_entity(entity_id)
            .and_then(|e| e.element_data().sector())
            .and_then(|sector| {
                self.grid_sector_by_number(crate::sector::SectorNumber::new(
                    u16::from(sector) as i16
                ))
            })
            .and_then(|sector| match sector.lift_type {
                Some(crate::sector::LiftType::Wall) => {
                    Some((crate::element::Posture::OnWall, sector.lift_direction))
                }
                Some(crate::sector::LiftType::Ladder) => {
                    Some((crate::element::Posture::OnLadder, sector.lift_direction))
                }
                _ => None,
            });
        if let Some((posture, direction)) = lift_idle
            && let Some(entity) = self.get_entity_mut(entity_id)
        {
            entity.set_posture(posture);
            entity.element_data_mut().set_direction_instantly(direction);
            // Actor::Wait translates to the non-animation Freezing order on
            // a ladder or wall. Original MakeActionTransition has no
            // OnLadder/OnWall arm, so it deliberately preserves the current
            // action state (normally Moving) while holding the climb frame.
            tracing::debug!(
                entity = ?entity_id,
                ?posture,
                direction,
                "Wait: normalized idle actor in lift sector"
            );
        }

        let mut elem = SequenceElement::new(1, crate::element::Command::Wait, Some(entity_id));
        elem.priority = SequencePriority::Wait;
        // RHElementActor::Wait launches this through the normal owned-element
        // Instruct path.  That path stamps the current posture/action state
        // and, crucially, prepends the Waiting -> Bored transition orders.
        // Bypassing it made a freshly loaded upright NPC jump straight from
        // its authored WAITING_UPRIGHT pose to WAITING_UPRIGHT_BORED on the
        // first frame.
        self.launch_element(elem);
    }

    /// Consume the typed motion-stage input and feed it into
    /// the motion grid (pathfinder graph, lift tables, obstacle states).
    /// Called once during `Engine::new`; bridges background-load and
    /// motion-area initialisation.  Must run only during level load —
    /// it mutates hashed state and is not driven by the tick pipeline,
    /// so calling it during gameplay would desync rollback.
    pub(crate) fn build_motion_stage(
        &mut self,
        assets: &mut LevelAssets,
        staging: &mut LevelLoadStaging,
    ) {
        if let Some(motion_data) = staging.motion.motion_data.take() {
            let lifts = std::mem::take(&mut staging.motion.lifts);
            self.initialize_motion_from_level_data(assets, staging, &motion_data, &lifts);
        }
    }

    /// Reveal all blipped entities — backs the console `UNBLIP`
    /// command, which iterates every NPC and reveals it.
    pub(crate) fn reveal_all_blips(&mut self) {
        for (_, entity) in self.world.entities.npcs_mut() {
            if entity.element_data().blipped {
                entity.reveal_blip();
            }
        }
    }

    /// Get a mutable reference to an entity by ID.
    pub(crate) fn get_entity_mut<I: Into<EntityId>>(&mut self, id: I) -> Option<&mut Entity> {
        self.world.entities.get_mut(id)
    }

    /// Remove an entity. Leaves a None hole (IDs are stable).
    pub(crate) fn remove_entity<I: Into<EntityId>>(&mut self, id: I) {
        let id = id.into();
        self.world.entities.remove(id);
        // Remove from index lists
        self.world.pc_ids.retain(|&i| i != id);
        self.world.original_pc_registry_ids.retain(|&i| i != id);
        self.players.seats[0].selection.retain(|&i| i != id);
        // Any pending path request for this actor is cancelled when
        // the element tears down.  Entity removal implies all its
        // elements die, so drop the retry-queue entries eagerly
        // instead of waiting for the next retry pass to notice the
        // owner is gone.
        self.orders.failed_path_requests.retain(|r| r.owner != id);
        self.orders
            .pending_move_requests
            .retain(|(eid, _)| *eid != id);
        self.orders.pending_path_requests.retain_not_owned_by(id);
    }

    /// Number of live entities.
    pub fn entity_count(&self) -> usize {
        self.world.entities.occupied().count()
    }

    /// Remove a PC entity from the engine by its character profile index.
    ///
    /// 1. Look up the PC by profile index.
    /// 2. Clear it from the current selection (forwards
    ///    `MSG_UNSELECT_CHARACTER`).  `remove_entity` would retain it
    ///    out of `selected_pc_ids` too, but doing it here keeps any
    ///    intermediate inspection consistent.
    /// 3. Flag the PC as no longer playable (`SetPlayable(false)`).
    /// 4. Detach the entity slot from all ID lists
    ///    (`RemoveElement(pc, remove_from_script=false)`).
    ///
    /// Used by [`convert_selected_peasants_to_blazons`] (and any
    /// future peasant-liquidation path).  Returns `true` when a PC
    /// was actually removed, `false` when no matching entity was
    /// found.
    pub(crate) fn remove_pc_by_profile(
        &mut self,
        profile_idx: crate::profiles::CharacterProfileIdx,
    ) -> bool {
        let Some(pc_id) = self.world.pc_ids.iter().copied().find(|&id| {
            matches!(
                self.get_entity(id),
                Some(Entity::Pc(pc)) if pc.pc.profile_index == profile_idx,
            )
        }) else {
            return false;
        };

        // `MSG_UNSELECT_CHARACTER`: clears selection, hides portrait
        // highlight, etc.  The selection list is authoritative, so
        // removing the id here mirrors the message's observable effect.
        self.players.seats[0].selection.retain(|&id| id != pc_id);

        // `SetPlayable(false)` — survives into the handful of frames
        // between clearing selection and wiping the slot.  After
        // `remove_entity` the field is academic.
        if let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) {
            pc.pc.playable = false;
        }

        // `RemoveElement(pc, remove_from_script=false)`.
        self.remove_entity(pc_id);
        true
    }

    /// Convert selected peasants to blazons.
    ///
    /// Walks the mission team, sorting each peasant into reservists
    /// (random-weighted by life points) or straight removal, invokes
    /// `remove_pc_by_profile` per peasant, resets the mission team,
    /// and credits `BLAZON_VALUE`.
    ///
    /// Triggered from `MSG_START_MISSION` when
    /// `IsMenToBlazonConversionMode()` is set; the caller lives in
    /// `game_session.rs` on the Sherwood "StartMission" button path.
    pub(crate) fn convert_selected_peasants_to_blazons(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &crate::profiles::ProfileManager,
    ) {
        let campaign = &self.mission_domain.campaign;
        let number_to_convert =
            campaign.get_number_of_peasants_to_convert_to_blazons(profiles) as usize;
        let quotation = {
            let next_idx = match campaign.next_mission_idx {
                Some(i) => i,
                None => {
                    tracing::warn!("convert_selected_peasants_to_blazons: no next mission");
                    return;
                }
            };
            campaign.missions[next_idx]
                .profile(profiles)
                .peasant_to_blazon_quotation
        };
        let mission_team: Vec<usize> = campaign.mission_team_indices.clone();

        // Snapshot life_points + profile_idx per team entry before we
        // start mutating the campaign.  `remove_pc_by_profile` takes a
        // profile index rather than a character index because the
        // engine-side entity is indexed by profile.
        let entries: Vec<(usize, Option<crate::profiles::CharacterProfileIdx>, i16)> = mission_team
            .iter()
            .map(|&char_idx| {
                let (profile_idx, life_points) = campaign
                    .characters
                    .get(char_idx)
                    .map(|desc| (desc.character_profile_idx, desc.status.life_points))
                    .unwrap_or((None, 0));
                (char_idx, profile_idx, life_points)
            })
            .collect();

        const LIFEPOINTS_PC_X2: u32 = (crate::pc_status::LIFEPOINTS_PC as u32) << 1;

        for (i, (char_idx, profile_idx_opt, life_points)) in entries.iter().enumerate() {
            if i >= number_to_convert {
                // The "Place those peasants on a free beam-me" branch
                // is inactive, so extra
                // peasants past the convert count stay in the team
                // untouched here.  The trailing `ResetMissionTeam()`
                // below wipes the team list so they don't carry into
                // the new mission.
                break;
            }

            // Original: `RHGame::ConvertSelectedPeasantsToBlazons` in
            // `original-code/RHgame.cpp:4202-4251` uses
            // `rand() % (LIFEPOINTS_PC << 1) < life_points`; healthier
            // peasants survive into reservists, frailer ones die outright.
            let roll = crate::sim_rng::u32(
                sim,
                crate::sim_rng::RngSite::PeasantReservistSurvival,
                0..LIFEPOINTS_PC_X2,
            ) as i32;
            let campaign = &mut self.mission_domain.campaign;
            if roll < *life_points as i32 {
                campaign.move_to_reservists(*char_idx);
            } else {
                campaign.remove_from_gang(*char_idx);
            }

            if let Some(profile_idx) = profile_idx_opt {
                self.remove_pc_by_profile(*profile_idx);
            }
        }

        // Reset the mission team.
        let campaign = &mut self.mission_domain.campaign;
        campaign.reset_mission_team();
        // Credit `floor(number_to_convert / quotation)` blazons.
        if quotation != 0 {
            let credited = (number_to_convert as i32) / (quotation as i32);
            campaign.add_value(crate::campaign::CampaignValue::Blazon, credited);
        }
    }

    // ─── Read-only accessors for host-side code ────────────────────

    /// Win/loss tracking and mission metadata.  Host UI reads these
    /// flags to render the HUD / debrief / quit buttons.
    pub fn mission(&self) -> &MissionState {
        &self.mission_domain.state
    }

    /// Current mission's background map name (without extension), as
    /// set by the mission profile at level-load.
    pub fn mission_map_name(&self) -> &str {
        &self.mission_domain.state.map_name
    }

    /// Monotonically increasing frame counter (one per processed tick).
    pub fn frame_counter(&self) -> u32 {
        self.control.frame_counter
    }

    /// Sim-state portion of the sound system (source list + finished
    /// exclamation queue).  Host sound pipeline reads this when
    /// flushing sources.
    pub fn sound_sim(&self) -> &crate::sound::SoundSimState {
        &self.feedback.sound_sim
    }

    /// Read-only access to the sample-length lookup used for
    /// Loaded mission script (bytecode + VM).  `None` if scripts are
    /// disabled or the level has no script.  Host renderers and the
    /// console read the script VM state for inspection.
    pub fn mission_script(&self) -> Option<&MissionScript> {
        self.scripts.mission.as_ref()
    }

    /// Mutable access to ordered script effects for Lua/tool adapters.
    ///
    /// Exposed `pub` so the host crate's Lua scripting layer
    /// (`robin_rs::lua_session`) can drive custom-mission Lua events
    /// against the same effect buffer the `.scb` VM uses. The
    /// `RollbackSafeEngine` invariant still holds — Lua sessions are
    /// single-player only (see `docs/lua.md`) and never run during
    /// rollback resimulation.
    pub fn mission_script_effects_mut(&mut self) -> Option<&mut crate::natives::ScriptEffects> {
        self.scripts
            .mission
            .as_mut()
            .map(MissionScript::script_effects_mut)
    }

    /// True iff men-to-blazon conversion mode is active. Read by titbit
    /// rendering to suppress the per-PC
    /// WorkIcon while the conversion screen is up.
    pub fn is_men_to_blazon_conversion_mode(&self) -> bool {
        self.script_domains.mission_ui.men_to_blazon_conversion_mode
    }

    /// Number of temporary blazon highlights active on this frame.
    pub fn active_blinking_blazons(&self) -> u32 {
        self.script_domains
            .mission_ui
            .active_blinking_blazons(self.control.frame_counter)
    }

    /// Refresh the per-patch `display_doors` flag for this frame's
    /// selection state.  `DisplayAllDoorsAndJumpZones` clears every
    /// patch first, then the currently-selected patch sets its own
    /// `display_doors`.  The flag drives the door-outline render pass
    /// and the patch FX consumer.
    ///
    /// `Patch::display_doors` is explicitly excluded from serialization and
    /// state hashing, so local cursor movement cannot perturb rollback or a
    /// multiplayer peer's deterministic state.
    pub fn refresh_selected_patch_display_doors(&mut self, selected_patch_idx: Option<u32>) {
        for patch in self.script_domains.interactables.patches.iter_mut() {
            patch.display_doors = false;
        }
        if let Some(idx) = selected_patch_idx
            && let Some(patch) = self
                .script_domains
                .interactables
                .patches
                .get_mut(idx as usize)
        {
            patch.display_doors = true;
        }
    }

    /// Queue the `UpdateInformationBars` engine command on the script
    /// host.  Called from the host after a save-load so the script
    /// refreshes its side of the information-bar UI.
    pub fn queue_update_information_bars(&mut self) {
        if let Some(effects) = self.mission_script_effects_mut() {
            effects.emit_engine(crate::natives::EngineCommand::UpdateInformationBars);
        }
    }

    /// Toggle the engine-owned men-to-blazon conversion mode. Read by the
    /// `IsMenToBlazonConversionMode` native and the
    /// blazon-bar recomputation in `UpdateInformationBars`.
    pub(crate) fn set_men_to_blazon_conversion_mode(&mut self, enabled: bool) {
        self.script_domains.mission_ui.men_to_blazon_conversion_mode = enabled;
    }

    /// Run the mission script's `PostInitialize` hook once, then no-op.
    /// The serialized flag keeps both live play and rollback replay
    /// idempotent; [`EngineInner::perform_post_initialize`] owns the
    /// original post-refresh host boundary.
    pub(crate) fn run_post_initialize_if_needed(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let Some(script) = self.scripts.mission.as_mut() else {
            return;
        };
        if script.post_initialized {
            return;
        }
        script.post_initialized = true;

        let result = self
            .call_script_vm(
                sim,
                assets,
                ScriptVmKey::Global,
                "PostInitialize",
                &[],
                crate::natives::ScriptCallFrame::default(),
            )
            .map(|_| ());

        if let Err(e) = result {
            tracing::warn!("Script PostInitialize failed: {e}");
        }
    }

    /// Mutate a campaign value with the side effects of `AddValue`.
    /// In addition to the raw field write, RANSOM credits to the
    /// per-mission collected-money counter and (for positive deltas
    /// after the first frame) emits the `CashWon` jingle; SCORE credits
    /// to the per-mission added-score counter.  Other campaign values
    /// have no extra side effects.
    pub fn add_campaign_value(&mut self, name: crate::campaign::CampaignValue, amount: i32) {
        self.mission_domain.campaign.values[name] += amount;
        Self::apply_value_add_side_effects(
            &mut self.mission_domain.mission_stat,
            &mut self.feedback.pending_side_effects,
            self.control.frame_counter,
            name,
            amount,
        );
    }

    /// Force a campaign value with the side effects of `SetValue`.
    /// RANSOM emits the `CashWon` jingle when the new value is greater
    /// than the old one (and the universal frame counter has advanced
    /// past 0).
    pub fn set_campaign_value(&mut self, name: crate::campaign::CampaignValue, value: i32) {
        let old = self.mission_domain.campaign.values[name];
        self.mission_domain.campaign.values[name] = value;
        Self::apply_value_set_side_effects(
            &mut self.feedback.pending_side_effects,
            self.control.frame_counter,
            name,
            old,
            value,
        );
    }

    fn apply_value_add_side_effects(
        mission_stat: &mut MissionStat,
        side_effects: &mut SideEffects,
        frame_counter: u32,
        name: crate::campaign::CampaignValue,
        amount: i32,
    ) {
        // Credit the mission-stat counters unconditionally for
        // RANSOM/SCORE — only the CashWon jingle is gated on
        // `amount > 0 && frame_counter > 0`.
        match name {
            crate::campaign::CampaignValue::Ransom => {
                mission_stat.add_collected_money(amount);
                if amount > 0 && frame_counter > 0 {
                    side_effects
                        .sounds
                        .push(SoundCommand::Jingle(crate::sound::Jingle::CashWon));
                }
            }
            crate::campaign::CampaignValue::Score => {
                mission_stat.add_score(amount);
            }
            _ => {}
        }
    }

    fn apply_value_set_side_effects(
        side_effects: &mut SideEffects,
        frame_counter: u32,
        name: crate::campaign::CampaignValue,
        old: i32,
        new: i32,
    ) {
        if name == crate::campaign::CampaignValue::Ransom && new > old && frame_counter > 0 {
            side_effects
                .sounds
                .push(SoundCommand::Jingle(crate::sound::Jingle::CashWon));
        }
    }

    /// The campaign owned by this live mission engine.
    pub fn campaign(&self) -> &crate::campaign::Campaign {
        &self.mission_domain.campaign
    }

    /// Has the given peasant display name already been registered on
    /// the campaign's no-duplicates list?  Read-only.
    pub fn is_peasant_name_registered(&self, name: &str) -> bool {
        self.mission_domain
            .campaign
            .is_peasant_name_registered(name)
    }

    /// Add a display name to the campaign's peasant-name dedupe list.
    /// Called once per peasant at level-load, before the mission
    /// begins ticking.
    pub(crate) fn register_peasant_name(&mut self, name: String) {
        self.mission_domain
            .required_campaign_mut("registering a mission peasant name")
            .register_peasant_name(name);
    }

    /// Explicitly replace campaign progress for the `CAMPAIGN` developer
    /// console command. Mission construction and teardown never use this.
    pub(crate) fn replace_campaign(&mut self, campaign: crate::campaign::Campaign) {
        self.mission_domain.campaign = campaign;
    }

    /// Consume a finished engine and return its one campaign allocation.
    pub(crate) fn into_campaign(self) -> crate::campaign::Campaign {
        self.mission_domain.campaign
    }

    /// Reset transient runtime state that isn't — or shouldn't be —
    /// carried across a save/load boundary.  Called by
    /// [`Engine::restore`](crate::engine::Engine::restore) right after
    /// overlaying the saved engine's fields, so the next tick starts
    /// with a clean slate regardless of what the pre-load session was
    /// doing (mid-drag selection, mid-zoom, mid-tick side-effect
    /// queue, …).  This is the engine-owned half of the post-load
    /// resynchronisation.
    pub(crate) fn post_load_fixups(&mut self, display: &mut HostDisplayState) {
        // Alt-hover vision cone selection is host-owned now — the host
        // wipes `host.selected_view_element` in `Host::post_load_reset`.
        // The selection ring animation phase is host-owned now and is
        // reset in `Host::post_load_reset` too.

        // Per-frame / per-tick scratch flags.
        self.script_domains.mission_ui.force_check = false;
        self.control.chorus_timer = 0;
        self.control.fast_forward = false;
        self.orders.pending_move_requests.clear();
        self.orders.pending_path_requests.clear();
        self.orders.failed_path_requests.clear();

        // Force a full redraw on the next frame — the background cache
        // from the pre-load session is no longer valid for the restored
        // camera/mission state.
        display.display_op = DisplayOpCode::Redraw;

        // Abort any mid-zoom state carried over from the save.  Run
        // here so the restored engine starts the next tick with a
        // clean zoom state, rather than relying on a host-driven
        // cache-validity hook.
        if self.is_zooming(display) {
            let zoom_up = self.is_zoom_up_possible() as u32;
            let zoom_down = self.is_zoom_down_possible() as u32;
            self.orders
                .messenger
                .send(crate::messenger::Message::with_value(
                    crate::messenger::MessageType::Simple(
                        crate::messenger::SimpleMessage::ZoomUpEnd,
                    ),
                    (zoom_up << 16) | zoom_down,
                ));
            let bg = &mut self.feedback.cutscene_camera.display.background_transform;
            bg.zoom_to_up = false;
            bg.zoom_to_down = false;
            bg.required_zoom_up = false;
            bg.required_zoom_down = false;
            self.feedback.cutscene_camera.display.display_op = DisplayOpCode::NoBackgroundMove;
            self.feedback.cutscene_camera.zoom_init_done = false;
        }

        // Drop any mid-tick side-effect scratch (sounds, UI requests,
        // …) that was being built before the quick-load.  Normally
        // drained by `perform_hourglass`; this covers the partial-tick
        // case where the load pre-empted the drain.
        self.feedback.pending_side_effects = SideEffects::default();

        // Anonymous sequence-timer entries are tied to `SequenceManager`
        // state that was just replaced; the reloaded manager rebuilds
        // its own timer list as sequences resume.
        // TODO(original-parity): verify whether the original load path clears
        // these timers or reconstructs their remaining duration from sequences.
        // Preserve the established Rust save-load behavior until that is known.
        self.orders.timer_elements.clear();

        // Walk every PC and reconcile the loaded selection list
        // against the per-PC `interface_hidden` / `playable` /
        // life-points flags.  The HUD is immediate-mode and re-derives
        // every frame, so the only state that can drift is
        // `selected_pc_ids` itself — serde restored it as it was at
        // save time, but the per-PC `interface_hidden` / `playable`
        // flags also restored from disk may now be inconsistent with
        // the cached selection (e.g. a mid-recording quick-save where
        // the messenger had a pending unselect).  Drop any selected id
        // whose PC has had its portrait hidden or been made unplayable.
        self.players.seats[0]
            .selection
            .retain(|&id| match self.world.entities.get(id) {
                Some(crate::element::Entity::Pc(pc)) => {
                    !pc.pc.interface_hidden && pc.pc.playable && pc.pc.life_points > 0
                }
                _ => false,
            });

        // Re-broadcast `MSG_STATURE(0)` and a `MSG_SELECT_ACTION`
        // trailer for the currently-cached selected action so any
        // script / HUD consumer listening on the messenger queue
        // resynchronises its view of posture + action after a
        // save-load.  The immediate-mode HUD already re-derives from
        // engine state each frame so these are belt-and-braces —
        // needed for script subscribers that only react to message
        // edges rather than polling.
        self.orders.messenger.send(crate::messenger::Message::new(
            crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
        ));
        let action = self.get_selected_action();
        let pc_id = self.players.seats[0].selection.first().copied();
        self.orders
            .messenger
            .send(crate::messenger::Message::pc_with_value(
                crate::messenger::PcMessage::SelectAction,
                pc_id,
                action as u32,
            ));
    }

    // ─── Test-only helpers ────────────────────────────────────────
    //
    // These are `#[doc(hidden)]` but still `pub` because the downstream
    // `robin_rs` crate ships tests that drive engine state through
    // known-safe back doors (setting mission/quit flags, seeding
    // round-trip state for save-load tests, etc.).  They are not part
    // of the public API and never called from production code.

    /// Test helper: set `mission_won` / `quit_won` / `quit_lost` flags.
    #[doc(hidden)]
    pub fn test_set_mission_flags(&mut self, quit_won: bool, quit_lost: bool, mission_won: bool) {
        self.mission_domain.state.quit_won = quit_won;
        self.mission_domain.state.quit_lost = quit_lost;
        self.mission_domain.state.mission_won = mission_won;
    }

    /// Test helper: seed `frame_counter` (save-round-trip tests).
    #[doc(hidden)]
    pub fn test_set_frame_counter(&mut self, frame: u32) {
        self.control.frame_counter = frame;
    }

    /// Test helper: seed miscellaneous scalar engine fields used by
    /// save-round-trip tests.
    #[doc(hidden)]
    pub fn test_set_engine_scalars(
        &mut self,
        cheat_used_flags: u32,
        speed: f32,
        speed_int: u16,
        lock_engine: bool,
        freeze_all: bool,
        script_globals: Vec<i32>,
    ) {
        self.mission_domain.cheat_used_flags = cheat_used_flags;
        self.control.speed = speed;
        self.control.speed_int = speed_int;
        self.set_engine_locked(lock_engine);
        self.set_actors_frozen(freeze_all);
        self.scripts.globals = script_globals;
    }

    /// Test helper: seed the mission stat without running a mission.
    #[doc(hidden)]
    pub fn test_set_mission_stat(&mut self, stat: MissionStat) {
        self.mission_domain.mission_stat = stat;
    }

    /// Current RNG seed.  Used by the replay recorder to stamp the
    /// deterministic seed into the `.rhrec.jsonl` header.  Read-only.
    pub fn rng_seed(&self) -> u64 {
        self.control.rng.seed()
    }

    /// Which of the 10 known playable characters a PC entity represents.
    /// Returns `None` for entities that aren't PCs or whose character
    /// profile wasn't recognised at level-load time.
    pub fn pc_character_kind(
        &self,
        pc_id: EntityId,
    ) -> Option<crate::character_kind::CharacterKind> {
        self.get_entity(pc_id).and_then(|e| e.pc_data())?.kind
    }

    /// Clear the one-shot `display_double_status_bar` flag on every NPC.
    /// Resets the flag right after the bar renderer draws.  The
    /// renderer is a host-side `&EngineInner` pass, so the clear runs
    /// here.
    pub(crate) fn clear_npc_double_status_bar_flags(&mut self) {
        let ids = self.world.entities.npc_ids().collect::<Vec<_>>();
        for id in ids {
            if let Some(e) = self.get_entity_mut(id)
                && let Some(npc) = e.npc_data_mut()
            {
                npc.display_double_status_bar = false;
            }
        }
    }

    /// Restore the simulation RNG from a known seed.  Used when
    /// loading a replay or a save — replay/load is a mission-lifecycle
    /// boundary, outside the per-tick input pipeline.
    pub fn restore_rng_from_seed(&mut self, seed: u64) {
        self.control.rng.reseed(seed);
    }
}

#[cfg(test)]
mod campaign_lifecycle_tests {
    use std::sync::Arc;

    use super::{EngineInner, LevelAssets};
    use crate::campaign::{Campaign, CampaignValue};
    use crate::game_operation::GameCode;
    use crate::mission::{Mission, MissionStatus};
    use crate::player_command::PlayerCommand;
    use crate::player_profile::DifficultyLevel;
    use crate::profiles::{MissionProfile, MissionType, ProfileManager};

    fn marked_campaign() -> Campaign {
        let mut campaign = Campaign::default();
        campaign.values[CampaignValue::Custom20] = 0x25_25_25;
        campaign
    }

    fn active_historical_mission() -> (Campaign, LevelAssets) {
        let mut profiles = ProfileManager::default();
        profiles.missions.push(MissionProfile {
            mission_type: MissionType::Historical,
            min_new_team_members: 0,
            max_new_team_members: 0,
            ..MissionProfile::default()
        });

        let mut mission = Mission::new();
        mission.profile_idx = Some(0);
        let mut campaign = marked_campaign();
        campaign.missions.push(mission);
        campaign.current_mission_idx = Some(0);

        let assets = LevelAssets {
            profile_manager: Arc::new(profiles),
            ..LevelAssets::default()
        };
        (campaign, assets)
    }

    #[test]
    fn quit_updates_preserve_the_campaign_allocation() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut campaign = marked_campaign();
        campaign.production_sectors.reserve_exact(257);
        assert!(!campaign.production_sectors.is_empty());
        let production_sectors = campaign.production_sectors.as_ptr();
        let production_sector_capacity = campaign.production_sectors.capacity();
        let mut engine = EngineInner::new_with_campaign(campaign);

        engine.apply_quit_mission_updates(
            sim,
            &LevelAssets::default(),
            GameCode::LevelFailed,
            DifficultyLevel::Medium,
        );
        let campaign = engine.into_campaign();

        assert_eq!(campaign.production_sectors.as_ptr(), production_sectors);
        assert_eq!(
            campaign.production_sectors.capacity(),
            production_sector_capacity
        );
        assert_eq!(campaign.values[CampaignValue::Custom20], 0x25_25_25);
    }

    #[test]
    fn successful_quit_updates_keep_original_order_and_state() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut campaign, assets) = active_historical_mission();
        campaign.values[CampaignValue::LivingSoldiers] = 7;
        campaign.values[CampaignValue::DeadSoldiers] = 11;
        campaign.values[CampaignValue::Score] = 13;

        let mut engine = EngineInner::new_with_campaign(campaign);
        engine.mission_domain.mission_stat.living_soldier_count = 2;
        engine.mission_domain.mission_stat.total_soldier_count = 5;
        engine.mission_domain.mission_stat.new_peasant_count = 99;
        engine.apply_quit_mission_updates(
            sim,
            &assets,
            GameCode::LevelSucceeded,
            DifficultyLevel::Medium,
        );

        let campaign = engine.campaign();
        assert_eq!(campaign.missions[0].status, MissionStatus::Won);
        assert_eq!(campaign.values[CampaignValue::LivingSoldiers], 9);
        assert_eq!(campaign.values[CampaignValue::DeadSoldiers], 14);
        assert_eq!(campaign.values[CampaignValue::Score], 1013);
        assert_eq!(engine.mission_domain.mission_stat.added_score, 1000);
        assert_eq!(engine.mission_domain.mission_stat.new_peasant_count, 0);
    }

    #[test]
    fn serialized_quit_command_applies_deterministically() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (campaign, assets) = active_historical_mission();
        let mut first = EngineInner::new_with_campaign(campaign.clone());
        let mut second = EngineInner::new_with_campaign(campaign);

        let command = PlayerCommand::ApplyQuitMissionUpdates {
            exit_code: GameCode::LevelSucceeded,
            difficulty: DifficultyLevel::Hard,
        };
        let encoded = serde_json::to_string(&command).expect("serialize quit command");
        let decoded: PlayerCommand =
            serde_json::from_str(&encoded).expect("deserialize quit command");

        for engine in [&mut first, &mut second] {
            let mut display = super::HostDisplayState::default();
            let mut input = super::InputState::default();
            engine.apply_command(sim, &mut display, &mut input, &assets, &decoded);
        }

        assert_eq!(
            crate::replay::state_hash(&first),
            crate::replay::state_hash(&second)
        );
        assert_eq!(
            first.mission_domain.mission_stat,
            second.mission_domain.mission_stat
        );
    }

    #[test]
    fn save_load_round_trip_preserves_the_required_campaign() {
        let engine = EngineInner::new_with_campaign(marked_campaign());

        let json = serde_json::to_string(&engine).expect("serialize active engine");
        let loaded: EngineInner = serde_json::from_str(&json).expect("deserialize active engine");
        let campaign = loaded.into_campaign();

        assert_eq!(campaign.values[CampaignValue::Custom20], 0x25_25_25);
        assert_eq!(campaign.production_sectors.len(), 13);
    }
}

/// Complete the profile and AI attachments required by full-tick unit tests.
///
/// Production entities receive these attachments during level loading. Tests
/// that construct active actors directly must do the equivalent before
/// calling `perform_hourglass`; keeping it here prevents individual fixtures
/// from weakening the runtime's required-data invariants.
#[cfg(test)]
pub(crate) fn complete_test_runtime_fixture(engine: &mut EngineInner, assets: &mut LevelAssets) {
    let mut profiles = (*assets.profile_manager).clone();
    let mut needs_hth_weapon = false;

    for (_, pc) in engine.world.entities.pcs() {
        if !pc.element.active || pc.pc.life_points <= 0 {
            continue;
        }
        let profile_idx = usize::from(pc.pc.profile_index);
        profiles
            .characters
            .resize_with(profile_idx + 1, crate::profiles::CharacterProfile::default);
        if profiles.characters[profile_idx].hth_weapon_id == 0 {
            profiles.characters[profile_idx].hth_weapon_id = 1;
        }
        needs_hth_weapon = true;
    }

    for (soldier_id, soldier) in engine.world.entities.soldiers_mut() {
        if soldier.npc.ai_brain.is_none() {
            soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(
                crate::ai_enemy::EnemyAi::new(soldier_id.0),
            ));
        }
        let enemy_ai =
            soldier.npc.ai_brain.enemy_mut().unwrap_or_else(|| {
                panic!("test soldier {} has a non-enemy AI brain", soldier_id.0)
            });
        if !soldier.element.active || soldier.human.unconscious {
            continue;
        }
        let profile_idx = usize::from(soldier.soldier.soldier_profile_index);
        profiles
            .soldiers
            .resize_with(profile_idx + 1, crate::profiles::SoldierProfile::default);
        if profiles.soldiers[profile_idx].hth_weapon_id == 0 {
            profiles.soldiers[profile_idx].hth_weapon_id = 1;
        }
        if enemy_ai.hth_weapon_id == 0 {
            enemy_ai.hth_weapon_id = 1;
        }
        needs_hth_weapon = true;
    }

    for (civilian_id, civilian) in engine.world.entities.civilians_mut() {
        if civilian.npc.ai_brain.is_none() {
            civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(
                crate::ai_friendly::FriendlyAi::new(civilian_id.0),
            ));
        }
        assert!(
            civilian.npc.ai_brain.friendly().is_some(),
            "test civilian {} has a non-friendly AI brain",
            civilian_id.0
        );
    }

    if needs_hth_weapon && profiles.hth_weapons.is_empty() {
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
    }
    assets.profile_manager = std::sync::Arc::new(profiles);
}
