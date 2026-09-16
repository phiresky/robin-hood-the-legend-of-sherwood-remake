//! Core game engine.
//!
//! This is the central game loop that drives everything: the state machine,
//! per-frame update tick (`perform_hourglass`), rendering dispatch (`draw`),
//! level initialization, camera/zoom control, and subsystem management.
//!
//! Entity/rendering calls are stubbed where systems are not yet implemented —
//! this module captures the *architecture*: the data structures, control
//! flow, and state transitions.

mod ability_execution;
mod ai;
pub(crate) use ai::debug_detectable_mutation_load_snapshot;
mod achievements;
pub use achievements::PcExperienceSnapshot;
mod ale;
mod animation;
pub(crate) mod anti_collision;
mod archery;
mod beggar;
mod camera;
mod cloak;
mod commands;
mod console_dispatch;
mod corpse_intersection;
pub(crate) mod diagnostics;
mod display_state;
mod tactical_control;
pub use display_state::DrawOrder;
mod door_pass;
#[cfg(test)]
mod filter_ai_event_tests;
pub(crate) mod fog_of_war;
mod frame;
mod global_options;
mod host_accessors;
pub mod input;
pub(crate) mod jump;
pub mod level_loading;
pub mod melee;
mod mission_runtime;
mod mission_start;
mod movement;
mod nets;
mod order_arbitration;
mod patch_effects;
pub mod peripherals;
mod posture_transitions;
mod presentation_view;
mod projectile_runtime;
pub use presentation_view::PresentationView;
mod purse;
mod queries;
mod refresh_seek;
mod reinforcement;
mod rollback_safe;
mod rolling;
pub(crate) mod script;
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
pub use snapshot::PersistedEngineState;
mod soldier_helpers;
mod special_motion;
pub(crate) mod state;
#[doc(hidden)]
pub use state::ScriptDomains;
#[cfg(test)]
mod resource_environment_tests;
pub mod target_interaction;
#[cfg(test)]
mod target_script_tests;
mod teleport;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
mod tick;
mod titbit_sync;
mod trading;
mod transitions;
mod types;
mod wasp_nest;

pub(crate) use commands::command_action_distance_animation;
pub use commands::{coin_pickup_target, object_pickup_command};
pub use console_dispatch::ConsoleResponse;
pub use frame::{
    ExternalAction, ExternalActionResult, ExternalFacts, FrameAdvanceError, FrameConsoleResponse,
    RecordedDropAleRoute, SimCommand, SimEvents, SimulationCommandPhase, SimulationFrameInput,
    SimulationFrameOutput, SimulationTick, SoundBoundary, SoundBoundaryPolicy,
};
pub use global_options::*;
pub(crate) use movement::{FailedPathRequest, PendingPathRequest, PendingPathRequestQueue};
pub(crate) use movement::{
    adapt_source_to_current_door_with_identity, current_door_for_route_source,
};
pub use peripherals::{CameraDisplayState, DebugFlags, DevState, HostDisplayState};
pub use rollback_safe::{
    Engine, EngineArgs, GroundMarkSpriteData, HostConsoleDispatch, LevelLoadArgs,
    MinimapWidgetSetup, MissionBootstrapCompletion, ParityReplaySetup, PresentationEngine,
    SnapshotGridComponent, SnapshotRestoreError, SpatialPresentationSnapshot,
};
pub use scroll_reveal::{BeggarRemark, ScrollStatus};
pub use seat::SeatState;
pub use selection::Stature;
pub use types::*;

/// Whether Original dispatches this actor/order pair through
/// sprite motion rather than a plain action or another
/// specialized executor.
///
/// Parity replay uses this to interpret legacy recorder fields whose validity
/// changed only at the motion-initialization boundary. Keep the
/// answer tied to the single authoritative original-game behavior catalog.
#[doc(hidden)]
pub fn original_actor_order_uses_motion_executor(entity_id: EntityId, order: OrderType) -> bool {
    matches!(
        tick::classify_live_actor_execute_arm(entity_id, order),
        Some(tick::ExecuteOwnerFamily::Movement)
    )
}

use crate::ai::AiGlobalState;
use crate::element::{Entity, EntityId};
use crate::fast_find_grid::FastFindGrid;
use crate::markers::GroundMark;
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

fn attentive_owner_handoff_debug_config() -> Option<&'static diagnostics::ExactOwnerFrame> {
    diagnostics::config().attentive_owner.as_ref()
}

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
/// This type is public only because [`Engine`](rollback_safe::Engine) exposes
/// it as a borrowed, read-only `Deref` target. Its fields, constructors, and
/// mutators are crate-private, and production builds deliberately do not
/// implement `Clone`, Serde `Serialize`/`Deserialize`, or native bitcode codecs for it.
/// Whole-state ownership and snapshot encoding/decoding belong to the `Engine`
/// facade.
///
/// The owner's `Serialize` and this projection's `StateHash` follow the current nested ownership layout.
/// Multiplayer peers, rollback, and current-format replays therefore observe
/// the same deterministic state boundaries. Unit tests retain test-only
/// `Clone` and Serde implementations for low-level engine fixtures.
#[cfg_attr(test, derive(Clone))]
#[derive(robin_state_hash_derive::StateHash)]
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

    // Implemented subsystems
    /// Deterministic per-player selection, input-mode, and macro state.
    pub(crate) players: PlayerRuntime,

    /// Deterministic sound, marker, director-camera, and tick-output state.
    pub(crate) feedback: FeedbackRuntime,
    // (Deferred bg-blits live on `pending_side_effects.bg_blits` now;
    // load-once index tables live on `LevelAssets::{source_durations,
    // patch_entity_handles, scroll_entity_ids, all_soldier_entity_ids}`.)
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
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PendingBgBlit {
    pub entity_id: EntityId,
    pub restore_only: bool,
    pub decal: Option<PendingBgBlitDecal>,
}

/// Exact sprite frame to keep as a persistent background decal.
///
/// The original game's immediate background swap temporarily forces the patch
/// FX to the last transition frame, blits it to the map, then restores its
/// previous row/frame. The Rust engine computes that same transition-frame
/// result without mutating the live sprite; the hardware renderer consumes
/// the concrete frame id and destination later.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
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

/// Resolve the original game's actor-order animation identity from the explicit
/// installed pointer mirror. A selected SequenceManager element is not a
/// substitute: selection and instruction/order-advancement reference publication are
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
    /// Copy authoritative state inside the crate without exposing ownership of
    /// the read-only downstream projection.
    pub(crate) fn clone_authoritative_state(&self) -> Self {
        Self {
            mission_domain: self.mission_domain.clone(),
            control: self.control.clone(),
            ai: self.ai.clone(),
            world: self.world.clone(),
            script_domains: self.script_domains.clone(),
            orders: self.orders.clone(),
            scripts: self.scripts.clone(),
            players: self.players.clone(),
            feedback: self.feedback.clone(),
        }
    }

    /// Queue concrete speech sample resolutions produced by the logical sound
    /// manager after the preceding engine frame.
    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn queue_resolved_exclamations(
        &mut self,
        resolutions: Vec<crate::sound::ResolvedExclamation>,
    ) {
        self.try_queue_resolved_exclamations(resolutions, false)
            .unwrap_or_else(|reason| panic!("live sound resolutions rejected: {reason}"));
    }

    /// Queue resolutions recorded by the Original host sound manager.
    ///
    /// Unlike live host resolutions, these are authoritative replay inputs
    /// and may describe Original-only speech for which Rust has no logical AI
    /// request. The tick boundary preserves their completion timing without
    /// synthesizing a Rust speech latch.
    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn queue_replay_resolved_exclamations(
        &mut self,
        resolutions: Vec<crate::sound::ResolvedExclamation>,
    ) {
        self.try_queue_resolved_exclamations(resolutions, true)
            .unwrap_or_else(|reason| panic!("replay sound resolutions rejected: {reason}"));
    }

    pub(super) fn try_queue_resolved_exclamations(
        &mut self,
        resolutions: Vec<crate::sound::ResolvedExclamation>,
        replay_injected: bool,
    ) -> Result<(), String> {
        if !self.feedback.sound_sim.resolved_exclamations.is_empty() {
            return Err(
                "resolved exclamations were not consumed before the next sound boundary".to_owned(),
            );
        }
        self.feedback.sound_sim.resolved_exclamations = resolutions;
        self.feedback
            .sound_sim
            .replay_injected_resolved_exclamations = replay_injected;
        Ok(())
    }

    /// Cancel the first logical exclamation for an actor without calling
    /// sound-completion state, matching the original game's exclamation-stop scan.
    ///
    /// Original stops the first matching active channel, then erases the
    /// first matching node from the pending-sounds list. A playing Rust entry is
    /// the split-state equivalent of both of those Original records, while
    /// an unresolved entry exists only in `pending_exclamations`.  Later
    /// requests for the same actor must retain their list order.
    pub(super) fn cancel_exclamation_callbacks(&mut self, actor_id: u32) {
        self.debug_speech_lifecycle(actor_id, "cancel_callbacks_enter", "StopExclamation");
        let sound = &mut self.feedback.sound_sim;
        if let Some(index) = sound
            .playing_exclamations
            .iter()
            .position(|playing| playing.actor_id == actor_id)
        {
            sound.playing_exclamations.remove(index);
            self.debug_speech_lifecycle(
                actor_id,
                "cancel_callbacks_playing_removed",
                "StopExclamation",
            );
            return;
        }

        let Some(index) = sound
            .pending_exclamations
            .iter()
            .position(|pending| pending.actor_id == actor_id)
        else {
            self.debug_speech_lifecycle(actor_id, "cancel_callbacks_no_pending", "StopExclamation");
            return;
        };
        let pending = sound.pending_exclamations.remove(index);

        // During parity replay the host's resolution is staged separately
        // from its still-pending logical request.  Remove only the staged
        // resolution belonging to the node erased above.
        if let Some(index) = sound.resolved_exclamations.iter().position(|resolved| {
            resolved.actor_id == pending.actor_id
                && resolved.exclamation_id == pending.exclamation_id
                && resolved.identifier
                    == (pending.profile_id & 0xFFFF_0000) | u32::from(pending.exclamation_id)
        }) {
            sound.resolved_exclamations.remove(index);
        }
        self.debug_speech_lifecycle(
            actor_id,
            "cancel_callbacks_pending_removed",
            "StopExclamation",
        );
    }

    pub(crate) fn engine_locked(&self) -> bool {
        self.control.engine_locked()
    }

    /// Resolve the authoritative symmetric relationship between two actors'
    /// allegiances. Gameplay code must use this instead of comparing raw IDs.
    pub fn relationship(
        &self,
        first: crate::element::Camp,
        second: crate::element::Camp,
    ) -> crate::diplomacy::Relationship {
        self.mission_domain.diplomacy.relationship(first, second)
    }

    pub fn camps_are_hostile(
        &self,
        first: crate::element::Camp,
        second: crate::element::Camp,
    ) -> bool {
        self.mission_domain.diplomacy.is_hostile(first, second)
    }

    pub fn camps_are_allied(
        &self,
        first: crate::element::Camp,
        second: crate::element::Camp,
    ) -> bool {
        self.mission_domain.diplomacy.is_allied(first, second)
    }

    pub fn is_player_aligned_camp(&self, camp: crate::element::Camp) -> bool {
        self.mission_domain.diplomacy.is_player_aligned(camp)
    }

    pub fn relationship_to_player(
        &self,
        camp: crate::element::Camp,
    ) -> crate::diplomacy::Relationship {
        self.mission_domain.diplomacy.relationship_to_player(camp)
    }

    pub fn is_hostile_to_player_camp(&self, camp: crate::element::Camp) -> bool {
        self.mission_domain.diplomacy.is_hostile_to_player(camp)
    }

    pub fn is_allied_to_player_camp(&self, camp: crate::element::Camp) -> bool {
        self.mission_domain.diplomacy.is_allied_to_player(camp)
    }

    #[cfg(any(test, feature = "test-helpers"))]
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
        // The original game keeps player description and status as aliases
        // into campaign state and serializes that description reference separately.
        // The list index is independent actor/UI storage and is never used to
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
            // The original game's test path uses a fixed zero seed.
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
        if let Some(runtime) = assets.attachments.spellforge_runtime.as_ref() {
            runtime.set_name_bindings(assets.scripts.names.as_ref().clone());
        }
        self.scripts
            .initialize_spellforge_package(assets)
            .unwrap_or_else(|error| panic!("Spellforge package initialization failed: {error}"));
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
        let startup_started = web_time::Instant::now();
        self.validate_actor_placement();
        tracing::debug!(
            elapsed_ms = startup_started.elapsed().as_secs_f64() * 1000.0,
            "engine init: validate placement"
        );
        let startup_started = web_time::Instant::now();

        // Pathfinder obstacle states now that the graph is loaded.
        if !assets
            .navigation
            .pathfinder_graph
            .static_data
            .move_layers
            .is_empty()
        {
            let world = &mut self.world;
            let grid = std::sync::Arc::make_mut(&mut world.fast_grid);
            world
                .pathfinder
                .initialize_from_graph(assets.navigation.pathfinder_graph.as_ref(), grid);
        }

        tracing::debug!(
            elapsed_ms = startup_started.elapsed().as_secs_f64() * 1000.0,
            "engine init: pathfinder defaults"
        );
        let startup_started = web_time::Instant::now();

        // Original-game initialization runs script initialization before AI
        // initialization. This ordering is required
        // now that AI initialization's typed state changes synchronously dispatch
        // FilterAIEvent through the bound actor VMs.
        if self.scripts.mission.is_some() {
            self.initialize_mission_script_with(sim, assets, 0, &assets.navigation.hiking_paths);
        }

        tracing::debug!(
            elapsed_ms = startup_started.elapsed().as_secs_f64() * 1000.0,
            "engine init: mission script"
        );

        // The original initializes scrolls immediately after the engine
        // script and before AI. Random sprite-frame selection is the remaining
        // entity-side half of that step.
        self.initialize_all_scrolls(sim);

        // Initialize AI for all NPCs and global AI state. Runs here —
        // not pre-bitmap — because `init_one_ai`'s `TestIfPathIsFine`
        // reads `fast_grid.map_bbox` + motion lines, and the
        // pathfinder's `move_box_half_diagonals` table must already be
        // populated so `spawn_soldier`'s move_box ends up at the real
        // profile-sized pathfinder box instead of the `(-1,-1,1,1)`
        // fallback.
        let startup_started = web_time::Instant::now();
        self.init_ai(sim, assets);
        tracing::debug!(
            elapsed_ms = startup_started.elapsed().as_secs_f64() * 1000.0,
            "engine init: AI"
        );

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
                if !pc.pc.playable
                    || pc.pc.command_interface
                        != crate::human_control::CommandInterface::HeroActions
                {
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
            self.select_pc(sim, assets, 0, pc_id, false, false);
            assert_eq!(
                self.players.seats[0].selection.as_slice(),
                &[pc_id],
                "initial PC selection message did not select its target"
            );
        }

        // Establish frame-zero visibility after every entity, authored sight
        // obstacle, and player allegiance has been loaded.
        self.refresh_fog_of_war(assets, true);

        tracing::info!("EngineInner: initialization complete");
    }

    /// Verify every actor's mission-start placement is legal.
    ///
    /// * Out-of-range layer (above the special layer) — shipped
    ///   data never trips it.  We use `tracing::error!` (no panic) so a
    ///   bad mission file still boots while yelling in the logs.
    /// * Move-box colliding with an obstacle is a non-fatal warn.
    fn validate_actor_placement(&self) {
        let special_layer = self.world.fast_grid.level.special_layer;
        for (_, entity) in self.world.entities.actors() {
            let elem = entity.element_data();
            let Some(layer) = elem.optional_layer() else {
                continue;
            };
            let layer = layer.get();
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
    /// subsystem implementation) and then random frame selection so every
    /// scroll starts on a random frame of its fluttering animation
    /// instead of all waving in lockstep.
    fn initialize_all_scrolls(&mut self, sim: &crate::sim_rng::SimulationContext) {
        for (_, scroll) in self.world.entities.scrolls_mut() {
            // The original game selects a random sprite frame after script initialization.
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
        remaining_frames: i32,
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
    pub(super) fn terminate_prev_camera_sequence_element(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        if let Some(r) = self.feedback.cutscene_camera.sequence_element.take() {
            self.element_terminated(sim, assets, &mut Vec::new(), r.sequence_id, r.element_index);
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
        completed_at_unix_seconds: Option<i64>,
        campaign_run_nonce: Option<u64>,
    ) {
        let won = exit_code == crate::game_operation::GameCode::LevelSucceeded;

        // Calculating results is deterministic and independent from host
        // persistence policy. Only a successful terminal boundary freezes a
        // result; replay/headless/custom/cheat filtering happens later when a
        // caller elects to promote it into campaign/profile history.
        if won {
            // A terminal script may have moved or spawned actors after the
            // preceding regular tick scan. Freeze the exact terminal layout.
            self.refresh_achievement_progress(assets);
        }

        let profiles = &assets.profile_manager;
        let campaign = self.mission_domain.campaign_mut();
        if campaign.current_mission_idx.is_some() {
            campaign.set_mission_done(won, None, profiles);
        }

        let (living, dead) = self.count_soldiers_at_quit();

        self.reset_all_pc_comas(sim, assets);

        if won && self.mission_domain.campaign().current_mission_idx.is_some() {
            // The LIVING/DEAD/SCORE value additions are gated on
            // `mission_won` — a lost mission must NOT accumulate these
            // totals onto the campaign.
            let tied_score = self.score_tied_unconscious_soldiers();
            self.mission_domain.apply_won_updates(
                &mut self.feedback.pending_side_effects,
                self.control.frame_counter,
                sim,
                profiles,
                living,
                dead,
                tied_score,
                difficulty,
            );
        } else {
            // Explicitly zero on the lost path.
            self.mission_domain.mission_stat.new_peasant_count = 0;
        }

        if won {
            self.evaluate_campaign_deeds(assets);
            self.mission_domain.achievements.finalize_success();
        }

        let Some(mission_index) = self.mission_domain.campaign.current_mission_idx else {
            return;
        };
        let outcome = match exit_code {
            crate::game_operation::GameCode::LevelSucceeded => {
                crate::campaign_history::MissionAttemptOutcome::Won
            }
            crate::game_operation::GameCode::LevelFailed => {
                crate::campaign_history::MissionAttemptOutcome::Lost
            }
            crate::game_operation::GameCode::LevelInterrupted => {
                crate::campaign_history::MissionAttemptOutcome::Interrupted
            }
            other => panic!("quit-mission history received non-terminal game code {other:?}"),
        };
        let stat = self.mission_domain.mission_stat.clone();
        let achievements = self
            .mission_domain
            .achievements
            .finalized_results()
            .copied();
        let duration_seconds = self
            .mission_domain
            .campaign
            .get_value(crate::campaign::CampaignValue::MissionLength)
            .max(0) as u32;
        self.mission_domain.campaign.record_mission_attempt(
            mission_index,
            outcome,
            completed_at_unix_seconds,
            campaign_run_nonce,
            duration_seconds,
            self.control.sim_config,
            &stat,
            achievements,
        );
    }

    /// Compute score bonus for living enemy soldiers that are tied or
    /// unconscious: iterates all Lacklandist soldiers, adds
    /// `SCORE_SOLDIER_TIED_AND_UNCONSCIOUS` (70) for each living soldier
    /// that is tied or unconscious.
    ///
    pub fn score_tied_unconscious_soldiers(&self) -> i32 {
        use crate::element::{Actor as _, Human as _};
        const SCORE_SOLDIER_TIED_AND_UNCONSCIOUS: i32 = 70;

        let mut score = 0;
        for (_, s) in self.world.entities.soldiers() {
            if self.is_hostile_to_player_camp(s.camp())
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
    /// ensuring accuracy. Original increments the living stat for every live
    /// soldier it sees but leaves the load-time total-soldier stat unchanged.
    pub(crate) fn count_soldiers_at_quit(&mut self) -> (u32, u32) {
        use crate::element::{Camp, Human as _};

        let mut living = 0u32;
        let mut dead = 0u32;
        let mut living_by_camp = std::collections::BTreeMap::<Camp, u32>::new();
        for (_, s) in self.world.entities.soldiers() {
            if s.life_points() > 0 {
                *living_by_camp.entry(s.camp()).or_default() += 1;
            }
            if self.is_hostile_to_player_camp(s.camp()) {
                if s.life_points() > 0 {
                    living += 1;
                } else {
                    dead += 1;
                }
            }
        }
        self.mission_domain
            .mission_stat
            .reset_faction_living_counts();
        for (camp, count) in living_by_camp {
            self.mission_domain
                .mission_stat
                .set_faction_living_soldiers_at_end(camp, count);
        }
        // The living-soldier increment runs inside the per-soldier loop,
        // accumulating onto whatever was previously in the stat rather than
        // overwriting. `ulTotalSoldierCount` was established at load time and
        // QuitMission does not mutate it.
        self.mission_domain.mission_stat.living_soldier_count = self
            .mission_domain
            .mission_stat
            .living_soldier_count
            .saturating_add(living);
        (living, dead)
    }

    /// Reset coma state on all PCs at mission end.
    ///
    /// Iterates all PCs and calls ResetComa on any that are in coma
    /// (amulet death-save).
    pub(crate) fn reset_all_pc_comas(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let coma_pc_ids: Vec<EntityId> = {
            let campaign = self.mission_domain.campaign();
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
            self.reset_coma(sim, assets, pc_id);
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
        input.controls.is_alt || self.players.seats[0].is_lock_alt
    }

    /// The persistent alt-lock flag on its own, ignoring the transient
    /// physical-alt state.  The sight HUD button reads this to draw
    /// itself as latched.
    pub fn is_lock_alt(&self) -> bool {
        self.players.seats[0].is_lock_alt
    }

    // ─── State changes ───────────────────────────────────────────

    /// Handle a state change request.
    pub(crate) fn change_state(
        &mut self,
        display: &mut CameraDisplayState,
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
                // The caller temporarily holds the engine-owned camera display
                // outside its owner; query this display, not the placeholder.
                if self.is_zoom_possible_for_camera(display) && self.is_zoom_up_possible() {
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
                if self.is_zoom_possible_for_camera(display) && self.is_zoom_down_possible() {
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
                // via its state query, and the other variants (locker, zoom,
                // beacon, …) are toggled by dedicated code paths rather
                // than through `ChangeState`.
                false
            }
        }
    }

    // ─── Entity management ──────────────────────────────────────

    /// Add an entity to the world. Returns its EntityId.
    pub(crate) fn add_entity(&mut self, mut entity: Entity) -> EntityId {
        let id = entity_id_for_occupied_slot(self.world.entities.len() as u32, &entity);
        self.initialize_entity_for_publication(id, &mut entity);
        if matches!(entity, Entity::Soldier(_)) {
            self.world.soldier_registry.register(id, entity.camp());
        }
        if entity.npc_data().is_some() {
            self.world.npc_registry_ids.push(id);
        }
        if entity.actor_data().is_some() {
            self.world.actor_registry_ids.push(id);
        }
        if matches!(entity, Entity::Pc(_) | Entity::Soldier(_)) {
            self.world.fighter_registry_ids.push(id);
        }
        self.world.entities.push(Some(entity));
        self.world.assign_next_original_creation_order(id);
        id
    }

    /// Publish a fixture and explicitly complete identities normally supplied
    /// by level loading. Gameplay publication never changes under `cfg(test)`.
    #[cfg(any(test, feature = "test-helpers"))]
    pub(crate) fn add_test_entity(&mut self, entity: Entity) -> EntityId {
        let id = self.add_entity(entity);
        self.backfill_test_entity_identity(id);
        id
    }

    /// Add an entity whose original-game initialization identity was consumed before
    /// synchronous child publication.
    pub(crate) fn add_entity_with_reserved_creation_order(
        &mut self,
        mut entity: Entity,
        creation_order: u32,
    ) -> EntityId {
        let id = entity_id_for_occupied_slot(self.world.entities.len() as u32, &entity);
        self.initialize_entity_for_publication(id, &mut entity);
        if matches!(entity, Entity::Soldier(_)) {
            self.world.soldier_registry.register(id, entity.camp());
        }
        if entity.npc_data().is_some() {
            self.world.npc_registry_ids.push(id);
        }
        if entity.actor_data().is_some() {
            self.world.actor_registry_ids.push(id);
        }
        if matches!(entity, Entity::Pc(_) | Entity::Soldier(_)) {
            self.world.fighter_registry_ids.push(id);
        }
        self.world.entities.push(Some(entity));
        self.world
            .assign_reserved_original_creation_order(id, creation_order);
        id
    }

    fn initialize_entity_for_publication(&mut self, id: EntityId, entity: &mut Entity) {
        if entity.is_soldier()
            && let Some(ai) = entity.ai_controller()
        {
            let count = match ai.current_music_alert_status {
                crate::ai::AlertLevel::Green => &mut self.ai.global.green_alert_soldiers,
                crate::ai::AlertLevel::Yellow => &mut self.ai.global.yellow_alert_soldiers,
                crate::ai::AlertLevel::Red => &mut self.ai.global.red_alert_soldiers,
            };
            *count = count
                .checked_add(1)
                .expect("soldier alert counter overflow at publication");
            let overall = self.ai.global.overall_villain_alert();
            self.ai.global.overall_alert_status = overall;
            self.ai.global.overall_villain_alert_status = overall;
        }

        // Adding a script element assigns its script-list
        // index as soon as it enters the entity list. AI door passing uses
        // this required identity to resolve the actor's committed gate-side
        // position from the shared entity views.
        entity.element_data_mut().index_in_elements_list = u16::try_from(id.index())
            .unwrap_or_else(|_| {
                panic!(
                    "entity slot {} exceeds legacy element-list index range",
                    id.index()
                )
            });

        if let Entity::Pc(pc) = entity {
            let position = pc.element.position_map();
            pc.actor.produced_noise = Some(crate::ai::Noise {
                origin: crate::ai::NoiseOrigin {
                    x: position.x,
                    y: position.y,
                    sector: pc.element.sector(),
                    layer: pc.element.optional_layer(),
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
        let is_vip = entity.is_vip();
        entity.element_data_mut().init_outline_colors(is_vip);

        // Override the Hidden/Default/Target outline-colour slots with
        // the VIP palette when the civilian is a VIP, applied here after
        // the base civilian colours are written.
        if let Entity::Civilian(c) = &*entity
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
        match &*entity {
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
    }

    /// Give a directly-constructed test actor the identity fields that level
    /// loading writes in production.
    ///
    /// Unit-test fixtures build `Entity` values from `Default` and publish them
    /// explicitly through [`Self::add_test_entity`], so two required identities are never
    /// filled in: a PC's stable campaign description
    /// identity behind coma/guard/ammo lookups) and an NPC brain's own actor
    /// handle. Both are backfilled here so individual fixtures don't have to
    /// and so the runtime keeps its strict required-data invariants. Fixtures
    /// that set either field explicitly keep their value.
    ///
    /// A fixture that seeded its own campaign roster is adopted rather than
    /// extended: the PC claims the first unclaimed description carrying its
    /// character profile, so a seeded ammo or coma status stays reachable.
    #[cfg(any(test, feature = "test-helpers"))]
    fn backfill_test_entity_identity(&mut self, id: EntityId) {
        let handle = id.index();
        match self.world.entities.get_mut(id) {
            Some(Entity::Pc(pc)) => {
                if pc.pc.campaign_description_index.is_some() {
                    return;
                }
                let profile_index = pc.pc.profile_index;
                let claimed: Vec<u32> = self
                    .world
                    .entities
                    .pcs()
                    .filter_map(|(_, pc)| pc.pc.campaign_description_index)
                    .collect();
                let characters = &mut self.mission_domain.campaign.characters;
                let description_index = characters
                    .iter()
                    .position(|description| {
                        description.character_profile_idx == Some(profile_index)
                    })
                    .filter(|index| !claimed.contains(&(*index as u32)))
                    .unwrap_or_else(|| {
                        characters.push(crate::campaign::PcDescription {
                            character_profile_idx: Some(profile_index),
                            instanced: true,
                            ..crate::campaign::PcDescription::default()
                        });
                        characters.len() - 1
                    });
                let Some(Entity::Pc(pc)) = self.world.entities.get_mut(id) else {
                    unreachable!("PC slot changed kind during fixture backfill");
                };
                pc.pc.campaign_description_index = Some(description_index as u32);
            }
            Some(Entity::Soldier(soldier)) => {
                if let Some(base) = soldier.npc.ai_brain.base_mut()
                    && base.me == 0
                {
                    base.me = handle;
                }
            }
            Some(Entity::Civilian(civilian)) => {
                if let Some(base) = civilian.npc.ai_brain.base_mut()
                    && base.me == 0
                {
                    base.me = handle;
                }
            }
            _ => {}
        }
    }

    /// Get a reference to an entity by ID.
    pub fn get_entity<I: Into<EntityId>>(&self, id: I) -> Option<&Entity> {
        self.world.entities.get(id)
    }

    /// Get a reference to an entity that is required to exist at this point
    /// in the simulation (e.g. an active melee participant or a live AI
    /// target established earlier in the same logic flow).  Panics with the
    /// given context when the entity is missing: a vanished required entity
    /// indicates corrupted sim state or a port bug, and must not silently
    /// decay into a default gameplay decision.
    #[track_caller]
    pub(crate) fn expect_entity<I: Into<EntityId>>(&self, id: I, ctx: &str) -> &Entity {
        let id = id.into();
        self.get_entity(id)
            .unwrap_or_else(|| panic!("required entity {id:?} missing ({ctx})"))
    }

    /// Return the authoritative original-game creation order for
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
        match self.world.entities.current_element_for_actor(actor) {
            Some((seq_id, idx)) => self
                .orders
                .sequence_manager
                .get_element(seq_id, idx)
                .map(|e| e.command)
                .unwrap_or(crate::element::Command::Wait),
            None => crate::element::Command::Wait,
        }
    }

    /// Gate and direction stored on the actor's currently selected PassDoor
    /// movement element.
    ///
    /// The movement operand retains its traversal direction after crossing
    /// consumes the position interface's live door pointer.
    pub fn actor_selected_pass_door(
        &self,
        actor: EntityId,
    ) -> Option<(crate::gate::DoorIndex, i16)> {
        let element = self
            .world
            .entities
            .current_element_for_actor(actor)
            .and_then(|(sequence_id, element_index)| {
                self.orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
            })?;
        if element.command != crate::element::Command::PassDoor {
            return None;
        }
        let crate::sequence::SequenceElementData::Movement {
            gate_id, direction, ..
        } = &element.data
        else {
            panic!("selected PassDoor for {actor:?} is not a movement element")
        };
        Some((
            gate_id.unwrap_or_else(|| panic!("selected PassDoor for {actor:?} has no gate")),
            *direction,
        ))
    }

    /// Original-game mixed gate order mapped to runtime door indices.
    ///
    /// Stateful doors and stateless jump gates preserve construction order
    /// within each kind, but Rust installs those kinds in a different mixed
    /// order. Recorded Original gate indices must cross this mapping before
    /// they are lowered into runtime movement sequences.
    pub fn legacy_gate_order(&self, assets: &LevelAssets) -> Vec<crate::gate::DoorIndex> {
        let retained = assets
            .navigation
            .legacy_grid_topology
            .as_ref()
            .expect("Original gate translation requires retained grid topology");
        crate::legacy_save::gate_topology::derive_legacy_gate_order(
            &retained.gates,
            &self.script_domains.interactables.doors,
        )
        .unwrap_or_else(|error| panic!("derive Original gate translation: {error}"))
    }

    /// Current animation/order type for parity diagnostics.
    pub fn actor_order_type(&self, actor: EntityId) -> Option<crate::order::OrderType> {
        self.get_entity(actor)
            .and_then(|entity| entity.actor_data())
            .map(|actor| resolve_actor_order_type(actor.installed_order))
    }

    /// Mirror an original-game boundary that assigns the order from the selected
    /// sequence element's current order. Callers must invoke this only where
    /// the original game performs that assignment (update, accepted instruction,
    /// or corrected movement retranslation), never as a read-time fallback.
    pub(crate) fn publish_selected_order_as_installed(&mut self, actor: EntityId) {
        let installed_order = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, actor)
            .map(|(_, _, order)| crate::element::InstalledActorOrder {
                order_id: order.order_id,
                order_type: order.order_type,
            });
        tracing::trace!(?actor, ?installed_order, "publishing installed order");
        self.get_entity_mut(actor)
            .expect("installed order publication owner disappeared")
            .actor_data_mut()
            .expect("installed order publication owner lost actor data")
            .installed_order = installed_order;
    }

    /// Original-game animation selection: the live sequence order,
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
    /// table. Resolves the element's priority through its owner when the priority
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
            // synchronously and never reach instruction handling /
            // priority resolution. Mirror that here so commands like
            // `SEND_MESSAGE` don't fall into the actor_branch default.
            if elem.executed_immediately() {
                return crate::sequence::SequencePriority::Normal;
            }
            let owner_entity = elem.owner.and_then(|id| entities.get(id));
            match owner_entity {
                Some(entity) if entity.kind().is_actor() => {
                    let is_unconscious = entity.is_unconscious();
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
    /// still `NotYetSet`. Eager priority resolution runs when a new
    /// sequence element is handed to an actor.
    fn resolve_element_priority(&self, elem: &mut crate::sequence::SequenceElement) {
        if elem.priority == crate::sequence::SequencePriority::NotYetSet {
            let resolver = Self::priority_resolver(&self.world.entities);
            elem.priority = resolver(elem);
        }
    }

    /// Emit the exact owner topology needed to distinguish an attentive
    /// request/translation bug from a postponed-chain publication bug. The
    /// master gate is checked before any entity or sequence state is read.
    pub(super) fn trace_attentive_owner_handoff(
        &self,
        stage: &str,
        owner: EntityId,
        focus: Option<(crate::sequence::SequenceId, usize)>,
        detail: std::fmt::Arguments<'_>,
    ) {
        let Some(config) = attentive_owner_handoff_debug_config() else {
            return;
        };
        if self.control.frame_counter != config.frame
            || self.world.original_creation_order(owner) != config.creation_order
        {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let selected = self.world.entities.current_element_for_actor(owner);
        let deferred = manager
            .deferred_elements_to_go()
            .into_iter()
            .filter(|(seq_id, elem_idx)| {
                manager
                    .get_element(*seq_id, *elem_idx)
                    .is_some_and(|element| element.owner == Some(owner))
            })
            .collect::<Vec<_>>();
        let entity = self.world.entities.get(owner).unwrap_or_else(|| {
            panic!(
                "attentive-owner diagnostic target {} disappeared",
                owner.index()
            )
        });
        let enemy = entity.enemy_ai().unwrap_or_else(|| {
            panic!(
                "attentive-owner diagnostic target {} has no Enemy AI",
                owner.index()
            )
        });
        let actor = entity.actor_data().unwrap_or_else(|| {
            panic!(
                "attentive-owner diagnostic target {} has no actor state",
                owner.index()
            )
        });
        eprintln!(
            "PARITY_ATTENTIVE_OWNER frame={} owner={} owner_co={} stage={} detail={} attentive={} will_be_attentive={} ai_state={:?} ai_substate={:?} action_state={:?} sequence_started={} installed_order={:?} sprite_motion={:?} sprite_action={:?} selected={selected:?} deferred={deferred:?} focus={focus:?}",
            self.control.frame_counter,
            owner.index(),
            config.creation_order,
            stage,
            detail,
            enemy.attentive,
            enemy.will_be_attentive,
            enemy.base.current_state,
            enemy.base.current_substate,
            actor.action_state,
            actor.sequence_element_started,
            actor.installed_order,
            entity.sprite().last_motion_state,
            entity.sprite().last_action,
        );
        for sequence in manager.sequences_iter() {
            for (elem_idx, element) in sequence.elements.iter().enumerate() {
                if element.owner != Some(owner) {
                    continue;
                }
                eprintln!(
                    "PARITY_ATTENTIVE_OWNER frame={} owner={} stage=element seq={} elem={} id={} command={:?} state={:?} priority={:?} postponed={:?} orders={:?}",
                    self.control.frame_counter,
                    owner.index(),
                    sequence.id.0,
                    elem_idx,
                    element.id,
                    element.command,
                    element.state,
                    element.priority,
                    element.postponed,
                    element
                        .orders
                        .iter()
                        .map(|order| (order.order_type, order.order_id, order.done))
                        .collect::<Vec<_>>(),
                );
            }
        }
    }

    /// Register one element through the original game's sequence-element launch
    /// boundary.
    ///
    /// Ordinary owner work remains untouched until the later sequence-
    /// the manager update starts and instructs the element. Priority, transition stamps,
    /// generated orders, and arbitration therefore observe actor state at
    /// instruction time, after every entity has completed its current
    /// frame-update slot. `SequenceManager` separately routes the two original-game
    /// registration-time exceptions: explicit waiting-priority work and the
    /// immediate command whitelist.
    pub(crate) fn launch_element(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        elem: crate::sequence::SequenceElement,
    ) -> crate::sequence::SequenceId {
        let attentive_owner = elem.owner.filter(|_| {
            matches!(
                elem.command,
                crate::element::Command::EnterAttentiveMode
                    | crate::element::Command::LeaveAttentiveMode
                    | crate::element::Command::LeaveAttentiveModeOfficer
            )
        });
        if let Some(owner) = attentive_owner {
            self.trace_attentive_owner_handoff(
                "launch_before",
                owner,
                None,
                format_args!("attentive command registration"),
            );
        }
        let seq_id = self
            .launch_element_inline(sim, assets, &mut Vec::new(), elem)
            .unwrap_or_else(|error| panic!("sequence element launch failed: {error:?}"));
        if let Some(owner) = attentive_owner {
            self.trace_attentive_owner_handoff(
                "launch_after",
                owner,
                Some((seq_id, 0)),
                format_args!("attentive command registered"),
            );
        }
        seq_id
    }

    /// Admit an owned fixture element at an instruction boundary using the
    /// same priority arbitration and transition stamping as scheduled work.
    #[cfg(test)]
    pub(crate) fn launch_element_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        elem: crate::sequence::SequenceElement,
    ) -> crate::sequence::SequenceId {
        let owner = elem.owner.expect("instruction fixture requires an owner");
        let seq_id = self.orders.sequence_manager.insert_element(elem);
        self.orders.sequence_manager.start_sequence_level(seq_id);
        self.dispatch_sequence_phase_action(
            sim,
            assets,
            crate::sequence::SequenceAction::InstructOwner {
                owner,
                sequence_id: seq_id,
                element_index: 0,
            },
        );
        seq_id
    }

    /// Register a direct facing turn without instructing its owner yet.
    /// Cross-owner patrol coordination runs before the member's entity slot;
    /// The original game's sequence tick arbitrates turning only after that
    /// slot, allowing Halt's retained movement exit transition to execute once.
    pub(crate) fn launch_turn_sequence_deferred_no_transitions(
        &mut self,
        owner: EntityId,
        command: crate::element::Command,
        explicit_direction: Option<i16>,
        target_x: f32,
        target_y: f32,
    ) -> crate::sequence::SequenceId {
        let seq_id = self
            .orders
            .sequence_manager
            .register_owned_command(owner, command);
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
                let posture = e.element_data().posture();
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

    /// Apply the PC-on-shoulders movement redirect when the registered
    /// element reaches instruction, matching original-game behavior.
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
        if !rider.is_pc() || rider.element_data().posture() != Posture::OnShoulders {
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

    /// Non-interruptable postpone guard. Runs *before* transition generation
    /// so a command issued on top of a NonInterruptable current element
    /// skips the transition check entirely and either postpones the new
    /// command or rejects a MOVE issued before a freshly-instructed
    /// PASS_DOOR has executed. Returns `true` when the guard consumed the
    /// element (caller should skip generate_transition + arbitrate);
    /// `false` otherwise.
    fn non_interruptable_guard(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        new_seq: crate::sequence::SequenceId,
        new_idx: usize,
    ) -> bool {
        use crate::element::Command;
        use crate::sequence::SequencePriority;

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
            self.element_impossible(sim, assets, active_scripts, new_seq, new_idx);
        } else {
            // `new.Postpone(current)` — current is the blocker, new is
            // the waiter.
            self.engine_postpone(
                sim,
                assets,
                active_scripts,
                cur_seq,
                cur_idx,
                new_seq,
                new_idx,
            );
        }
        true
    }

    /// Launch a 1-frame idle `Command::Wait` owned element at
    /// `SequencePriority::Wait`.  Used to park an actor in idle after
    /// a cross-entity state change (drop corpse, post-tie, post-combat)
    /// so its AI re-enters the default loop instead of continuing the
    /// pre-event command.
    pub(crate) fn actor_wait(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> crate::sequence::SequenceId {
        let mut wait_elem =
            crate::sequence::SequenceElement::new(1, crate::element::Command::Wait, Some(owner));
        wait_elem.priority = crate::sequence::SequencePriority::Wait;
        self.launch_element(sim, assets, wait_elem)
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
    pub(crate) fn actor_freeze_execution(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        use crate::sequence::CascadeFlags;

        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.execution_frozen = true;
        }
        if let Some((cur_seq, cur_idx)) = self.current_sequence_element_for_actor(owner) {
            // Movement-element interruption runs path-request cancellation
            // before delegating to the base state change. Without this, a
            // `MoveWaiting` element's pathfinder request and
            // failed-path retry entry leak past the freeze, and the
            // 100-frame retry queue can fire `element_impossible` /
            // hero-speech on an actor that has been frozen / killed.
            crate::engine::order_arbitration::stop_owner_active_mechanics(
                &mut self.world,
                &mut self.orders,
                owner,
            );
            self.element_interrupted(
                sim,
                assets,
                &mut Vec::new(),
                cur_seq,
                cur_idx,
                CascadeFlags::NEXT_LEVEL,
            );
        }
    }

    /// Apply PC instruction handling before delegating to the base actor.
    /// Returns true when the instruction completes without delegation.
    ///
    /// Arrival speech must terminate here before base Actor's
    /// non-interruptable-current guard can postpone it. Otherwise parallel
    /// movement can win the postponed-chain priority comparison, abandon the
    /// speech, and cascade `Impossible` into later posture recovery work.
    fn pc_instruct_early_completion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
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
        if matches!(
            command,
            Some(crate::element::Command::CrouchUp | crate::element::Command::CrouchDown)
        ) {
            if self
                .get_entity(owner)
                .and_then(Entity::human_data)
                .is_some_and(|human| !human.opponents.is_empty())
            {
                self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                return true;
            }
            // Posture commands stop nonmovement work before transition
            // generation, even when the requested posture is already current.
            // MoveWaiting is deliberately outside the movement command group.
            let current_is_nonmovement = self
                .current_sequence_element_for_actor(owner)
                .and_then(|(sequence, index)| {
                    self.orders.sequence_manager.get_element(sequence, index)
                })
                .is_some_and(|element| !element.command.is_part_of_movement());
            if current_is_nonmovement {
                self.stop_actor_orders(
                    sim,
                    assets,
                    active_scripts,
                    owner,
                    crate::sequence::SequencePriority::Preference,
                );
            }
            return false;
        }
        let expression = match command {
            Some(crate::element::Command::SpeakHeroReachDestination) => {
                crate::engine::melee::HERO_DONE_COMMAND
            }
            Some(crate::element::Command::SpeakVipsAreForRobin) => {
                crate::engine::melee::HERO_PROVOKE_VIP
            }
            _ => return false,
        };
        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
        self.hero_speaking(assets, owner, expression);
        true
    }

    /// Register a prebuilt sequence for the manager hourglass.
    ///
    /// Original-game sequence launching does not call
    /// priority resolution; owned elements resolve it later at their ordered
    /// instruction boundary. Keeping `NotYetSet` here also preserves dynamic
    /// Wait priority when the owner dies or becomes unconscious between
    /// registration and dispatch.
    pub(crate) fn launch_sequence(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        seq: crate::sequence::Sequence,
    ) -> crate::sequence::SequenceId {
        self.launch_sequence_inline(sim, assets, &mut Vec::new(), seq)
            .unwrap_or_else(|error| panic!("sequence launch failed: {error:?}"))
    }

    /// Read the actor's selected instruction, including during callbacks.
    fn current_sequence_element_for_actor(
        &self,
        actor: EntityId,
    ) -> Option<(crate::sequence::SequenceId, usize)> {
        self.world.entities.current_element_for_actor(actor)
    }

    fn select_sequence_element(
        &mut self,
        owner: EntityId,
        selection: Option<(crate::sequence::SequenceId, usize)>,
    ) {
        if let Some(actor) = self
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::actor_data_mut)
        {
            actor.selected_sequence_element = selection
                .map(|(sequence, index)| crate::sequence::SequenceElementRef::new(sequence, index));
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
        let posture = entity.element_data().posture();
        if matches!(
            posture,
            Posture::Flying | Posture::OnLadder | Posture::OnWall
        ) {
            return true;
        }
        self.world
            .entities
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
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
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

            actor.seek_target = None;
            (target, actor.post_seek_sequence.take())
        };
        let Some(post_seek) = post_seek else {
            return false;
        };

        // Clear the completed movement's goal before termination callbacks
        // and the post-seek interaction can observe the handoff.
        if seek_element.is_some() {
            self.get_entity_mut(owner)
                .unwrap_or_else(|| panic!("post-seek owner {owner:?} disappeared"))
                .position_iface_mut()
                .set_map_goal(crate::coordinates::MapPoint::ZERO);
        }

        if let Some(target_id) = target
            && self.get_entity(target_id).is_some_and(|e| e.is_pc())
        {
            self.stop_actor_orders(
                sim,
                assets,
                active_scripts,
                target_id,
                crate::sequence::SequencePriority::Normal,
            );
        }
        if let Some((seq_id, elem_idx)) = seek_element {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
        }

        // Termination callbacks can register the parent's next command level.
        // Register the post-seek sequence after that successor while retaining
        // the live script context throughout the synchronous callbacks.
        self.launch_sequence_inline(sim, assets, active_scripts, post_seek.into_sequence())
            .unwrap_or_else(|error| panic!("post-seek sequence launch failed: {error:?}"));
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
    /// `stop_owner(Preference)` cascade runs, so each `CondolationCard`
    /// delivered while the sequence is being torn down is tagged
    /// `from_halt=true`. The downstream removal-notification handler
    /// checks that tag to suppress the `Think(EVENT_DONE)` /
    /// `Think(EVENT_IMPOSSIBLE)` / `Think(EVENT_COULDNT_REACHPOINT)`
    /// dispatches that should not fire from a halt.
    ///
    /// Movement calls halt here unless `GotoFlags::NO_HALT` is set.
    pub(crate) fn halt_actor(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        if let Some(entity) = self.get_entity_mut(owner)
            && let Some(ai) = entity.ai_controller_mut()
        {
            ai.inside_halt_method = true;
        }
        self.orders.sequence_manager.set_halt_pending(true);

        self.stop_actor_orders(
            sim,
            assets,
            &mut Vec::new(),
            owner,
            crate::sequence::SequencePriority::Preference,
        );
        // Path-request cancellation fires from movement-element
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
        self.orders.sequence_manager.set_halt_pending(false);
        if let Some(entity) = self.get_entity_mut(owner)
            && let Some(ai) = entity.ai_controller_mut()
        {
            ai.inside_halt_method = false;
        }
    }

    /// Launch a one-shot damage sequence; wraps
    /// [`Self::launch_sequence`] so the damage element's priority is
    /// resolved eagerly.
    pub(crate) fn launch_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor: EntityId,
        hp: u16,
        concussion: u16,
    ) -> crate::sequence::SequenceId {
        self.launch_sequence(
            sim,
            assets,
            crate::sequence::Sequence::single_damage(actor, hp, concussion),
        )
    }

    // ─── Test-only helpers ────────────────────────────────────────
    //
    // These are `#[doc(hidden)]` but still `pub` because the downstream
    // `robin_rs` crate ships tests that drive engine state through
    // known-safe back doors (setting mission/quit flags, seeding
    // round-trip state for save-load tests, etc.).  They are not part
    // of the public API and never called from production code.

    /// Test helper: set `mission_won` / `quit_won` / `quit_lost` flags.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub(crate) fn test_set_mission_flags(
        &mut self,
        quit_won: bool,
        quit_lost: bool,
        mission_won: bool,
    ) {
        self.mission_domain.state.quit_won = quit_won;
        self.mission_domain.state.quit_lost = quit_lost;
        self.mission_domain.state.mission_won = mission_won;
    }

    /// Test helper: seed `frame_counter` (save-round-trip tests).
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub(crate) fn test_set_frame_counter(&mut self, frame: u32) {
        self.control.frame_counter = frame;
    }

    /// Test helper: seed miscellaneous scalar engine fields used by
    /// save-round-trip tests.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub(crate) fn test_set_engine_scalars(
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
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub(crate) fn test_set_mission_stat(&mut self, stat: MissionStat) {
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
    pub(crate) fn restore_rng_from_seed(&mut self, seed: u64) {
        self.control.rng.reseed(seed);
    }
}

#[cfg(test)]
mod campaign_lifecycle_tests {
    use std::sync::Arc;

    use super::{EngineInner, LevelAssets};
    use crate::achievement::{AchievementEvaluation, AchievementId};
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

    fn lacklandist_soldier(life_points: i16) -> crate::element::Entity {
        let mut soldier = crate::element::ActorSoldier {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::ActorSoldier;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.life_points = life_points;
        soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
        crate::element::Entity::Soldier(soldier)
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
            None,
            Some(1),
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
            None,
            Some(1),
        );

        let campaign = engine.campaign();
        assert_eq!(campaign.missions[0].status, MissionStatus::Won);
        assert_eq!(campaign.values[CampaignValue::LivingSoldiers], 7);
        assert_eq!(campaign.values[CampaignValue::DeadSoldiers], 11);
        assert_eq!(campaign.values[CampaignValue::Score], 1013);
        assert_eq!(engine.mission_domain.mission_stat.added_score, 1000);
        assert_eq!(
            campaign.missions[0]
                .attempt_history()
                .latest()
                .expect("successful quit records an immutable attempt")
                .stats()
                .added_score,
            Some(1000)
        );
        assert_eq!(engine.mission_domain.mission_stat.new_peasant_count, 0);
        assert_eq!(engine.mission_domain.mission_stat.living_soldier_count, 2);
        assert_eq!(engine.mission_domain.mission_stat.total_soldier_count, 5);
        assert!(engine.mission_achievement_results().is_some());
    }

    #[test]
    fn only_successful_quit_freezes_achievement_results() {
        let sim = crate::sim_rng::test_context();
        let (campaign, assets) = active_historical_mission();
        let mut failed = EngineInner::new_with_campaign(campaign.clone());
        failed
            .mission_domain
            .achievements
            .record_evaluation(AchievementId::Ghost, AchievementEvaluation::Earned)
            .unwrap();
        failed.apply_quit_mission_updates(
            &sim,
            &assets,
            GameCode::LevelFailed,
            DifficultyLevel::Medium,
            None,
            Some(1),
        );
        assert!(failed.mission_achievement_results().is_none());

        let mut succeeded = EngineInner::new_with_campaign(campaign);
        succeeded
            .mission_domain
            .achievements
            .record_evaluation(AchievementId::Ghost, AchievementEvaluation::Earned)
            .unwrap();
        succeeded.apply_quit_mission_updates(
            &sim,
            &assets,
            GameCode::LevelSucceeded,
            DifficultyLevel::Medium,
            None,
            Some(1),
        );
        assert_eq!(
            succeeded
                .mission_achievement_results()
                .unwrap()
                .evaluation(AchievementId::Ghost),
            AchievementEvaluation::Earned
        );
    }

    #[test]
    fn successful_quit_uses_local_exit_counts_without_double_counting_load_total() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut campaign, assets) = active_historical_mission();
        campaign.values[CampaignValue::LivingSoldiers] = 7;
        campaign.values[CampaignValue::DeadSoldiers] = 11;
        campaign.values[CampaignValue::Score] = 13;

        let mut engine = EngineInner::new_with_campaign(campaign);
        engine.mission_domain.mission_stat.living_soldier_count = 2;
        engine.mission_domain.mission_stat.total_soldier_count = 9;
        engine.add_test_entity(lacklandist_soldier(100));
        engine.add_test_entity(lacklandist_soldier(50));
        engine.add_test_entity(lacklandist_soldier(0));

        engine.apply_quit_mission_updates(
            sim,
            &assets,
            GameCode::LevelSucceeded,
            DifficultyLevel::Medium,
            None,
            Some(1),
        );

        let campaign = engine.campaign();
        assert_eq!(campaign.missions[0].status, MissionStatus::Won);
        assert_eq!(campaign.values[CampaignValue::LivingSoldiers], 9);
        assert_eq!(campaign.values[CampaignValue::DeadSoldiers], 12);
        assert_eq!(campaign.values[CampaignValue::Score], 1013);
        assert_eq!(engine.mission_domain.mission_stat.living_soldier_count, 4);
        assert_eq!(engine.mission_domain.mission_stat.total_soldier_count, 9);
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
            completed_at_unix_seconds: None,
            campaign_run_nonce: Some(1),
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

    // Complete authored bindings and live membership after direct fixture setup.
    // Inactive and unconscious actors remain registered until actual removal.
    assets.entities.soldier_entity_ids = engine.world.entities.soldier_ids().collect();
    engine.world.npc_registry_ids = engine.world.entities.npc_ids().collect();
    engine.world.actor_registry_ids = engine
        .world
        .entities
        .actors()
        .map(|(id, _)| id.into())
        .collect();
    engine.world.fighter_registry_ids = engine
        .world
        .entities
        .occupied()
        .filter_map(|(id, entity)| {
            matches!(entity, Entity::Pc(_) | Entity::Soldier(_)).then_some(id)
        })
        .collect();
    let creation_orders = &engine.world.original_creation_order_by_entity;
    for registry in [
        &mut engine.world.actor_registry_ids,
        &mut engine.world.fighter_registry_ids,
    ] {
        registry.sort_by_key(|id| {
            *creation_orders
                .get(id)
                .expect("fixture actor requires a creation order")
        });
    }
    engine.world.npc_registry_ids.sort_by_key(|id| {
        *creation_orders
            .get(id)
            .expect("fixture NPC requires a creation order")
    });

    // Profiles are static level data: production actors carry them whatever
    // their live state, and a fixture actor that starts dead, inactive or
    // unconscious still needs one the moment it revives or is scanned.
    for (_, pc) in engine.world.entities.pcs_mut() {
        if pc
            .element
            .sprite
            .position_iface
            .get_pathfinder_index()
            .is_none()
        {
            pc.element
                .sprite
                .position_iface
                .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        }
        let profile_idx = usize::from(pc.pc.profile_index);
        if profiles.characters.len() <= profile_idx {
            profiles
                .characters
                .resize_with(profile_idx + 1, crate::profiles::CharacterProfile::default);
        }

        // Every PC needs its campaign-description identity: the runtime
        // resolves coma/ammo/portrait state through the campaign character
        // table and treats a missing link as corrupted data.
        let campaign = &mut engine.mission_domain.campaign;
        let description_idx = match pc.pc.campaign_description_index {
            Some(idx) => idx as usize,
            None => {
                let idx = campaign.characters.len();
                pc.pc.campaign_description_index = Some(idx as u32);
                idx
            }
        };
        if campaign.characters.len() <= description_idx {
            campaign
                .characters
                .resize_with(description_idx + 1, crate::campaign::PcDescription::default);
        }
        let description = &mut campaign.characters[description_idx];
        if description.character_profile_idx.is_none() {
            description.character_profile_idx = Some(pc.pc.profile_index);
        }

        if profiles.characters[profile_idx].hth_weapon_id == 0 {
            profiles.characters[profile_idx].hth_weapon_id = 1;
        }
        needs_hth_weapon = true;
    }

    for (soldier_id, soldier) in engine.world.entities.soldiers_mut() {
        if soldier.soldier.cached_camp == crate::element::Camp::Error {
            // Direct unit-test actors have no profile loader to populate
            // their allegiance. Complete the fixture with the canonical
            // enemy camp; production keeps rejecting invalid allegiances.
            soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
        }
        if soldier
            .element
            .sprite
            .position_iface
            .get_pathfinder_index()
            .is_none()
        {
            soldier
                .element
                .sprite
                .position_iface
                .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        }
        if soldier.npc.ai_brain.is_none() {
            soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(
                crate::ai_enemy::EnemyAi::new(soldier_id.0),
            ));
        }
        let enemy_ai =
            soldier.npc.ai_brain.enemy_mut().unwrap_or_else(|| {
                panic!("test soldier {} has a non-enemy AI brain", soldier_id.0)
            });
        // Inactive and unconscious soldiers still appear in camp-soldier
        // snapshots (e.g. as bodies), and those snapshots resolve the HtH
        // weapon profile for every entry with an enemy AI.
        let profile_idx = usize::from(soldier.soldier.soldier_profile_index);
        let behavior_idx = usize::from(enemy_ai.behavior_profile);
        if profiles.soldiers.len() <= profile_idx.max(behavior_idx) {
            profiles
                .soldiers
                .resize_with(profile_idx.max(behavior_idx) + 1, || {
                    crate::profiles::SoldierProfile {
                        intelligence: 50,
                        courage: 50,
                        initiative: 50,
                        ..Default::default()
                    }
                });
        }
        if profiles.soldiers[profile_idx].hth_weapon_id == 0 {
            profiles.soldiers[profile_idx].hth_weapon_id = 1;
        }
        if enemy_ai.hth_weapon_id == 0 {
            enemy_ai.hth_weapon_id = 1;
        }
        needs_hth_weapon = true;
    }

    for (civilian_id, civilian) in engine.world.entities.civilians_mut() {
        if civilian.civilian.cached_camp == crate::element::Camp::Error {
            // Civilians constructed by tests represent the player-allied
            // population unless a test explicitly selected another camp.
            civilian.civilian.cached_camp = crate::element::Camp::Royalists;
        }
        if civilian
            .element
            .sprite
            .position_iface
            .get_pathfinder_index()
            .is_none()
        {
            civilian
                .element
                .sprite
                .position_iface
                .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        }
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

    for (_, entity) in engine.world.entities.occupied() {
        if let Some(ai) = entity.enemy_ai() {
            let index = usize::from(ai.behavior_profile);
            if profiles.soldiers.len() <= index {
                profiles
                    .soldiers
                    .resize_with(index + 1, || crate::profiles::SoldierProfile {
                        intelligence: 50,
                        courage: 50,
                        initiative: 50,
                        ..Default::default()
                    });
            }
        }
    }
    if needs_hth_weapon && profiles.hth_weapons.is_empty() {
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
    }
    assets.profile_manager = std::sync::Arc::new(profiles);
    engine.world.soldier_registry.rebuild_from_order(
        &engine.world.entities,
        assets.entities.soldier_entity_ids.iter().copied(),
    );
    // Fixtures may construct brains or seed alert levels after publishing their actors.
    // Finish that explicit setup before exercising the live alert setter.
    let mut counts = [0u16; 3];
    for (_, soldier) in engine.world.entities.soldiers() {
        let level = soldier
            .npc
            .ai_brain
            .base()
            .expect("completed fixture soldier has AI")
            .current_music_alert_status;
        let count = &mut counts[level as usize];
        *count = count
            .checked_add(1)
            .expect("fixture soldier alert counter overflow");
    }
    [
        engine.ai.global.green_alert_soldiers,
        engine.ai.global.yellow_alert_soldiers,
        engine.ai.global.red_alert_soldiers,
    ] = counts;
    let overall = engine.ai.global.overall_villain_alert();
    engine.ai.global.overall_alert_status = overall;
    engine.ai.global.overall_villain_alert_status = overall;
}
