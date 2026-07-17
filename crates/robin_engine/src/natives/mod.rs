//! Native function registry for the Robin Hood scripting VM.
//!
//! The script VM core registers 265 host functions that scripts
//! call via `NativeCall <index>`. This module provides:
//!
//! 1. A name table mapping each index to its registered name (for logging
//!    and debugging).
//! 2. A `GameHost` struct implementing `interp::HostFunctions` that
//!    dispatches native calls. Functions without a real implementation
//!    are logged and return 0.
//!
//! Real implementations are added incrementally. Currently implemented:
//!   - 0/1/2: InitGlobal, SetGlobal, GetGlobal — cross-script globals
//!   - 3–16: GetActorScript..GetWayIndex — entity handle lookup/reverse lookup
//!   - 30/31/32: Start, Thanx, Then — sequence manager
//!   - 74: ThisActor — current script entity handle
//!   - 75: GetNumberOfActorsInEngine — entity count
//!   - 76–84: IsActorAnimation..IsActorCart — entity type checks
//!   - 85: IsNull — null handle check
//!   - 86: IsActorEqual — handle comparison
//!   - 87–90: IsActorDead, IsActorKO, IsActorTied, IsActorHS
//!   - 91–102: Actor state (posture, direction, location, movement, pain, etc.)
//!   - 95/96: GetActorLocation/SetActorLocation — entity position ↔ location handle
//!   - 97/98: IsInside/IsInsideBuilding — zone/building containment checks
//!   - 103: StopActor — cancels pending sequence elements for the actor
//!   - 104: Sees — synchronous NPC-to-human visibility check
//!   - 105: EnableViewCone — debug view cone toggle
//!   - 108: PrototypeFilterEvent — prototype FilterAIEvent dispatch via nested-VM yield/resume
//!   - 111: God — null handle (sentinel actor)
//!   - 112: Select — select-all / unselect-all PCs
//!   - 113/114: Deactivate/Activate — per-actor SetActive or PC SetPlayable
//!   - 123–141: AI functions (alert, state, attitude, paths, noise, rank, etc.)
//!   - 129: SetAILevel — AI difficulty level / blood alcohol
//!   - 130/131: StareActor/StareLocation — NPC gaze direction
//!   - 133: AssignPost — guard post assignment
//!   - 134/135: LockAI/UnlockAI — NPC/animal AI script-lock flag
//!   - 136: ForceBattleDecision — force combat AI decision
//!   - 137: MakeNoise — broadcast noise stimulus to all NPCs
//!   - 138/139: Freeze/FreezeAll — per-actor or engine-wide tick freeze
//!   - 159: NoWhere, 160: GetDistance, 161: Rand, 162: PrintConsole
//!   - 176–181: SetCompanyNumber, SetAlwaysAttentive, SetInvisible, IsInvisible
//!   - 195/196: GetCustomCampaignValue/SetCustomCampaignValue — second BTreeMap
//!   - 197/198: GetCustomNPCValue/SetCustomNPCValue — BTreeMap keyed by (actor, id)
//!   - 206/207/208: BitwiseAnd/Or/Xor
//!   - 214: DeclareAsCombatTrainer — flag soldier as trainer
//!   - 221/222: IsActorRider, IsUnblipped — entity state checks
//!   - 224/227: AddRepulsivePoint/DeleteRepulsivePoint — NPC avoidance zones
//!   - 240: IsActorActive — entity active state
//!   - 252: MakePCCrouched, 259/260: GetActorActionState/SetActorActionState
//!   - 264: ForbidNPCRemark — suppress NPC remark categories

mod commands;
mod defs;
mod signatures;
#[cfg(test)]
mod tests;

pub use commands::{DeferredCommand, EngineCommand, ObjectiveChange, SoundCommand};
pub use defs::{NativeFn, ORIGINAL_NATIVE_COUNT, RUST_EXTENSION_NATIVE_START, native_name};
pub use signatures::{
    NATIVE_REGISTRY, NATIVE_SIGNATURES, NativeDefinition, NativeNamespace, NativeParamSig,
    NativeSignature, native_definition_by_index, native_definition_by_name,
    native_signature_by_index, native_signature_by_name,
};

// BTreeMap (not BTreeMap) so iteration order is deterministic across
// clients/processes — required for rollback multiplayer determinism.
use std::collections::{BTreeMap, BTreeSet};

use crate::ai::{AiGlobalState, AiState, AlertLevel, EmoticonType, GotoFlags};
use crate::coordinates::MapBBox;
use crate::element::{ActionState, Camp, Command, Entity, EntityId, Posture, TargetFilter};
use crate::element_kinds::ElementKind;
use crate::gate::Door;
use crate::interp::{HostFunctions, NativeStack};
use crate::order::OrderType;
use crate::patch::Patch;
use crate::profiles::Action;
use crate::sequence::{Field, FieldValue, MoveFlags, RecordingSession, Sequence, SequenceElement};
use robin_util::static_arc::StaticArc;

thread_local! {
    static SCRIPT_SIGHT_OBSTACLES: std::cell::RefCell<crate::sight_obstacle::SharedSightObstacles> =
        std::cell::RefCell::new(crate::sight_obstacle::SharedSightObstacles::default());
}

pub(crate) fn set_script_sight_obstacles(
    sight_obstacles: crate::sight_obstacle::SharedSightObstacles,
) {
    SCRIPT_SIGHT_OBSTACLES.with(|cell| {
        *cell.borrow_mut() = sight_obstacles;
    });
}

fn script_sight_obstacles() -> crate::sight_obstacle::SharedSightObstacles {
    SCRIPT_SIGHT_OBSTACLES.with(|cell| cell.borrow().clone())
}

/// Convert a raw script-supplied animation ordinal to an
/// [`OrderType`].  Script-authored data should always be a valid
/// `OrderType` ordinal; if a script passes a value outside the enum
/// range, that's a data bug and we panic with context so it surfaces
/// immediately rather than silently corrupting the sequence element.
fn anim_ordinal_to_order_type(anim: i32, native: &str) -> OrderType {
    OrderType::try_from(anim as u32)
        .unwrap_or_else(|_| panic!("{native}: script passed invalid animation ordinal {anim}"))
}

// ── Script handle encoding ──────────────────────────────────────────
//
// C++ script values are typed `void*` handles. `NULL` is 0; valid
// handles are opaque non-null pointers. The Rust VM stores script values
// as `i32`, so we encode those opaque handles as tagged integers:
//
//   high 4 bits: script type tag
//   low 28 bits: 0-based index in that type's table
//
// This keeps the script-facing index payload 0-based while preserving
// the original null semantics (`0` is still null and can never name the
// first actor).

const SCRIPT_HANDLE_INDEX_MASK: i32 = 0x0fff_ffff;
const SCRIPT_HANDLE_ACTOR_TAG: i32 = 0x1000_0000;
const SCRIPT_HANDLE_DOOR_TAG: i32 = 0x2000_0000;
const SCRIPT_HANDLE_PATCH_TAG: i32 = 0x3000_0000;
const SCRIPT_HANDLE_LOCATION_TAG: i32 = 0x4000_0000;
const SCRIPT_HANDLE_SOUND_SOURCE_TAG: i32 = 0x5000_0000;
const SCRIPT_HANDLE_BUILDING_TAG: i32 = 0x6000_0000;
const SCRIPT_HANDLE_WAY_TAG: i32 = 0x7000_0000;

/// Geometry snapshot for a single script zone, captured at level load.
/// Lets the [`IsInside`] native recompute occupancy via the same
/// polygon point-in-test that runs at the script-call site — needed
/// for teleport-correctness in the same script tick, since
/// `zone_occupants` only updates on explicit Add/CleanFromScriptZone
/// natives or on the next per-frame `tick_zone_occupants` pass.
#[derive(
    Debug, Clone, Default, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct ScriptZonePolygon {
    pub layer: u16,
    pub bounding_box: MapBBox,
    pub points: Vec<crate::coordinates::MapPoint>,
}

/// A host-function implementation that handles the global-variable
/// trio and logs all other calls as unimplemented stubs.
#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
pub struct GameHost {
    /// Cross-script global variables (InitGlobal/SetGlobal/GetGlobal).
    pub globals: BTreeMap<i32, i32>,
    /// Campaign-level custom values (GetCustomCampaignValue/SetCustomCampaignValue).
    pub campaign_values: BTreeMap<i32, i32>,
    /// Per-NPC custom values keyed by (actor_handle, id).
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    pub npc_values: BTreeMap<(i32, i32), i32>,
    /// Incrementing sequence ID for the Then() sequence manager call.
    pub sequence_id: i32,
    /// Set by `ForceCheckVictory` native — tells the engine to run
    /// `CheckVictoryCondition` on the next frame instead of waiting
    /// for the 3-second interval.
    pub force_check: bool,
    /// If true, print each call to stderr as it happens.
    pub verbose: bool,
    /// Deferred commands for the engine to process after script execution.
    pub commands: Vec<EngineCommand>,
    /// Current outline display state (readable by GetOutlineDisplay).
    pub outline_display: bool,

    /// Entity storage, swapped in from EngineInner before script execution
    /// and swapped back out after.  Empty when no script is running.
    pub entities: Vec<Option<Entity>>,
    /// Global AI state, swapped in from EngineInner before script execution.
    pub ai_global: AiGlobalState,
    /// Current mission ambiance, refreshed before each script call so
    /// synchronous visibility natives use the same light-adjusted radius as
    /// the engine AI pass.
    pub ambiance: crate::engine::Ambiance,
    /// Whether the current mission is a forest level. `Sees` needs this for
    /// RHElementActorNPC::ComputeVisibility's Royalist 180-degree branch.
    pub is_forest_level: bool,
    /// FastFindGrid state, swapped in from EngineInner before script execution.
    /// Script-native visibility must use the same `FastFindGrid::is_reachable`
    /// path as engine AI, not a separate obstacle scan.
    pub fast_grid: crate::fast_find_grid::FastFindGrid,
    /// Campaign state, swapped in from EngineInner before script execution.
    pub campaign: Option<crate::campaign::Campaign>,
    /// Profile manager (immutable for the mission's lifetime).  Installed
    /// once at level load by
    /// [`EngineInner::install_script_static_data_into_game_host`] from
    /// `LevelAssets::profile_manager`; previously read off
    /// `Campaign::profiles`.  Lives on the host directly so the campaign
    /// swap-out doesn't make profile data inaccessible to natives.
    pub profile_manager: StaticArc<crate::profiles::ProfileManager>,

    /// Per-mission debriefing stats, swapped in from EngineInner.  Script
    /// natives that mutate campaign RANSOM/SCORE values credit this so
    /// the side effects of campaign value updates match the in-mission
    /// pickup path.
    pub mission_stat: crate::mission_stat::MissionStat,

    /// Handle of the entity whose script is currently running.
    pub script_this: i32,

    /// Number of script locations in the level (swapped in from EngineInner).
    pub script_location_count: usize,
    /// Number of script-point locations; points come first in the
    /// `location_*` arrays, sectors follow. Location payload indices in
    /// `0..script_point_count` are points — used as a point/sector
    /// discriminator guard in natives like `RecordScrollCameraTo`.
    pub script_point_count: usize,
    /// Number of script buildings in the level (swapped in from EngineInner).
    pub script_building_count: usize,
    /// Number of script hiking paths in the level (swapped in from EngineInner).
    pub script_hiking_path_count: usize,
    /// Mission hiking paths, installed with other level-static script data.
    pub hiking_paths: StaticArc<Vec<crate::level_data::RawHikingPath>>,
    /// Number of sound sources (swapped in from EngineInner).
    pub sound_source_count: usize,
    /// Per-sound-source liveness flag (index = script index); `false`
    /// means the slot was `None` at the last refresh (i.e. destroyed via
    /// `DestroySoundSource`).  Deletion nulls the slot but keeps the
    /// array length, so `GetSoundSourceScript` must reject destroyed
    /// indices with the "already been destroyed" error.
    pub sound_source_alive: Vec<bool>,

    /// Position (x, y) for each script location index.
    /// Populated by the engine when loading a level.
    pub location_positions: Vec<(f32, f32)>,
    /// Layer (floor) for each static script location handle, parallel to
    /// `location_positions` (only covers indices `< script_location_count`).
    pub location_layers: Vec<u16>,
    /// Sector number for each static script location handle, parallel to
    /// `location_positions`.
    pub location_sectors: Vec<u16>,
    /// Dynamically computed locations created by GetActorLocation /
    /// ComputeLocationBetween. Payload index = `script_location_count + index`.
    pub computed_locations: Vec<(f32, f32)>,
    /// Parallel to `computed_locations`: the (layer, sector) each computed
    /// location was stamped with.  `GetActorLocation` carries the actor's
    /// current (layer, sector) onto the allocated point, and
    /// `ComputeLocationBetween` inherits pA's (layer, sector); both are
    /// needed so downstream `SetActorLocation(actor, loc)` can refresh
    /// the target's obstacle/sector via `GetProjectionArea`.
    /// `None` means the computed point has no associated sector metadata.
    pub computed_location_layers: Vec<Option<(u16, u16)>>,
    /// Production sector registrations: (type, location_handle, speed).
    /// Populated by RegisterAsProductionSector; consumed by the engine.
    pub production_registrations: Vec<(i32, i32, i32)>,
    /// Production point registrations: (type, location_handle).
    /// Populated by AddProductionPoint; consumed by the engine.
    pub production_points: Vec<(i32, i32)>,

    // ── Sequence recording ──────────────────────────────────────
    /// The sequence currently being recorded via Start/Record*/Then/Thanx.
    /// `None` when not inside a Start..Thanx block.
    pub recording: Option<RecordingSession>,
    /// Sequences completed by Thanx() that the engine should launch.
    /// Drained by the engine after each script call.
    pub completed_sequences: Vec<Sequence>,

    // ── Door / patch / building / sound state for script natives ────
    /// Door state. Script handles are tagged 0-based indices (0 = null).
    /// Populated by the engine when loading a level.
    pub doors: Vec<Door>,
    /// Patch state. Script handles are tagged 0-based indices (0 = null).
    /// Populated by the engine when loading a level.
    pub patches: Vec<Patch>,
    /// Queued sound commands for the engine to process after script execution.
    pub sound_commands: Vec<SoundCommand>,
    /// Set to true when a patch change requires background redraw.
    pub background_invalidated: bool,

    /// Currently executing scroll entity handle (for ThisScroll). 0 = none.
    pub current_scroll: i32,
    /// Scroll status per entity handle.
    pub scroll_status: BTreeMap<i32, i32>,
    /// NPC handle → attached scroll handle (0 or absent = detached).
    pub scroll_attachments: BTreeMap<i32, i32>,
    /// NPCs whose attached scroll changed value since the last titbit
    /// sync — drained by `sync_speak_titbits` to force a SPEAK titbit
    /// remove+add pulse. `AttachScroll` strips the previous SPEAK titbit
    /// and installs a fresh one whenever the attached scroll pointer
    /// differs (matters for any titbit-index-bound consumer).
    pub scroll_attachment_dirty: BTreeSet<i32>,

    /// Entity active state (for FX animations). Key = entity handle.
    pub entity_active: BTreeMap<i32, bool>,

    /// Current animation per entity handle.
    ///
    /// For actors, this is `GetAnimation()` = the `OrderType` of the
    /// front order on the actor's current sequence element. For objects,
    /// this is the object's currently configured animation.
    ///
    /// Populated by [`EngineInner::refresh_game_host_entity_state`]
    /// before every script call so natives like `GetCurrentAction`
    /// can read live animation codes without needing a borrow into
    /// the sequence manager.
    pub current_animations: BTreeMap<i32, OrderType>,

    /// Building occupants. Index = building index. Value = actor handles.
    pub building_occupants: Vec<Vec<i32>>,
    /// Parallel to `building_occupants` (same indexing): whether each
    /// building carries an arrow reserve the player can collect.
    /// Loaded from the GUYS/CAVE tenant chunk at level-load and
    /// propagated into `ai::House::arrow_reserve` by
    /// `EngineInner::initialize_buildings`.
    pub arrow_reserves: Vec<bool>,
    /// Actor handle → building handle (which building they're in).
    pub actor_building: BTreeMap<i32, i32>,
    /// Script zone occupants. Key = location handle, value = actor handles.
    pub zone_occupants: BTreeMap<i32, Vec<i32>>,
    /// Geometry for each script zone (layer + polygon).  Index is
    /// `zone_idx = location_payload_index - script_point_count`.
    /// Populated at level load by
    /// [`crate::engine::EngineInner::install_script_static_data_into_game_host`]
    /// and consulted by the [`IsInside`] native to recompute the
    /// "really inside" geometric test ("works also after teleports").
    pub script_zone_polygons: Vec<ScriptZonePolygon>,
    /// PC auth bits per actor handle (for door special authorisation).
    pub pc_auth_bits: BTreeMap<i32, u16>,

    // ── PC / NPC snapshot state (populated by engine before script) ──
    /// All PC actor handles, in spawn order.
    pub pc_handles: Vec<i32>,
    /// Currently selected PC entity handles.
    pub selected_pc_handles: Vec<i32>,
    /// Robin Hood's entity handle (0 = not yet spawned).
    pub robin_handle: i32,
    /// PC entity handle → character profile index.
    pub pc_profile_map: BTreeMap<i32, crate::profiles::CharacterProfileIdx>,
    /// Whether any civilian NPC is dead (snapshot).
    pub any_civilian_dead: bool,
    /// Whether any enemy (soldier) NPC is dead (snapshot).
    pub any_enemy_dead: bool,
    /// Maximum alert level among living soldiers (0=green, 1=yellow, 2=red).
    pub overall_enemy_alert: i32,
    /// Maximum alert level among living civilians.
    pub overall_civilian_alert: i32,

    /// Deferred game-logic commands for the engine to process after script.
    pub deferred_commands: Vec<DeferredCommand>,

    /// Mission-objective panel changes queued by the Spellforge
    /// `AddObjective` / `CompleteObjective` natives. The host drains
    /// this each frame to update its objectives UI. Empty for missions
    /// that don't use the objectives system (i.e. all vanilla missions
    /// — only Spellforge mods touch it).
    pub pending_objective_changes: Vec<ObjectiveChange>,

    /// Name → entity handle maps populated when a Spellforge-format
    /// `.rhm` is loaded (the extended format prefixes each entity with
    /// a string identifier). Backs the Lua-only `GetActor("name")`,
    /// `GetItem`, `GetLocation`, `GetPatrol`, `GetScroll` natives, and
    /// the reverse `GetActorName(handle)` lookup. Empty for vanilla
    /// `.rhm` files.
    pub lua_actor_names: BTreeMap<String, i32>,
    pub lua_item_names: BTreeMap<String, i32>,
    pub lua_location_names: BTreeMap<String, i32>,
    pub lua_patrol_names: BTreeMap<String, i32>,
    pub lua_scroll_names: BTreeMap<String, i32>,

    /// Building active state. Index = building index.
    pub building_active: Vec<bool>,
    /// Building → gate (door) handles. Index = building index.
    pub building_gates: Vec<Vec<i32>>,

    /// Whether the game is in "men to blazon" conversion UI mode (Sherwood).
    pub men_to_blazon_conversion_mode: bool,

    /// Number of trailing "castle" blazons on the blazon bar that should
    /// flash to "normal" after a tactical-mission overflow.  Set by
    /// `SetBlinkingBlazons`; zero means no blink is active.
    pub blinking_blazons: u32,
    /// EngineInner frame at which the blink latch clears.  `u32::MAX`
    /// when no blink is armed.  The blazon-set refresh decrements over
    /// `BLINK_TIMEOUT` (50) ticks.
    pub blink_expire_frame: u32,

    /// Patch → animation FX entity handle. Index = patch index.
    /// Populated by the engine when loading a level.
    pub patch_animation_entities: Vec<Option<i32>>,

    /// EngineInner frame counter, copied in before each script call so
    /// natives can compute absolute-frame timestamps (e.g. emoticon
    /// expiration).
    pub frame_counter: u32,

    /// Map bounding box in world coordinates. Used by RecordEnterGame /
    /// RecordLeaveGame to compute map-edge spawn and exit points via
    /// `compute_border_point`. Populated by the engine from
    /// `FastFindGrid::map_bbox` before script execution.
    pub map_bbox: MapBBox,

    /// Per-sector type/lift snapshot needed by the record-time
    /// `append_move_to_sequence` to drive the building / ladder-lift
    /// branches without holding a reference to `FastFindGrid`.
    /// Populated from `FastFindGrid::level::sectors` by the engine's
    /// `install_script_static_data_into_game_host`.
    /// Key: sector number (`u16`).
    pub sector_kinds: BTreeMap<u16, SectorKindInfo>,

    /// Nested-script call queued by a native (currently only
    /// `PrototypeFilterEvent`) during a running VM.  Drained by the
    /// interpreter on the same step that queued it: the
    /// [`HostFunctions::take_pending_nested_call`] override returns
    /// this, the interpreter then yields with
    /// [`StopReason::PendingNestedCall`], and
    /// `MissionScript::call_actor_function`'s loop dispatches the
    /// call before resuming the outer VM.  See
    /// `PrototypeFilterEvent` arm of `call()` and the resume loop
    /// in `engine/types.rs` for details.
    pub pending_nested_call: Option<crate::interp::PendingNestedCall>,

    /// Recursion depth of the nested-script-call stack.  Each actor
    /// script normally has its own VMCore (giving an implicit per-core
    /// stack-depth limit); we cap explicitly (see
    /// [`MAX_NESTED_CALL_DEPTH`]) so a script that recursively calls
    /// back into itself via `PrototypeFilterEvent` doesn't blow the
    /// host stack.
    pub nested_call_depth: u8,
}

/// Maximum allowed depth of nested script calls (e.g. one
/// `PrototypeFilterEvent` whose target itself calls
/// `PrototypeFilterEvent`).  Beyond this, the engine returns the
/// base-class default for the call (`1` for `FilterAIEvent`, `0`
/// otherwise) without re-entering a VM.  Picked at 4 to absorb
/// realistic A → B → A → B chains without becoming a debugging
/// hazard if a script accidentally creates a true cycle.
pub const MAX_NESTED_CALL_DEPTH: u8 = 4;

/// Cached subset of sector properties needed by record-time gate
/// expansion in [`GameHost::append_move_to_sequence`].  Same data as
/// the runtime `EngineInner::sector_is_building` / `is_ladder_lift`
/// helpers expose, but keyed off `GameHost`-owned data so the natives
/// layer doesn't need a back-reference to the engine.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct SectorKindInfo {
    /// True for building sectors.
    pub is_building: bool,
    /// True for lift sectors with the LADDER lift type.
    pub is_ladder_lift: bool,
    /// Lift type for lift sectors.
    pub lift_type: Option<crate::sector::LiftType>,
    /// True for door sectors.
    pub is_door: bool,
}

impl GameHost {
    pub fn new() -> Self {
        Self {
            globals: BTreeMap::new(),
            campaign_values: BTreeMap::new(),
            npc_values: BTreeMap::new(),
            sequence_id: 0,
            force_check: false,
            verbose: false,
            entities: Vec::new(),
            ai_global: AiGlobalState::default(),
            ambiance: crate::engine::Ambiance::default(),
            is_forest_level: false,
            fast_grid: crate::fast_find_grid::FastFindGrid::default(),
            campaign: None,
            profile_manager: StaticArc::new(crate::profiles::ProfileManager::new()),
            mission_stat: crate::mission_stat::MissionStat::default(),
            commands: Vec::new(),
            outline_display: false,
            script_this: 0,
            script_location_count: 0,
            script_point_count: 0,
            script_building_count: 0,
            script_hiking_path_count: 0,
            hiking_paths: StaticArc::new(Vec::new()),
            sound_source_count: 0,
            sound_source_alive: Vec::new(),
            location_positions: Vec::new(),
            location_layers: Vec::new(),
            location_sectors: Vec::new(),
            computed_locations: Vec::new(),
            computed_location_layers: Vec::new(),
            production_registrations: Vec::new(),
            production_points: Vec::new(),
            recording: None,
            completed_sequences: Vec::new(),
            doors: Vec::new(),
            patches: Vec::new(),
            sound_commands: Vec::new(),
            background_invalidated: false,
            current_scroll: 0,
            scroll_status: BTreeMap::new(),
            scroll_attachments: BTreeMap::new(),
            scroll_attachment_dirty: BTreeSet::new(),
            entity_active: BTreeMap::new(),
            current_animations: BTreeMap::new(),
            building_occupants: Vec::new(),
            arrow_reserves: Vec::new(),
            actor_building: BTreeMap::new(),
            zone_occupants: BTreeMap::new(),
            script_zone_polygons: Vec::new(),
            pc_auth_bits: BTreeMap::new(),
            pc_handles: Vec::new(),
            selected_pc_handles: Vec::new(),
            robin_handle: 0,
            pc_profile_map: BTreeMap::new(),
            any_civilian_dead: false,
            any_enemy_dead: false,
            overall_enemy_alert: 0,
            overall_civilian_alert: 0,
            deferred_commands: Vec::new(),
            pending_objective_changes: Vec::new(),
            lua_actor_names: BTreeMap::new(),
            lua_item_names: BTreeMap::new(),
            lua_location_names: BTreeMap::new(),
            lua_patrol_names: BTreeMap::new(),
            lua_scroll_names: BTreeMap::new(),
            building_active: Vec::new(),
            building_gates: Vec::new(),
            men_to_blazon_conversion_mode: false,
            blinking_blazons: 0,
            blink_expire_frame: u32::MAX,
            patch_animation_entities: Vec::new(),
            frame_counter: 0,
            map_bbox: MapBBox::new(),
            sector_kinds: BTreeMap::new(),
            pending_nested_call: None,
            nested_call_depth: 0,
        }
    }

    /// Drain all queued engine commands. Called by the engine after script execution.
    pub fn drain_commands(&mut self) -> Vec<EngineCommand> {
        std::mem::take(&mut self.commands)
    }

    /// Mutate a campaign value, with the same side effects as the
    /// engine-side `EngineInner::add_campaign_value`: RANSOM credits
    /// `mission_stat.collected_money` (unconditionally — the underlying
    /// counter is unsigned and wraps on negative deltas) and queues the
    /// `CashWon` jingle only on positive deltas after the first frame;
    /// SCORE credits `mission_stat.added_score` unconditionally.  Both
    /// are swapped in from the engine, so the side effects land on the
    /// same per-mission state the in-mission pickup path writes to.
    pub fn add_campaign_value(&mut self, name: crate::campaign::CampaignValue, amount: i32) {
        let Some(campaign) = self.campaign.as_mut() else {
            return;
        };
        campaign.values[name] += amount;
        // Mission-stat counters are credited unconditionally for
        // RANSOM/SCORE — only the CashWon jingle is gated on
        // `amount > 0 && frame_counter > 0`.
        match name {
            crate::campaign::CampaignValue::Ransom => {
                self.mission_stat.add_collected_money(amount);
                if amount > 0 && self.frame_counter > 0 {
                    self.commands
                        .push(EngineCommand::PlayJingle(crate::sound::Jingle::CashWon));
                }
            }
            crate::campaign::CampaignValue::Score => {
                self.mission_stat.add_score(amount);
            }
            _ => {}
        }
    }

    /// Force a campaign value to a specific number.  RANSOM queues
    /// `CashWon` when the new value exceeds the old (and the universal
    /// frame counter has advanced past 0).
    pub fn set_campaign_value(&mut self, name: crate::campaign::CampaignValue, value: i32) {
        let Some(campaign) = self.campaign.as_mut() else {
            return;
        };
        let old = campaign.values[name];
        campaign.values[name] = value;
        if name == crate::campaign::CampaignValue::Ransom && value > old && self.frame_counter > 0 {
            self.commands
                .push(EngineCommand::PlayJingle(crate::sound::Jingle::CashWon));
        }
    }

    pub fn verbose(mut self) -> Self {
        self.verbose = true;
        self
    }

    /// Arm the blazon-bar blink latch for `BLINK_TIMEOUT` frames.
    /// The last `n` castle blazons flash to the "normal" sprite, then
    /// revert once the blazon-set refresh counts the timeout down to
    /// zero.  `n == 0` disarms the latch.
    pub fn set_blinking_blazons(&mut self, n: u32) {
        const BLINK_TIMEOUT: u32 = 50;
        self.blinking_blazons = n;
        self.blink_expire_frame = if n == 0 {
            u32::MAX
        } else {
            self.frame_counter.saturating_add(BLINK_TIMEOUT)
        };
    }

    /// The blink count that the blazon bar should display this frame.
    /// Returns 0 once the blazon-set refresh timeout would have fired.
    pub fn active_blinking_blazons(&self) -> u32 {
        if self.frame_counter < self.blink_expire_frame {
            self.blinking_blazons
        } else {
            0
        }
    }

    /// Look up an entity by actor handle. Returns None for null or invalid handles.
    fn get_entity(&self, handle: i32) -> Option<&Entity> {
        let idx = Self::actor_handle_index(handle)?;
        self.entities.get(idx)?.as_ref()
    }

    /// Look up an entity mutably by its actor handle.
    fn get_entity_mut(&mut self, handle: i32) -> Option<&mut Entity> {
        let idx = Self::actor_handle_index(handle)?;
        self.entities.get_mut(idx)?.as_mut()
    }

    fn occupied_entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> + '_ {
        self.entities.iter().enumerate().filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|entity| (EntityId::new(idx as u32, entity.entity_id_kind()), entity))
        })
    }

    fn occupied_entities_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut Entity)> + '_ {
        self.entities
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, slot)| {
                slot.as_mut()
                    .map(|entity| (EntityId::new(idx as u32, entity.entity_id_kind()), entity))
            })
    }

    /// Check whether an actor handle refers to a valid entity.
    fn actor_exists(&self, handle: i32) -> bool {
        self.get_entity(handle).is_some()
    }

    /// `ActorExists && IsActor`: the handle resolves to a live actor
    /// entity (PC / Soldier / Civilian, plus any other `is_actor` kind).
    /// Used by Record* natives that gate element creation on the script
    /// passing a real actor.
    fn is_actor_handle(&self, handle: i32) -> bool {
        self.get_entity(handle).is_some_and(|e| e.is_actor())
    }

    /// True iff `handle` resolves to an actor or an FX target.  Used as
    /// the guard for `RecordPlayAnim` / `RecordPlayAnimFreeze`.
    fn is_actor_or_fx_target(&self, handle: i32) -> bool {
        self.get_entity(handle)
            .is_some_and(|e| e.is_actor() || e.is_fx_target())
    }

    /// True iff `handle` resolves to a PC whose profile carries a
    /// `LittleJohnCarry` or `FarmerCarry` action — i.e. the PC can
    /// `TakeCorpse` / `LeaveCorpse`.
    fn is_pc_carrier(&self, handle: i32) -> bool {
        let Some(entity) = self.get_entity(handle) else {
            return false;
        };
        let Some(pc) = entity.pc_data() else {
            return false;
        };
        if self.campaign.is_none() {
            return false;
        }
        self.profile_manager
            .get_character(pc.profile_index)
            .is_some_and(|cp| cp.can_carry())
    }

    fn actor_action_distance(&self, actor: i32, animation: OrderType) -> Option<f32> {
        let Some(entity) = self.get_entity(actor) else {
            tracing::warn!(
                actor,
                ?animation,
                "GameHost::actor_action_distance: actor handle is missing"
            );
            return None;
        };
        match entity.sprite().action_distance(animation) {
            Ok(distance) => Some(distance),
            Err(err) => {
                tracing::warn!(
                    actor,
                    ?animation,
                    error = %err,
                    "GameHost::actor_action_distance: missing sprite action distance"
                );
                None
            }
        }
    }

    /// Drain all completed sequences (built by Start/Record*/Thanx).
    /// Called by the engine after each script execution.
    pub fn take_completed_sequences(&mut self) -> Vec<Sequence> {
        std::mem::take(&mut self.completed_sequences)
    }

    // ── Recording helpers ───────────────────────────────────────

    /// Get the current recording command level, or 1 if not recording.
    fn recording_level(&self) -> u16 {
        self.recording.as_ref().map_or(1, |r| r.command_level)
    }

    /// Convert a script actor handle to the live typed entity ID.
    /// 0 (null handle) or stale handles map to `None`.
    fn actor_id(&self, handle: i32) -> Option<EntityId> {
        let idx = Self::actor_handle_index(handle)?;
        let entity = self.entities.get(idx)?.as_ref()?;
        Some(EntityId::new(idx as u32, entity.entity_id_kind()))
    }

    /// Add a sequence element to the current recording session.
    /// Returns 1 on success, 0 if not currently recording.
    fn record_element(&mut self, element: SequenceElement) -> i32 {
        if let Some(rec) = &mut self.recording {
            rec.add_element(element);
            1
        } else {
            tracing::warn!("Record function called outside Start/Thanx block");
            0
        }
    }

    /// Look up an actor's recording-time origin and refresh its cached
    /// motion target: if the actor already has a cached motion target
    /// from a previous `Record*` move in this session, that cached
    /// point becomes the origin of the new walk and the entry is
    /// overwritten with the new destination.  Otherwise the actor's
    /// live position is the origin and a fresh entry is added.
    ///
    /// Append `elem` and bump the recording session's command level
    /// when `is_first` is false — each emission lands at its own level
    /// (sequential execution rather than concurrent).  The first
    /// emission keeps the caller-provided starting level so the helper
    /// composes cleanly with the surrounding recording flow.
    fn record_seq_step(&mut self, elem: SequenceElement, is_first: bool) {
        if !is_first && let Some(rec) = self.recording.as_mut() {
            rec.advance_level();
        }
        self.record_element(elem);
    }

    /// Look up the sector kind cache populated by the engine at level
    /// load.  Returns `None` only for unknown sector numbers (caller
    /// should treat as "neither building nor ladder").
    fn sector_kind(&self, sector: u16) -> Option<SectorKindInfo> {
        self.sector_kinds.get(&sector).copied()
    }

    fn sector_is_building(&self, sector: u16) -> bool {
        self.sector_kind(sector).is_some_and(|k| k.is_building)
    }

    fn sector_is_ladder_lift(&self, sector: u16) -> bool {
        self.sector_kind(sector).is_some_and(|k| k.is_ladder_lift)
    }

    fn sector_lift_type(
        &self,
        sector: crate::sector::SectorNumber,
    ) -> Option<crate::sector::LiftType> {
        self.sector_kind(u16::from(sector))
            .and_then(|k| k.lift_type)
    }

    fn sector_is_door(&self, sector: u16) -> bool {
        self.sector_kind(sector).is_some_and(|k| k.is_door)
    }

    fn door_index_for_goal_sector(
        &self,
        goal_sector: u16,
        goal: (f32, f32),
    ) -> Option<crate::gate::DoorIndex> {
        self.doors.iter().enumerate().find_map(|(idx, door)| {
            let matches_endpoint = door.sector_out == goal_sector || door.sector_in == goal_sector;
            let matches_click_sector = door.click_polygon_contains(goal.0, goal.1);
            (matches_endpoint || matches_click_sector).then_some(crate::gate::DoorIndex(idx as u32))
        })
    }

    /// Walks the gate path from `(source_sector, source)` to
    /// `(goal_sector, goal)` and appends the corresponding sub-elements
    /// to the active recording session (ASSERT_POSITION leader,
    /// per-gate approach + PASS_DOOR / JUMP / CHANGE_POSITION +
    /// post-pass ASSERT_POSITION, optional trailing MOVE).  Returns
    /// `false` when there is no path between the sectors or when
    /// called outside an active recording session; `true` otherwise
    /// (including the same-sector fast path).
    ///
    /// Side effects (seed `ASSERT_POSITION` against the source sector,
    /// choose move-after-last-door, raise `TO_JUMP` until past the
    /// first jump gate, lockpick short-circuit, `SEEK` building-interior
    /// trailing MOVE) are driven from the data swapped onto `GameHost`
    /// at level load (`doors`, `sector_kinds`, `entities` for the
    /// actor's auth/lockpick lookup).
    ///
    /// `victim` is the SEEK target, passed straight through onto the
    /// trailing MOVE element's `element` field.
    #[allow(clippy::too_many_arguments)]
    fn append_move_to_sequence(
        &mut self,
        actor_handle: i32,
        action: OrderType,
        mut source: (f32, f32),
        mut source_sector: u16,
        mut _source_layer: u16,
        goal: (f32, f32),
        goal_sector: u16,
        goal_layer: u16,
        victim: Option<EntityId>,
        tolerance: f32,
        initial_flags: crate::sequence::MoveFlags,
        speed_factor: f32,
    ) -> bool {
        use crate::element::Command;
        use crate::gate::{find_path_gates, find_path_into_door};
        use crate::position_interface::SectorHandle;
        use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElement, SequenceElementData};

        debug_assert!(
            !initial_flags.contains(MoveFlags::STRAIGHT),
            "AppendMoveToSequence assert: STRAIGHT flag must be clear"
        );

        if self.recording.is_none() {
            return false;
        }

        let owner = self.actor_id(actor_handle);
        let to_pt = |(x, y): (f32, f32)| crate::coordinates::MapPoint { x, y };

        // Original AppendMoveToSequence rewrites the source when the
        // actor is currently straddling a gate (`pTarget->GetDoor()`).
        // Do this before the same-sector fast path and path lookup so
        // recorded/script movement starts from the gate's far side.
        if let Some((door_handle, door_direction)) = self.get_entity(actor_handle).map(|entity| {
            let pi = entity.position_iface();
            (pi.get_door(), pi.get_door_direction())
        }) && let Some((adapted_source, adapted_sector, adapted_layer)) =
            crate::engine::adapt_source_to_current_door(&self.doors, door_handle, door_direction)
        {
            source = (adapted_source.x, adapted_source.y);
            source_sector = adapted_sector;
            _source_layer = adapted_layer;
        }

        // Counter for `record_seq_step`: the very first emission stays
        // at the caller-provided recording level; every subsequent
        // emission bumps the level (sequence-element count increments
        // once per sub-element).
        let mut emit_count: u32 = 0;

        // ── Same-sector fast path ──
        if source_sector == goal_sector {
            let mut elem = SequenceElement::new_movement(0, Command::Move, owner, action);
            if let SequenceElementData::Movement {
                destination,
                element,
                tolerance: tol,
                flags,
                speed_factor: sf,
                layer,
                ..
            } = &mut elem.data
            {
                *destination = to_pt(goal);
                *element = victim;
                *tol = tolerance;
                *flags = initial_flags;
                *sf = speed_factor;
                *layer = goal_layer;
            }
            self.record_seq_step(elem, emit_count == 0);
            return true;
        }

        // ── Cross-sector ASSERT_POSITION leader ──
        let mut leader = SequenceElement::new_movement(0, Command::AssertPosition, owner, action);
        if let SequenceElementData::Movement {
            sector,
            element,
            speed_factor: sf,
            ..
        } = &mut leader.data
        {
            *sector = SectorHandle::new(source_sector);
            *element = owner;
            *sf = speed_factor;
        }
        self.record_seq_step(leader, emit_count == 0);
        emit_count += 1;

        // ── Find the gate path ──
        let auth = self.get_entity(actor_handle).map(|e| e.actor_auth_info());
        let allow_leave_map = initial_flags.contains(MoveFlags::MAP);
        let goal_is_door_sector = self.sector_is_door(goal_sector);

        let path_opt = if goal_is_door_sector {
            self.door_index_for_goal_sector(goal_sector, goal)
                .and_then(|door_idx| {
                    find_path_into_door(
                        &self.doors,
                        source,
                        source_sector,
                        door_idx,
                        auth.as_ref(),
                        allow_leave_map,
                        &|sector| self.sector_lift_type(sector),
                    )
                })
        } else {
            find_path_gates(
                &self.doors,
                source,
                source_sector,
                goal,
                goal_sector,
                auth.as_ref(),
                allow_leave_map,
                &|sector| self.sector_lift_type(sector),
            )
        };

        let Some(gate_steps) = path_opt else {
            // PC speaks HERO_UNABLE_TO_DO_SOMETHING and returns false.
            // The hero-speaking side effect requires engine-side state
            // (sound, hud); queue an EngineCommand so the engine fires
            // the bark on drain.
            if let Some(pc_id) = self
                .get_entity(actor_handle)
                .filter(|e| e.is_pc())
                .and_then(|_| self.actor_id(actor_handle))
            {
                self.commands.push(EngineCommand::HeroSpeak {
                    pc_id,
                    expression: crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                });
            }
            tracing::debug!(
                actor = actor_handle,
                from_sector = source_sector,
                to_sector = goal_sector,
                "AppendMoveToSequence: no gate path"
            );
            return false;
        };

        let move_after_last_door = !goal_is_door_sector;

        // First-jump gate index — controls TO_JUMP flag.
        let first_jump = gate_steps.iter().enumerate().find_map(|(i, step)| {
            self.doors
                .get(usize::from(step.door_index))
                .filter(|d| d.is_jump())
                .map(|_| i)
        });

        // Snapshot per-gate data into a local struct so the per-gate
        // emission loop can run without re-borrowing `self.doors`.
        #[derive(Clone, Copy)]
        struct GateShot {
            door_index: crate::gate::DoorIndex,
            direct: bool,
            entry: crate::coordinates::MapPoint,
            exit: crate::coordinates::MapPoint,
            entry_layer: u16,
            exit_layer: u16,
            new_sector: u16,
            is_jump: bool,
            jump_line_src: Option<crate::jump_line::JumpLineIndex>,
            jump_line_dst: Option<crate::jump_line::JumpLineIndex>,
            is_locked_pc_unlockable: bool,
            entry_action: OrderType,
            door_action: OrderType,
        }

        let gate_shots: Vec<GateShot> = gate_steps
            .iter()
            .filter_map(|step| {
                let door = self.doors.get(usize::from(step.door_index))?;
                let (entry, exit, entry_layer, exit_layer, new_sector) = if step.direct {
                    (
                        door.point_out,
                        door.point_in,
                        door.layer_out,
                        door.layer_in,
                        u16::from(door.sector_in),
                    )
                } else {
                    (
                        door.point_in,
                        door.point_out,
                        door.layer_in,
                        door.layer_out,
                        u16::from(door.sector_out),
                    )
                };
                let is_jump = door.is_jump();
                let (jump_src, jump_dst) = if is_jump {
                    let (s, d) = if step.direct {
                        (door.jump_line_out, door.jump_line_in)
                    } else {
                        (door.jump_line_in, door.jump_line_out)
                    };
                    (
                        s.and_then(crate::jump_line::JumpLineIndex::new),
                        d.and_then(crate::jump_line::JumpLineIndex::new),
                    )
                } else {
                    (None, None)
                };
                let is_locked_pc_unlockable = !is_jump && door.locked_pc && door.unlockable;
                // Original RHsequence.cpp keeps the caller's action on
                // gate approach, WAIT_FREE_LIFT, PASS_DOOR, and
                // post-pass asserts.  Door-specific GetAction1/2 calls
                // exist in original-code but are commented out at
                // execution time.
                let (entry_action, door_action) = (action, action);
                Some(GateShot {
                    door_index: step.door_index,
                    direct: step.direct,
                    entry,
                    exit,
                    entry_layer,
                    exit_layer,
                    new_sector,
                    is_jump,
                    jump_line_src: jump_src,
                    jump_line_dst: jump_dst,
                    is_locked_pc_unlockable,
                    entry_action,
                    door_action,
                })
            })
            .collect();

        let has_lockpick = self
            .get_entity(actor_handle)
            .map(|e| e.actor_auth_info().has_lockpick)
            .unwrap_or(false);

        // Track the "previous" sector so each gate emission knows
        // what it's coming *from*.  After the first gate this is the
        // previous gate's `new_sector`.
        let mut prev_sector = source_sector;

        // Snapshot of the recording size at entry — used to skip the
        // 50-frame wait on the first gate of a building-source
        // emission.
        let first_gate_size = self
            .recording
            .as_ref()
            .map(|r| r.current_size())
            .unwrap_or(0);

        let mut ended_early = false;
        let mut last_new_sector = source_sector;

        let flags_at = |gate_idx: usize| -> MoveFlags {
            match first_jump {
                Some(j) if gate_idx <= j => initial_flags | MoveFlags::TO_JUMP,
                _ => initial_flags,
            }
        };

        for (gate_idx, shot) in gate_shots.iter().enumerate() {
            let gate_flags = flags_at(gate_idx);

            // ── Gate approach ──
            //
            // Original AppendMoveToSequence approaches every gate
            // before splitting into door handling or RHCOMMAND_JUMP.
            let old_is_building = self.sector_is_building(prev_sector);
            let entry_action = shot.entry_action;
            let door_action = shot.door_action;

            if old_is_building {
                let cur_size = self
                    .recording
                    .as_ref()
                    .map(|r| r.current_size())
                    .unwrap_or(0);
                if cur_size != first_gate_size {
                    let mut w = SequenceElement::new_generic(0, Command::WaitTimer, owner);
                    w.set_property(Field::Timer, FieldValue::Integer(50));
                    self.record_seq_step(w, emit_count == 0);
                    emit_count += 1;
                }
                // Random 0..30: source uses `rand() & 15 + rand() & 15`.
                // Script recording runs under the engine's installed
                // simulation RNG, so this consumes the same deterministic
                // stream as runtime gate routing.
                let r: u32 = crate::sim_rng::u32(
                    crate::sim_rng::RngSite::SequenceRecordingBuildingExitWait,
                    0..16,
                ) + crate::sim_rng::u32(
                    crate::sim_rng::RngSite::SequenceRecordingBuildingExitWait,
                    0..16,
                );
                let mut w = SequenceElement::new_generic(0, Command::WaitTimer, owner);
                w.set_property(Field::Timer, FieldValue::Integer(r));
                self.record_seq_step(w, emit_count == 0);
                emit_count += 1;

                // CHANGE_POSITION teleport.
                let dx = shot.exit.x - shot.entry.x;
                let dy = shot.exit.y - shot.entry.y;
                let dir = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
                let mut cp =
                    SequenceElement::new_movement(0, Command::ChangePosition, owner, entry_action);
                if let SequenceElementData::Movement {
                    destination,
                    layer,
                    sector,
                    flags,
                    direction,
                    speed_factor: sf,
                    ..
                } = &mut cp.data
                {
                    *destination = shot.entry;
                    *layer = shot.entry_layer;
                    *sector = SectorHandle::new(prev_sector);
                    *flags = gate_flags;
                    *direction = dir;
                    *sf = speed_factor;
                }
                self.record_seq_step(cp, emit_count == 0);
                emit_count += 1;
            } else {
                // MOVE to gate entry + ASSERT_POSITION.
                let mut m = SequenceElement::new_movement(0, Command::Move, owner, entry_action);
                if let SequenceElementData::Movement {
                    destination,
                    element,
                    tolerance: tol,
                    flags,
                    speed_factor: sf,
                    ..
                } = &mut m.data
                {
                    *destination = shot.entry;
                    *element = victim;
                    *tol = 0.0;
                    *flags = gate_flags;
                    *sf = speed_factor;
                }
                self.record_seq_step(m, emit_count == 0);
                emit_count += 1;

                let mut ap =
                    SequenceElement::new_movement(0, Command::AssertPosition, owner, entry_action);
                if let SequenceElementData::Movement {
                    destination,
                    element,
                    tolerance: tol,
                    speed_factor: sf,
                    ..
                } = &mut ap.data
                {
                    *destination = shot.entry;
                    *element = owner;
                    *tol = 10.0;
                    *sf = speed_factor;
                }
                self.record_seq_step(ap, emit_count == 0);
                emit_count += 1;
            }

            if shot.is_jump {
                // ── Jump gate ──
                let (src, dst) = match (shot.jump_line_src, shot.jump_line_dst) {
                    (Some(s), Some(d)) => (s, d),
                    _ => {
                        tracing::warn!(
                            gate = %shot.door_index,
                            "Jump gate missing jump_line indices; skipping"
                        );
                        prev_sector = shot.new_sector;
                        last_new_sector = shot.new_sector;
                        continue;
                    }
                };
                let mut jump_elem = SequenceElement::new_generic(0, Command::Jump, owner);
                jump_elem.set_property(Field::JumplineSource, FieldValue::LineId(src));
                jump_elem.set_property(Field::JumplineDestination, FieldValue::LineId(dst));
                self.record_seq_step(jump_elem, emit_count == 0);
                emit_count += 1;
                prev_sector = shot.new_sector;
                last_new_sector = shot.new_sector;
                continue;
            }

            // ── Lockpick branch ──
            if shot.is_locked_pc_unlockable && has_lockpick {
                let cam_pt = if shot.direct { shot.exit } else { shot.entry };
                let mut turn = SequenceElement::new_generic(0, Command::Turn, owner);
                turn.set_property(
                    Field::CameraPoint,
                    FieldValue::GeoPoint2D {
                        x: cam_pt.x,
                        y: cam_pt.y,
                    },
                );
                self.record_seq_step(turn, emit_count == 0);
                emit_count += 1;

                let mut unlock = SequenceElement::new_generic(0, Command::UnlockDoor, owner);
                unlock.set_property(Field::Door, FieldValue::DoorId(shot.door_index));
                self.record_seq_step(unlock, emit_count == 0);
                emit_count += 1;

                ended_early = true;
                last_new_sector = shot.new_sector;
                break;
            }

            // ── Ladder-lift wait ──
            if self.sector_is_ladder_lift(shot.new_sector) {
                let mut wait =
                    SequenceElement::new_movement(0, Command::WaitFreeLift, owner, door_action);
                if let SequenceElementData::Movement {
                    sector,
                    gate_id,
                    speed_factor: sf,
                    ..
                } = &mut wait.data
                {
                    *sector = SectorHandle::new(shot.new_sector);
                    *gate_id = Some(shot.door_index);
                    *sf = speed_factor;
                }
                self.record_seq_step(wait, emit_count == 0);
                emit_count += 1;
            }

            // ── PASS_DOOR ──
            let mut pass = SequenceElement::new_movement(0, Command::PassDoor, owner, door_action);
            if let SequenceElementData::Movement {
                destination,
                layer,
                gate_id,
                flags,
                speed_factor: sf,
                ..
            } = &mut pass.data
            {
                *destination = shot.exit;
                *layer = shot.exit_layer;
                *gate_id = Some(shot.door_index);
                // Original PASS_DOOR constructor uses default flags
                // and only attaches the gate via SetGate.
                *flags = MoveFlags::empty();
                *sf = speed_factor;
            }
            self.record_seq_step(pass, emit_count == 0);
            emit_count += 1;

            // ── ASSERT post-pass ──
            let mut ap =
                SequenceElement::new_movement(0, Command::AssertPosition, owner, door_action);
            if let SequenceElementData::Movement {
                destination,
                element,
                tolerance: tol,
                speed_factor: sf,
                ..
            } = &mut ap.data
            {
                *destination = shot.exit;
                *element = owner;
                *tol = 10.0;
                *sf = speed_factor;
            }
            self.record_seq_step(ap, emit_count == 0);
            emit_count += 1;

            prev_sector = shot.new_sector;
            last_new_sector = shot.new_sector;
        }

        // ── Trailing emission ──
        if !ended_early {
            let last_into_building = self.sector_is_building(last_new_sector);

            // Trailing MOVE to the goal unless we landed inside a
            // building or `move_after_last_door=false`.
            if move_after_last_door && !last_into_building {
                let mut m = SequenceElement::new_movement(0, Command::Move, owner, action);
                if let SequenceElementData::Movement {
                    destination,
                    element,
                    tolerance: tol,
                    flags,
                    speed_factor: sf,
                    layer,
                    ..
                } = &mut m.data
                {
                    *destination = to_pt(goal);
                    *element = victim;
                    *tol = tolerance;
                    *flags = initial_flags;
                    *sf = speed_factor;
                    *layer = goal_layer;
                }
                self.record_seq_step(m, emit_count == 0);
                emit_count += 1;
            }

            // SEEK + last sector is building → trailing MOVE back to
            // the last gate's `point_in` so the seeker doesn't get
            // stuck at the interior teleport spot.
            if last_into_building
                && initial_flags.contains(MoveFlags::SEEK)
                && let Some(last_shot) = gate_shots.last()
            {
                let point_in = self
                    .doors
                    .get(usize::from(last_shot.door_index))
                    .map(|d| d.point_in)
                    .unwrap_or(last_shot.exit);
                let mut m = SequenceElement::new_movement(0, Command::Move, owner, action);
                if let SequenceElementData::Movement {
                    destination,
                    element,
                    tolerance: tol,
                    flags,
                    speed_factor: sf,
                    layer,
                    ..
                } = &mut m.data
                {
                    *destination = point_in;
                    *element = victim;
                    *tol = tolerance;
                    *flags = initial_flags;
                    *sf = speed_factor;
                    *layer = goal_layer;
                }
                self.record_seq_step(m, emit_count == 0);
                emit_count += 1;
            }
        }

        let _ = emit_count;
        true
    }

    /// Returns the resolved origin as `(x, y, layer, sector)`.  Returns
    /// `None` only when there is no active recording session and the
    /// actor handle is invalid.
    fn update_motion_start_position(
        &mut self,
        actor_handle: i32,
        new_dest: (f32, f32),
        new_dest_layer_sector: Option<(u16, u16)>,
    ) -> Option<(f32, f32, u16, u16)> {
        use crate::sequence::RecordingMotionTarget;
        // Fall back to live actor position if no cached entry.
        let live_origin: Option<(f32, f32, u16, u16)> = self.get_entity(actor_handle).map(|e| {
            let p = e.element_data().position_map();
            let layer = e.element_data().layer();
            let sector = e.element_data().sector().map(u16::from).unwrap_or(0);
            (p.x, p.y, layer, sector)
        });

        let (dest_layer, dest_sector) = new_dest_layer_sector.unwrap_or((0, 0));
        let new_target = RecordingMotionTarget {
            x: new_dest.0,
            y: new_dest.1,
            layer: dest_layer,
            sector: dest_sector,
        };

        let rec = self.recording.as_mut()?;
        match rec.moving_actors.get(&actor_handle).copied() {
            Some(prev) => {
                rec.moving_actors.insert(actor_handle, new_target);
                Some((prev.x, prev.y, prev.layer, prev.sector))
            }
            None => {
                rec.moving_actors.insert(actor_handle, new_target);
                live_origin
            }
        }
    }

    /// Convert a script movement style int to an OrderType.
    /// Style codes: WALKING = 0, RUNNING = 1, WALKING_NONINTERRUPTABLE = 2,
    /// RUNNING_NONINTERRUPTABLE = 3.  The Move-family natives
    /// (`RecordMove`, `RecordMoveNear`, `RecordMoveIntoBuilding`,
    /// `RecordTakeCorpse`, `RecordEnterGame`, `RecordLeaveGame`) map
    /// `{WALKING, WALKING_NONINTERRUPTABLE}` → WalkingUpright and the rest
    /// → RunningUpright.  Note: `RecordSeekActor` uses the *reverse*
    /// convention (style==1 → WALKING) via [`Self::seek_style`].
    fn movement_style(style: i32) -> OrderType {
        if style == 0 || style == 2 {
            OrderType::WalkingUpright
        } else {
            OrderType::RunningUpright
        }
    }

    /// Build a SendMessage sequence element carrying the given
    /// (message, arg1, arg2) triple.  Used by the
    /// `RecordSeekActorMessage[WithArguments]` natives to append the
    /// post-seek notification after the seek element.
    fn build_send_message_element(
        &self,
        level: u16,
        target_actor: i32,
        msg_id: i32,
        arg1: i32,
        arg2: i32,
    ) -> SequenceElement {
        let mut elem =
            SequenceElement::new_generic(level, Command::SendMessage, self.actor_id(target_actor));
        elem.set_property(Field::Message, FieldValue::Integer(msg_id as u32));
        elem.set_property(Field::MessageArgument, FieldValue::Integer(arg1 as u32));
        elem.set_property(
            Field::MessageExtendedArgument,
            FieldValue::Integer(arg2 as u32),
        );
        elem
    }

    /// Validate a script movement style argument. `RecordEnterGame`,
    /// `RecordLeaveGame`, and friends reject anything that isn't WALKING
    /// (1) or RUNNING (2) with an error; we warn-log and let the caller
    /// short-circuit so scripts that pass a bogus style don't silently
    /// default to RUNNING.
    fn validate_style(style: i32, native_name: &str) -> bool {
        if style == 1 || style == 2 {
            true
        } else {
            tracing::warn!(
                "{native_name}: illegal movement style {style} (expected 1=WALKING or 2=RUNNING)"
            );
            false
        }
    }

    /// Convert a script seek style int to an OrderType.
    /// RecordSeekActor: style==1 → WALKING, else → RUNNING.
    /// RecordSeekActorMessage: style==0 → WALKING, else → RUNNING (note: reversed!)
    fn seek_style(style: i32) -> OrderType {
        if style == 1 {
            OrderType::WalkingUpright
        } else {
            OrderType::RunningUpright
        }
    }

    /// Compute the map-edge "border" point reached by walking from
    /// `inside` in the opposite of `direction`, and an "outside" point
    /// a small margin further so the actor's sprite box sits entirely
    /// off the map.
    ///
    /// Used by RecordEnterGame / RecordLeaveGame to pick spawn / exit
    /// points at the map border based on the actor's facing direction.
    ///
    /// `inside` is assumed to be strictly inside `self.map_bbox`;
    /// panics if no edge is reached (shouldn't happen for a valid
    /// inside point and non-zero direction vector).
    fn compute_border_point(&self, inside: (f32, f32), direction: i16) -> ((f32, f32), (f32, f32)) {
        compute_border_point_bbox(self.map_bbox, inside, direction)
    }
}

/// Compute the map-edge "border" point reached by walking from
/// `inside` in the opposite of `direction`, and an "outside" point
/// a small margin further so the actor's sprite box sits entirely
/// off the map.  Standalone version of [`GameHost::compute_border_point`]
/// so level-load code (which does not hold a `GameHost`) can share the
/// same computation.
pub(crate) fn compute_border_point_bbox(
    map_bbox: MapBBox,
    inside: (f32, f32),
    direction: i16,
) -> ((f32, f32), (f32, f32)) {
    assert!(
        map_bbox.is_somewhere(),
        "compute_border_point: map_bbox not populated"
    );

    // The half-line starts at `inside` and goes in the `-direction`
    // direction.  (The actor will walk into the map in `+direction`.)
    let (dx, dy) = crate::element::direction_vector_16(direction);
    let hx = -dx;
    let hy = -dy;

    let x_min = map_bbox.x_min();
    let x_max = map_bbox.x_max();
    let y_min = map_bbox.y_min();
    let y_max = map_bbox.y_max();

    let (ix, iy) = inside;
    let mut best: Option<(f32, f32, f32)> = None;

    let mut try_edge = |x: f32, y: f32| {
        let dxp = x - ix;
        let dyp = y - iy;
        let sq = dxp * dxp + dyp * dyp;
        if best.is_none_or(|(bs, _, _)| sq < bs) {
            best = Some((sq, x, y));
        }
    };

    // Intersect half-line with each of the four map edges.
    // `t > 0` keeps only the forward direction.
    let eps = 1.0e-6_f32;
    if hy.abs() > eps {
        // Top edge (y = y_min).
        let t = (y_min - iy) / hy;
        if t > 0.0 {
            let x = ix + t * hx;
            if (x_min..=x_max).contains(&x) {
                try_edge(x, y_min);
            }
        }
        // Bottom edge (y = y_max).
        let t = (y_max - iy) / hy;
        if t > 0.0 {
            let x = ix + t * hx;
            if (x_min..=x_max).contains(&x) {
                try_edge(x, y_max);
            }
        }
    }
    if hx.abs() > eps {
        // Left edge (x = x_min).
        let t = (x_min - ix) / hx;
        if t > 0.0 {
            let y = iy + t * hy;
            if (y_min..=y_max).contains(&y) {
                try_edge(x_min, y);
            }
        }
        // Right edge (x = x_max).
        let t = (x_max - ix) / hx;
        if t > 0.0 {
            let y = iy + t * hy;
            if (y_min..=y_max).contains(&y) {
                try_edge(x_max, y);
            }
        }
    }

    let (_, bx, by) = best.expect("compute_border_point: no map-edge intersection");

    // Compute the outside point by stepping along the half-line in
    // 10-unit increments along the direction vector until a rough
    // sprite bounding box centred on the outside point no longer
    // touches the map box.  The sprite box `(-50, -70, 50, 20)` is a
    // conservative estimate of actor silhouette size.
    let shift_x = hx * 10.0;
    let shift_y = hy * 10.0;
    let sprite_x_min = -50.0_f32;
    let sprite_y_min = -70.0_f32;
    let sprite_x_max = 50.0_f32;
    let sprite_y_max = 20.0_f32;

    let mut ox = bx;
    let mut oy = by;
    // Cap iterations so we don't spin forever if the direction
    // vector is tangential to an edge (shouldn't happen in
    // practice with unit vectors on cardinal / diagonal sectors).
    for _ in 0..256 {
        ox += shift_x;
        oy += shift_y;
        let bxmin = ox + sprite_x_min;
        let bxmax = ox + sprite_x_max;
        let bymin = oy + sprite_y_min;
        let bymax = oy + sprite_y_max;
        let intersects = bxmax >= x_min && bxmin <= x_max && bymax >= y_min && bymin <= y_max;
        if !intersects {
            break;
        }
    }

    ((bx, by), (ox, oy))
}

impl GameHost {
    /// Convert a script seek style int for the *Message variants.
    /// style==0 → WALKING, else → RUNNING (reversed from RecordSeekActor).
    fn seek_message_style(style: i32) -> OrderType {
        if style == 0 {
            OrderType::WalkingUpright
        } else {
            OrderType::RunningUpright
        }
    }

    // ═══════════════════════════════════════════════════════════════
    //  Blazon / bonus implementations
    // ═══════════════════════════════════════════════════════════════

    /// WinBlazon: deactivate blazon entity, add its quantity to campaign blazon value.
    fn win_blazon(&mut self, handle: i32) {
        // Get quantity before deactivating
        let quantity = match self.get_entity(handle) {
            Some(Entity::Bonus(e)) => e.object.quantity as i32,
            Some(_) => {
                tracing::warn!("Script Error: WinBlazon handle {handle} is not a blazon");
                return;
            }
            None => {
                tracing::warn!("Script Error: WinBlazon with null handle");
                return;
            }
        };

        // Check already won (inactive)
        if let Some(entity) = self.get_entity(handle)
            && !entity.element_data().active
        {
            tracing::warn!("Script Error: WinBlazon blazon already won");
            return;
        }

        // Deactivate the blazon
        if let Some(entity) = self.get_entity_mut(handle) {
            entity.element_data_mut().active = false;
        }

        // Add value to campaign + run the post-win accounting (the
        // tactical-overflow branch clamps the campaign value and arms
        // the blink latch; handle it here so the blazon bar picks it
        // up on the next frame).
        let mut tactical_overflow: Option<u32> = None;
        if let Some(campaign) = self.campaign.as_mut() {
            campaign.add_value(crate::campaign::CampaignValue::Blazon, quantity);

            if let Some(idx) = campaign.current_mission_idx {
                let mission_type = campaign.missions[idx]
                    .profile(&self.profile_manager)
                    .mission_type;
                let current_blazons = campaign.get_value(crate::campaign::CampaignValue::Blazon);
                match mission_type {
                    crate::profiles::MissionType::Attack => {
                        // ATTACK missions win as soon as the collected
                        // total meets `number_of_blazons_to_win`.
                        let to_win = campaign.missions[idx]
                            .profile(&self.profile_manager)
                            .number_of_blazons_to_win;
                        if to_win as i32 <= current_blazons {
                            self.commands.push(EngineCommand::Win { show_window: true });
                        }
                    }
                    crate::profiles::MissionType::Tactical => {
                        // The blazon mission caps what the player may
                        // *bring* into it, so tactical overflow past
                        // `win - to_be_collected` is clamped and the
                        // excess is flashed on the bar.
                        if let Some(bm_idx) = campaign.blazon_mission_idx {
                            let bp = campaign.missions[bm_idx].profile(&self.profile_manager);
                            let collectable = bp
                                .number_of_blazons_to_win
                                .saturating_sub(bp.number_of_blazons_to_be_collected)
                                as i32;
                            if current_blazons > collectable {
                                let exceeding = (current_blazons - collectable) as u32;
                                campaign
                                    .set_value(crate::campaign::CampaignValue::Blazon, collectable);
                                tactical_overflow = Some(exceeding);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(n) = tactical_overflow {
            self.set_blinking_blazons(n);
        }

        // `UpdateInformationBars` / `UpdateBlazons` only fire in
        // campaign mode; in single-mission mode the update is skipped.
        if self.campaign.is_some() {
            self.commands.push(EngineCommand::UpdateInformationBars);
        }
    }

    /// LoseBlazon: reactivate blazon entity, subtract its quantity from campaign.
    fn lose_blazon(&mut self, handle: i32) {
        match self.get_entity_mut(handle) {
            Some(Entity::Bonus(e)) => {
                if e.element.active {
                    // Blazon was not won — nothing to do
                    return;
                }
                let quantity = e.object.quantity as i32;
                e.element.active = true;

                let had_campaign = self.campaign.is_some();
                if let Some(campaign) = self.campaign.as_mut() {
                    campaign.subtract_value(crate::campaign::CampaignValue::Blazon, quantity);
                }
                if had_campaign {
                    // Refresh the information bars in campaign mode.
                    self.commands.push(EngineCommand::UpdateInformationBars);
                }
            }
            Some(_) => {
                tracing::warn!("Script Error: LoseBlazon handle {handle} is not a blazon");
            }
            None => {
                tracing::warn!("Script Error: LoseBlazon with null handle");
            }
        }
    }

    /// IsBlazonWon: check if a blazon entity is inactive (collected).
    fn is_blazon_won(&self, handle: i32) -> i32 {
        match self.get_entity(handle) {
            Some(Entity::Bonus(e)) => {
                if !e.element.active {
                    1
                } else {
                    0
                }
            }
            Some(_) => {
                tracing::warn!("Script Error: IsBlazonWon handle {handle} is not a blazon");
                0
            }
            None => {
                tracing::warn!("Script Error: IsBlazonWon with null handle");
                0
            }
        }
    }

    /// IsBonusItemPickedUp: check if a bonus object has been taken.
    fn is_bonus_item_picked_up(&mut self, handle: i32) -> i32 {
        match self.get_entity(handle) {
            Some(entity) if entity.is_object() => {
                // Only `Entity::Bonus` qualifies as a bonus item.
                // Scrolls, projectiles, and nets fail the check and
                // short-circuit to the warn-and-return-false path.
                if entity.kind().is_bonus() {
                    match entity {
                        Entity::Bonus(e) => i32::from(e.object.taken),
                        _ => unreachable!(),
                    }
                } else {
                    tracing::warn!("Script error: IsBonusItemPickedUp item is not a bonus item");
                    0
                }
            }
            Some(_) => {
                tracing::debug!(
                    "Script Error: IsBonusItemPickedUp handle {handle} is not an object"
                );
                0
            }
            None => {
                tracing::debug!("Script Error: IsBonusItemPickedUp invalid handle {handle}");
                0
            }
        }
    }

    /// ConfiscateMoney: transfer all money from an NPC to the campaign ransom pool.
    fn confiscate_money(&mut self, handle: i32) {
        let money = match self.get_entity_mut(handle) {
            Some(Entity::Soldier(e)) => {
                let m = e.npc.money as i32;
                e.npc.money = 0;
                m
            }
            Some(Entity::Civilian(e)) => {
                let m = e.npc.money as i32;
                e.npc.money = 0;
                m
            }
            Some(Entity::Pc(_)) => return, // PCs are skipped
            Some(_) => {
                tracing::warn!("Script Error: ConfiscateMoney on non-human {handle}");
                return;
            }
            None => {
                tracing::warn!("Script Error: ConfiscateMoney invalid actor {handle}");
                return;
            }
        };

        if self.campaign.is_some() {
            self.add_campaign_value(crate::campaign::CampaignValue::Ransom, money);
        }
    }

    // ═══════════════════════════════════════════════════════════════
    //  Beam-me implementations
    // ═══════════════════════════════════════════════════════════════

    /// MoveBeamMe: relocate the PC at beam-me index `idx` to location `loc`.
    fn move_beam_me(&mut self, idx: i32, loc: i32) {
        // Find PC with matching beam_me_index
        let target_handle = self
            .pc_handles
            .iter()
            .find(|&h| {
                self.get_entity(*h)
                    .and_then(|e| e.pc_data())
                    .is_some_and(|pc| pc.beam_me_index == idx as i16)
            })
            .copied();

        let Some(handle) = target_handle else {
            // Reaching this branch is a script authoring bug.
            tracing::error!("Script Error: MoveBeamMe no PC with beam_me_index {idx}");
            return;
        };

        let Some((x, y)) = self.resolve_location_pos(loc) else {
            tracing::warn!("MoveBeamMe: cannot resolve location handle {loc}");
            return;
        };
        // Layer/sector are read off the target point and written onto
        // the PC alongside the position.  Without this the PC's
        // layer/sector stay stale, so collision/LOS/display-order
        // queries still use the old sector.
        let dest_layer_sector = self.resolve_location_layer_sector(loc);
        if let Some(entity) = self.get_entity_mut(handle) {
            let ed = entity.element_data_mut();
            ed.set_position_map(crate::coordinates::MapPoint { x, y });
            if let Some((layer, sector_num)) = dest_layer_sector {
                ed.set_layer(layer);
                ed.set_sector(crate::position_interface::SectorHandle::new(sector_num));
            }
            ed.update_grid_cell();
        }
    }

    /// GetActorForBeamMe: find the PC entity handle at beam-me index `idx`.
    fn get_actor_for_beam_me(&self, idx: i32) -> i32 {
        self.pc_handles
            .iter()
            .find(|&h| {
                self.get_entity(*h)
                    .and_then(|e| e.pc_data())
                    .is_some_and(|pc| pc.beam_me_index == idx as i16)
            })
            .copied()
            .unwrap_or(0)
    }

    // ═══════════════════════════════════════════════════════════════
    //  Relic lookup
    // ═══════════════════════════════════════════════════════════════

    /// GetRelic: find a bonus object entity by relic type index.
    fn get_relic(&self, relic_id: i32) -> i32 {
        use crate::element::ObjectType;

        let object_type = match relic_id {
            0 => ObjectType::BonusAmpulla,
            1 => ObjectType::BonusCoronationSpoon,
            2 => ObjectType::BonusRichardsCrown,
            3 => ObjectType::BonusRoyalSeal,
            4 => ObjectType::BonusRoyalSceptre,
            5 => ObjectType::BonusDomesdayBook,
            6 => ObjectType::BonusSwordOfTheState,
            _ => return 0,
        };

        if let Some((entity_id, _)) = self.occupied_entities().find(|(_, entity)| {
            matches!(
                entity,
                Entity::Bonus(bonus)
                    if bonus.element.active && bonus.object.object_type == object_type
            )
        }) {
            return Self::actor_handle(entity_id);
        }
        0 // not found
    }

    // ═══════════════════════════════════════════════════════════════
    //  Target transform
    // ═══════════════════════════════════════════════════════════════

    /// TransformHandleTargetToTakeTarget: swap HANDLE flag to TAKE flag on a target.
    fn transform_handle_target_to_take_target(&mut self, handle: i32) {
        match self.get_entity_mut(handle) {
            Some(Entity::Target(e)) => {
                let filter = e.target.action_filter;
                if filter.contains(TargetFilter::TAKE) {
                    tracing::warn!(
                        "Script Error: TransformHandleTargetToTakeTarget already takable"
                    );
                    return;
                }
                if !filter.contains(TargetFilter::HANDLE) {
                    tracing::warn!("Script Error: TransformHandleTargetToTakeTarget not handlable");
                    return;
                }
                // Swap: add TAKE, remove HANDLE
                e.target.action_filter = (filter | TargetFilter::TAKE) & !TargetFilter::HANDLE;
            }
            Some(_) => {
                tracing::warn!(
                    "Script Error: TransformHandleTargetToTakeTarget handle {handle} is not a target"
                );
            }
            None => {
                tracing::warn!(
                    "Script Error: TransformHandleTargetToTakeTarget invalid handle {handle}"
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════
    //  Persistent property dispatch
    // ═══════════════════════════════════════════════════════════════
    //
    // Property IDs:
    //   0  = arrows (human)       1  = money (NPC)
    //   2  = life points (human)  3  = concussion (human)
    //   4  = purses (PC)          5  = stones (PC)
    //   6  = apples (PC)          7  = ales (PC)
    //   8  = legs/rations (PC)    9  = plants (PC)
    //   10 = nets (PC)            11 = wasp nests (PC)
    //   12 = name (PC, set only)

    fn get_persistent_property(&self, actor: i32, prop: i32) -> i32 {
        use crate::profiles::Action;

        let entity = match self.get_entity(actor) {
            Some(e) => e,
            None => {
                tracing::warn!("Script Error: GetPersistentProperty invalid actor {actor}");
                return -1;
            }
        };

        match prop {
            // 0: arrows — first checks bow presence (returns 0 if no bow),
            // then per-class ammo lookup:
            //   - PC: reads PcStatus's ammo counter for Bow.
            //   - NPC (Soldier/Civilian): returns `npc.number_of_arrows`.
            // Bow presence:
            //   - PC: bow-action presence is reflected in PcStatus's ammo
            //     counter mapping (no counter means no bow).
            //   - Soldier: `InitializeWeapons(profile.shooting - 1)` only
            //     constructs `mpBow` when the profile has a valid shooting
            //     weapon.
            //   - Civilian: never has a bow.
            0 => {
                if !entity.is_human() {
                    tracing::warn!("Script Error: GetPersistentProperty 'arrows' on non-human");
                    return -1;
                }
                match entity {
                    Entity::Pc(e) => {
                        // Bow presence on a PC is determined by the
                        // profile's action list — same check
                        // `set_persistent_property` uses on the write
                        // path so the read/write stay symmetric.
                        if !self
                            .profile_manager
                            .get_character(e.pc.profile_index)
                            .is_some_and(|p| p.has_action(Action::Bow))
                        {
                            return 0;
                        }
                        // Campaign status is authoritative when present.  A
                        // live PC still owns actor-local ammo so this native
                        // works outside campaign mode, as the original
                        // RHElementActorPC::GetAmmoAmount does.
                        self.campaign
                            .as_ref()
                            .and_then(|c| c.get_character_by_profile(e.pc.profile_index))
                            .and_then(|idx| self.campaign.as_ref()?.characters.get(idx))
                            .map(|desc| desc.status.get_ammo(Action::Bow))
                            .or_else(|| e.pc.ammo.get(Action::Bow))
                            .expect("Bow has a live PC ammo counter") as i32
                    }
                    Entity::Soldier(e) => {
                        if self.soldier_has_bow_profile(e.soldier.soldier_profile_index) {
                            e.npc.number_of_arrows as i32
                        } else {
                            0
                        }
                    }
                    // Civilians never call InitializeWeapons → no bow → 0.
                    Entity::Civilian(_) => 0,
                    _ => 0,
                }
            }
            // 1: money — requires NPC
            1 => entity.npc_data().map_or_else(
                || {
                    tracing::warn!("Script Error: GetPersistentProperty 'money' on non-NPC");
                    -1
                },
                |npc| npc.money as i32,
            ),
            // 2: life points — requires human
            2 => {
                if !entity.is_human() {
                    tracing::warn!(
                        "Script Error: GetPersistentProperty 'life points' on non-human"
                    );
                    return -1;
                }
                match entity {
                    Entity::Pc(e) => e.pc.life_points as i32,
                    Entity::Soldier(e) => e.npc.life_points as i32,
                    Entity::Civilian(e) => e.npc.life_points as i32,
                    _ => -1,
                }
            }
            // 3: concussion — requires human
            3 => entity.human_data().map_or_else(
                || {
                    tracing::warn!("Script Error: GetPersistentProperty 'concussion' on non-human");
                    -1
                },
                |h| h.concussion_of_the_brain as i32,
            ),
            // 4–11: PC ammo properties
            4..=11 => {
                let pc = match entity.pc_data() {
                    Some(pc) => pc,
                    None => {
                        tracing::warn!("Script Error: GetPersistentProperty prop {prop} on non-PC");
                        return -1;
                    }
                };
                let action = match prop {
                    4 => Action::Purse,
                    5 => Action::Stone,
                    6 => Action::Apple,
                    7 => Action::Ale,
                    8 => Action::Eat,
                    9 => Action::Heal,
                    10 => Action::Net,
                    11 => Action::WaspNest,
                    _ => unreachable!(),
                };
                self.campaign
                    .as_ref()
                    .and_then(|c| c.get_character_by_profile(pc.profile_index))
                    .and_then(|idx| self.campaign.as_ref()?.characters.get(idx))
                    .map(|desc| desc.status.get_ammo(action))
                    .or_else(|| pc.ammo.get(action))
                    .expect("persistent PC ammo property has a live counter") as i32
            }
            _ => {
                tracing::warn!("Script Error: GetPersistentProperty invalid property {prop}");
                -1
            }
        }
    }

    fn set_persistent_property(&mut self, actor: i32, prop: i32, amount: i32) -> bool {
        use crate::pc_status::SpecialPeasantName;
        use crate::profiles::Action;

        // First handle entity-level mutations (money, life_points, concussion)
        match prop {
            // 1: money — requires NPC
            1 => {
                return match self.get_entity_mut(actor) {
                    Some(Entity::Soldier(e)) => {
                        e.npc.money = amount as u32;
                        true
                    }
                    Some(Entity::Civilian(e)) => {
                        e.npc.money = amount as u32;
                        true
                    }
                    Some(_) => {
                        tracing::warn!("Script Error: SetPersistentProperty 'money' on non-NPC");
                        false
                    }
                    None => {
                        tracing::warn!("Script Error: SetPersistentProperty invalid actor");
                        false
                    }
                };
            }
            // 2: life points — requires human.  Routing through the
            // full helper needs engine-level access (Sherwood/forest
            // level flag, max-life lookups, the death cascade), so we
            // queue a deferred command instead of writing the field
            // here.
            2 => {
                let is_human = match self.get_entity(actor) {
                    Some(e) => e.is_human(),
                    None => {
                        tracing::warn!("Script Error: SetPersistentProperty invalid actor");
                        return false;
                    }
                };
                if !is_human {
                    tracing::warn!(
                        "Script Error: SetPersistentProperty 'life points' on non-human"
                    );
                    return false;
                }
                self.deferred_commands
                    .push(DeferredCommand::SetScriptedLifePoints { actor, amount });
                return true;
            }
            // 12: name — requires PC, amount selects SPECIAL_PEASANT_A/B/C.
            // Validates `IsPC()`, switches on `amount ∈ {NAME_A, NAME_B,
            // NAME_C}`, and overwrites the PC's display name with the
            // menu-text string for the matching SPECIAL_PEASANT slot.
            // The slot id lives on `PcStatus::name_override` and the
            // localized string is resolved via `MenuTextLookup` at
            // display time (see `PcStatus::display_name`).
            12 => {
                if !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                    tracing::warn!(
                        "Script Error: SetPersistentProperty 'name' on non-PC (actor {actor})"
                    );
                    return false;
                }
                let Some(slot) = SpecialPeasantName::from_amount(amount) else {
                    tracing::warn!(
                        "Script Error: SetPersistentProperty 'name' invalid name ID {amount}"
                    );
                    return false;
                };
                let profile_index = match self.get_entity(actor).and_then(|e| e.pc_data()) {
                    Some(pc) => pc.profile_index,
                    None => return false,
                };
                if let Some(campaign) = self.campaign.as_mut()
                    && let Some(char_idx) = campaign.get_character_by_profile(profile_index)
                    && let Some(desc) = campaign.characters.get_mut(char_idx)
                {
                    desc.status.name_override = Some(slot);
                    return true;
                }
                // No campaign slot — the PC isn't part of the gang
                // (e.g. demo missions exclude most profiles).  We have
                // no equivalent off-campaign storage, so the rename is
                // dropped with a debug log rather than a hard error.
                tracing::debug!(
                    "SetPersistentProperty 'name': actor {actor} has no campaign slot; rename dropped"
                );
                return false;
            }
            // 3: concussion — requires human.  The setter is called with
            // `force_value == true`.  The KO/wakeup state-machine,
            // swordfight quit, healing-timeout, and titbit/event side
            // effects all live in the engine wrapper `apply_concussion`,
            // so we defer.
            3 => {
                let is_human = match self.get_entity(actor) {
                    Some(e) => e.is_human(),
                    None => {
                        tracing::warn!("Script Error: SetPersistentProperty invalid actor");
                        return false;
                    }
                };
                if !is_human {
                    tracing::warn!("Script Error: SetPersistentProperty 'concussion' on non-human");
                    return false;
                }
                self.deferred_commands
                    .push(DeferredCommand::SetScriptedConcussion {
                        actor,
                        amount,
                        force_value: true,
                    });
                return true;
            }
            _ => {}
        }

        // For ammo properties (0, 4–11), validate the live actor first.
        // The original calls RHElementActorPC::SetAmmoAmount directly;
        // campaign persistence is an additional mirror, not a prerequisite.
        let action = match prop {
            0 => Some(Action::Bow),
            4 => Some(Action::Purse),
            5 => Some(Action::Stone),
            6 => Some(Action::Apple),
            7 => Some(Action::Ale),
            8 => Some(Action::Eat),
            9 => Some(Action::Heal),
            10 => Some(Action::Net),
            11 => Some(Action::WaspNest),
            _ => None,
        };

        if let Some(action) = action {
            // Validate entity type and extract profile index.
            let profile_index = match self.get_entity(actor) {
                Some(entity) => {
                    if prop == 0 {
                        // Arrows: must be human and have a bow.
                        if !entity.is_human() {
                            tracing::warn!(
                                "Script Error: SetPersistentProperty 'arrows' on non-human"
                            );
                            return false;
                        }
                    } else {
                        // Props 4–11: PC-only.
                        if !entity.is_pc() {
                            tracing::warn!(
                                "Script Error: SetPersistentProperty prop {prop} on non-PC"
                            );
                            return false;
                        }
                    }
                    match entity.pc_data() {
                        Some(pc) => pc.profile_index,
                        None => {
                            // For an NPC archer, write `number_of_arrows`
                            // directly.  C++ gates `SetPersistentProperty`
                            // through `GetBow()`, which is constructed from
                            // the soldier profile's shooting weapon id.  The
                            // AI archer-unit flag is not enough: officer
                            // scripts can have archer AI without an actual
                            // bow weapon.
                            if prop == 0 {
                                let soldier_profile_index = match self.get_entity(actor) {
                                    Some(Entity::Soldier(s)) => s.soldier.soldier_profile_index,
                                    Some(_) => {
                                        tracing::debug!(
                                            "SetPersistentProperty: actor {actor} is not a PC, skipping ammo"
                                        );
                                        return false;
                                    }
                                    None => {
                                        tracing::warn!(
                                            "Script Error: SetPersistentProperty invalid actor {actor}"
                                        );
                                        return false;
                                    }
                                };
                                if !self.soldier_has_bow_profile(soldier_profile_index) {
                                    tracing::warn!(
                                        "Script Error: SetPersistentProperty 'arrows' on soldier without bow profile"
                                    );
                                    return false;
                                }
                                let Some(Some(Entity::Soldier(s))) =
                                    self.entities.get_mut(actor as usize)
                                else {
                                    tracing::warn!(
                                        "Script Error: SetPersistentProperty invalid actor {actor}"
                                    );
                                    return false;
                                };
                                s.npc.number_of_arrows = amount as u16;
                                tracing::debug!(
                                    actor,
                                    amount,
                                    "SetPersistentProperty: NPC bow ammo set"
                                );
                                return true;
                            }
                            tracing::debug!(
                                "SetPersistentProperty: actor {actor} is not a PC, skipping ammo"
                            );
                            return false;
                        }
                    }
                }
                None => {
                    tracing::warn!("Script Error: SetPersistentProperty invalid actor {actor}");
                    return false;
                }
            };

            // PC bow-presence guard: return `false` without mutating
            // when the PC has no bow.  Done explicitly rather than
            // relying on the downstream `max == 0` rejection so the
            // intent (and the early-out site) is visible.
            if prop == 0
                && !self
                    .profile_manager
                    .get_character(profile_index)
                    .is_some_and(|p| p.has_action(Action::Bow))
            {
                tracing::debug!(
                    actor,
                    "SetPersistentProperty: PC profile has no bow action; rejecting arrow set"
                );
                return false;
            }

            // Maximal ammo reads from the profile and applies difficulty
            // scaling before either live or campaign state is changed.
            let difficulty = crate::player_profile::PlayerProfileManager::global()
                .as_ref()
                .and_then(|mgr| mgr.get_active())
                .map(|p| p.difficulty)
                .unwrap_or(crate::player_profile::DifficultyLevel::Medium);
            let Some(profile) = self.profile_manager.get_character(profile_index) else {
                tracing::warn!(
                    ?profile_index,
                    "Script Error: SetPersistentProperty PC actor has no character profile"
                );
                return false;
            };
            let max = crate::inventory::max_ammo_for_action(profile, action, difficulty);
            let action_slot = crate::inventory::find_action_slot(profile, action);
            let amount_u16 = amount as u16;
            if max == 0 || amount_u16 > max {
                tracing::debug!(
                    actor,
                    ?action,
                    amount,
                    max,
                    "SetPersistentProperty: silently rejecting over-cap ammo write"
                );
                return false;
            }

            if let Some(campaign) = self.campaign.as_mut() {
                if let Some(char_idx) = campaign.get_character_by_profile(profile_index) {
                    let Some(desc) = campaign.characters.get_mut(char_idx) else {
                        tracing::warn!(
                            char_idx,
                            ?profile_index,
                            "SetPersistentProperty: campaign character index is missing"
                        );
                        return false;
                    };
                    desc.status.set_ammo(action, amount_u16);
                } else {
                    // A campaign slot is not required by the original live
                    // actor call, but a campaign that omits the actor's
                    // backing description is useful provenance when saves
                    // are inspected.
                    tracing::warn!(
                        ?profile_index,
                        "SetPersistentProperty: PC has no campaign character; updating live actor only"
                    );
                }
            }

            let Some(Entity::Pc(pc)) = self.get_entity_mut(actor) else {
                tracing::warn!(
                    "Script Error: SetPersistentProperty required PC actor {actor} disappeared"
                );
                return false;
            };
            pc.pc
                .ammo
                .set(action, amount_u16)
                .expect("persistent PC ammo property has a live counter");

            // Toggle the live PC entity's profile action slot so the HUD
            // reflects the new ammo. C++ resolves through
            // GetActionIndex(action); the action enum value is not the
            // portrait slot.
            if let Some(action_slot) = action_slot
                && action_slot < pc.pc.disabled_actions.len()
            {
                if amount_u16 == 0 {
                    pc.pc.disabled_actions[action_slot] = true;
                    if pc.pc.current_action == action {
                        pc.pc.current_action = Action::NoAction;
                    }
                    if pc.pc.saved_action == action {
                        pc.pc.saved_action = Action::NoAction;
                    }
                } else {
                    pc.pc.disabled_actions[action_slot] = false;
                }
            }
            return true;
        }

        tracing::warn!("Script Error: SetPersistentProperty invalid property {prop}");
        false
    }

    fn soldier_has_bow_profile(&self, profile_index: crate::profiles::SoldierProfileIdx) -> bool {
        let Some(profile) = self.profile_manager.get_soldier(profile_index) else {
            tracing::warn!(
                ?profile_index,
                "soldier_has_bow_profile: missing soldier profile"
            );
            return false;
        };
        self.profile_manager
            .get_bow(profile.shooting_weapon_id)
            .is_some()
    }
}

impl Default for GameHost {
    fn default() -> Self {
        Self::new()
    }
}

// ── Handle resolution helpers ────────────────────────────────────────
impl GameHost {
    fn make_script_handle(tag: i32, index: usize) -> i32 {
        let index = i32::try_from(index).expect("script handle index exceeds i32 range");
        assert!(
            (0..=SCRIPT_HANDLE_INDEX_MASK).contains(&index),
            "script handle index exceeds 28-bit payload: {index}"
        );
        tag | index
    }

    pub fn actor_handle<I: Into<EntityId>>(id: I) -> i32 {
        let id = id.into();
        Self::make_script_handle(SCRIPT_HANDLE_ACTOR_TAG, id.index() as usize)
    }

    pub fn actor_handle_from_index(index: usize) -> i32 {
        Self::make_script_handle(SCRIPT_HANDLE_ACTOR_TAG, index)
    }

    pub fn door_handle_from_index(index: usize) -> i32 {
        Self::make_script_handle(SCRIPT_HANDLE_DOOR_TAG, index)
    }

    pub fn location_handle_from_index(index: usize) -> i32 {
        Self::make_script_handle(SCRIPT_HANDLE_LOCATION_TAG, index)
    }

    pub fn sound_source_handle_from_index(index: usize) -> i32 {
        Self::make_script_handle(SCRIPT_HANDLE_SOUND_SOURCE_TAG, index)
    }

    pub fn building_handle_from_index(index: usize) -> i32 {
        Self::make_script_handle(SCRIPT_HANDLE_BUILDING_TAG, index)
    }

    pub fn actor_handle_index(handle: i32) -> Option<usize> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_ACTOR_TAG)
    }

    pub fn door_index(handle: i32) -> Option<usize> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_DOOR_TAG)
    }

    pub fn patch_index(handle: i32) -> Option<usize> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_PATCH_TAG)
    }

    pub fn location_index(handle: i32) -> Option<usize> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_LOCATION_TAG)
    }

    pub fn sound_source_index(handle: i32) -> Option<usize> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_SOUND_SOURCE_TAG)
    }

    pub fn building_index(handle: i32) -> Option<usize> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_BUILDING_TAG)
    }

    pub fn way_index(handle: i32) -> Option<usize> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_WAY_TAG)
    }

    fn handle_has_tag(handle: i32, tag: i32) -> bool {
        handle > 0 && (handle & !SCRIPT_HANDLE_INDEX_MASK) == tag
    }

    fn typed_handle_to_index(handle: i32, tag: i32) -> Option<usize> {
        Self::handle_has_tag(handle, tag).then_some((handle & SCRIPT_HANDLE_INDEX_MASK) as usize)
    }

    /// Whether `loc` is a script-sector handle (as opposed to a
    /// script-point handle or unrelated entity handle). Script location
    /// payload indices are laid out as `[points..., sectors...]`, so a
    /// sector handle satisfies `script_point_count <= idx < script_location_count`.
    /// Used as the script-sector type guard by
    /// `GetNumberOfActorsInSector` / `GetActorInSector` etc.
    fn is_script_sector_handle(&self, loc: i32) -> bool {
        let Some(idx) = Self::typed_handle_to_index(loc, SCRIPT_HANDLE_LOCATION_TAG) else {
            return false;
        };
        idx >= self.script_point_count && idx < self.script_location_count
    }

    fn get_door(&self, handle: i32) -> Option<&Door> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_DOOR_TAG)
            .and_then(|idx| self.doors.get(idx))
    }

    fn get_door_mut(&mut self, handle: i32) -> Option<&mut Door> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_DOOR_TAG)
            .and_then(|idx| self.doors.get_mut(idx))
    }

    fn get_patch(&self, handle: i32) -> Option<&Patch> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_PATCH_TAG)
            .and_then(|idx| self.patches.get(idx))
    }

    fn get_patch_mut(&mut self, handle: i32) -> Option<&mut Patch> {
        Self::typed_handle_to_index(handle, SCRIPT_HANDLE_PATCH_TAG)
            .and_then(|idx| self.patches.get_mut(idx))
    }

    /// Resolve an actor handle to a character profile index.
    /// Tries the engine PC profile map first, then treats the handle
    /// as a raw profile index (for campaign-only contexts like Sherwood).
    fn resolve_profile(&self, actor: i32) -> Option<crate::profiles::CharacterProfileIdx> {
        self.pc_profile_map.get(&actor).copied().or_else(|| {
            // Fallback: treat as raw profile index (pre-engine convention)
            self.campaign.as_ref()?;
            let idx = crate::profiles::CharacterProfileIdx(actor as u32);
            if self.profile_manager.get_character(idx).is_some() {
                Some(idx)
            } else {
                None
            }
        })
    }

    /// True iff `handle` refers to a script *point* (as opposed to a
    /// sector).  Used to reject non-point locations in `GetDistance`,
    /// `ComputeLocationBetween`, camera natives, etc.  Static script
    /// locations are laid out as `[points ...] [sectors ...]` in
    /// `location_positions` (so index < `script_point_count` = point);
    /// dynamically-computed locations (`GetActorLocation`,
    /// `ComputeLocationBetween`) are always points.
    fn is_script_point(&self, handle: i32) -> bool {
        let Some(idx) = Self::location_index(handle) else {
            return false;
        };
        if idx < self.script_point_count {
            return true;
        }
        // Computed locations live past `script_location_count` and are
        // always points.
        idx >= self.script_location_count
            && (idx - self.script_location_count) < self.computed_locations.len()
    }

    /// Resolve a location handle to its (x, y) position.
    /// Handles 1..=script_location_count are static locations from level data.
    /// Handles beyond that are dynamically computed by script natives.
    fn resolve_location_pos(&self, handle: i32) -> Option<(f32, f32)> {
        let idx = Self::location_index(handle)?;
        if idx < self.script_location_count {
            self.location_positions.get(idx).copied()
        } else {
            let computed_idx = idx - self.script_location_count;
            self.computed_locations.get(computed_idx).copied()
        }
    }

    /// Resolve a location handle to its (layer, sector_number).
    ///
    /// Static script locations carry layer/sector data — points and
    /// sectors loaded from `RawScriptObjects`.  Dynamically computed
    /// locations (`GetActorLocation`, `ComputeLocationBetween`, …) also
    /// carry layer/sector when created via the host natives that
    /// have that metadata available; otherwise they return `None`.
    /// These reads back the `RecordEnterGame` layer/sector pickup and
    /// the `SetActorLocation` sector refresh.
    fn resolve_location_layer_sector(&self, handle: i32) -> Option<(u16, u16)> {
        let idx = Self::location_index(handle)?;
        if idx < self.script_location_count {
            return Some((
                *self.location_layers.get(idx)?,
                *self.location_sectors.get(idx)?,
            ));
        }
        let computed_idx = idx - self.script_location_count;
        self.computed_location_layers.get(computed_idx).copied()?
    }

    /// Create a new dynamic location at (x, y) and return its script handle.
    /// `layer_sector` carries the source actor/point's (layer, sector); pass
    /// `None` for points without associated sector geometry.
    fn create_computed_location_full(
        &mut self,
        x: f32,
        y: f32,
        layer_sector: Option<(u16, u16)>,
    ) -> i32 {
        self.computed_locations.push((x, y));
        self.computed_location_layers.push(layer_sector);
        Self::location_handle_from_index(
            self.script_location_count + self.computed_locations.len() - 1,
        )
    }

    /// Validate a 0-based script-object index and return its opaque script
    /// handle, or 0 with an error log if out of range. Common shape for
    /// the `GetXScript` family of natives (doors, patches, locations,
    /// sound sources, buildings, hiking paths).
    ///
    /// `-1` means "no script reference" and silently returns null.
    fn script_index_to_handle(idx: i32, count: usize, kind: &str, tag: i32) -> i32 {
        if idx == -1 {
            return 0;
        }
        if idx >= 0 && (idx as usize) < count {
            return Self::make_script_handle(tag, idx as usize);
        }
        tracing::error!("Script Error: invalid {kind} ID {idx} (max={count})");
        0
    }

    /// Shared null-location-handle check for camera commands. Logs a
    /// warning tagged with the native's name when `loc == 0` and returns
    /// `false` so the caller can skip queueing its command.
    fn check_camera_location(loc: i32, native: &str) -> bool {
        if loc == 0 {
            tracing::warn!("Script Error: {native} called with NULL location");
            false
        } else {
            true
        }
    }

    // ── Native helpers ──────────────────────────────────────────
    //
    // These back the per-opcode arms in `HostFunctions::call` with the
    // per-actor-type dispatch.

    /// Common body for the `Activate` / `Deactivate` script natives.
    /// Returns `true` on a valid handle, `false` if the actor is
    /// missing — SCB scripts occasionally branch on the return value.
    fn script_activate_actor(&mut self, actor: i32, activate: bool) -> bool {
        // Phase 1: classify entity type (immutable borrow, released
        // before phase 2).
        enum Action {
            Pc,
            General,
            Invalid,
        }

        let action = match self.get_entity(actor) {
            Some(entity) if entity.is_pc() => Action::Pc,
            Some(_) => Action::General,
            None => Action::Invalid,
        };

        // Phase 2: apply changes with separate mutable borrows.
        match action {
            Action::Pc => {
                // PCs go through `playable` instead of `active`:
                // toggles `playable` without touching `active`, then
                // sends portrait-bar enable/disable messages.
                if let Some(entity) = self.get_entity_mut(actor)
                    && let Some(pc) = entity.pc_data_mut()
                {
                    pc.playable = activate;
                    if !activate {
                        // The Deactivate PC branch walks every
                        // quick-action memory slot, deleting
                        // seek/action sequences, resetting QUICKITOS,
                        // zeroing special-QA counts, removing QA
                        // titbits, and storing the empty-titbit
                        // sentinel.  Apply the entity-local state
                        // here; the engine-side helper clears
                        // titbit/macro-store state post-script.
                        pc.quick_action_types.clear();
                        for slot in pc.quick_action_sequences.iter_mut() {
                            *slot = None;
                        }
                        pc.titbits.clear();
                    }
                }
                // Queue portrait bar update.
                self.deferred_commands.push(DeferredCommand::SetPlayable {
                    actor,
                    playable: activate,
                });
                // On deactivate, also queue engine-side cleanup of QA
                // titbits and macro-store slots.
                if !activate {
                    self.deferred_commands
                        .push(DeferredCommand::ClearAllQuickActionSlots { actor });
                }
            }
            Action::General => {
                // Soldiers, civilians, animals, objects, etc.
                if let Some(entity) = self.get_entity_mut(actor) {
                    entity.element_data_mut().active = activate;
                }
            }
            Action::Invalid => {
                tracing::warn!(
                    "{}: invalid actor handle {actor}",
                    if activate { "Activate" } else { "Deactivate" }
                );
                return false;
            }
        }
        true
    }

    /// Common body for the `LockAI` script native.
    ///
    /// The actual `script_lock` call is routed through
    /// `DeferredCommand::ScriptLockAI` so the engine-side handler can
    /// peek the actor's currently-running sequence command — we need it
    /// to gate `Stop()`: only call `Stop()` when the current command is
    /// not already `LockAi`.  `GameHost` doesn't see the sequence
    /// manager directly, so the peek has to live in the engine handler.
    fn script_lock_ai(&mut self, actor: i32, remember_events: bool) {
        let Some(entity) = self.get_entity_mut(actor) else {
            tracing::warn!("LockAI: invalid actor handle {actor}");
            return;
        };

        if entity.is_npc() {
            self.deferred_commands.push(DeferredCommand::ScriptLockAI {
                actor,
                send_back: remember_events,
            });
        } else {
            tracing::warn!("LockAI: tried to lock the AI of a PC ({actor})");
        }
    }

    /// Common body for the `UnlockAI` script native.
    fn script_unlock_ai(&mut self, actor: i32) {
        let Some(entity) = self.get_entity_mut(actor) else {
            tracing::warn!("UnlockAI: invalid actor handle {actor}");
            return;
        };

        if entity.is_npc() {
            let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
            if let Some(ai) = entity.ai_controller_mut() {
                if ai.script_locked {
                    ai.script_unlock(is_unconscious);
                } else {
                    tracing::warn!("UnlockAI: NPC {actor} is not script-locked");
                }
            }
        } else {
            tracing::warn!("UnlockAI: tried to unlock the AI of a PC ({actor})");
        }
    }

    /// Common body for the `Freeze` script native.  Only PCs have a
    /// readable freeze flag (`fried_psykokwack`); the NPC counterpart
    /// is intentionally a no-op (the original game's NPC freeze flag
    /// was never consulted by the AI tick).
    fn script_freeze_actor(&mut self, actor: i32, freeze: bool) {
        let Some(entity) = self.get_entity_mut(actor) else {
            tracing::warn!("Freeze: invalid actor handle {actor}");
            return;
        };

        if !entity.is_human() {
            tracing::warn!("Freeze: target {actor} is not human");
            return;
        }

        if entity.is_pc()
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.fried_psykokwack = freeze;
        }
        // NPC branch intentionally empty: the C++ NPC freeze flag
        // (`mbFriedPikachu`) is stored on assignment and never consulted —
        // freezing an NPC via this native is a no-op in the original
        // engine.
    }
}

impl HostFunctions for GameHost {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn clone_dyn(&self) -> Box<dyn HostFunctions> {
        Box::new(self.clone())
    }
    fn take_pending_nested_call(&mut self) -> Option<crate::interp::PendingNestedCall> {
        self.pending_nested_call.take()
    }

    fn call(&mut self, index: u32, stack: &mut NativeStack) -> i32 {
        use NativeFn::*;

        if let Ok(f) = NativeFn::try_from(index) {
            match f {
                // --- victory ---
                ForceCheckVictory => {
                    self.force_check = true;
                    0
                }

                // --- globals ---
                InitGlobal => {
                    let value = stack.pop_i32();
                    let id = stack.pop_i32();
                    self.globals.insert(id, value);
                    0
                }
                SetGlobal => {
                    let value = stack.pop_i32();
                    let id = stack.pop_i32();
                    // Script globals must be created by InitGlobal
                    // first; SetGlobal on an un-init'd id warns and
                    // no-ops.
                    if let std::collections::btree_map::Entry::Occupied(mut e) =
                        self.globals.entry(id)
                    {
                        e.insert(value);
                    } else {
                        tracing::warn!("Script Error: Non-valid ID for script global {id}");
                    }
                    0
                }
                GetGlobal => {
                    let id = stack.pop_i32();
                    // Returns -1 with a warning on an un-init'd id.
                    match self.globals.get(&id) {
                        Some(v) => *v,
                        None => {
                            tracing::warn!("Script Error: Non-valid ID for script global {id}");
                            -1
                        }
                    }
                }

                // --- sequence manager ---
                Start => {
                    // If a recording is already active, warn and
                    // return 0 *without mutating state*; otherwise
                    // allocate, set sequence_level = 1, return 1.
                    if self.recording.is_some() {
                        tracing::error!(
                            "Script error in Start: cannot start a new record sequence while another is still being recorded"
                        );
                        0
                    } else {
                        self.recording = Some(RecordingSession::new());
                        self.sequence_id = 1;
                        1
                    }
                }
                Thanx => {
                    // Errors out on "no active recording" (false) and
                    // on "empty recording" (false).  Happy path
                    // launches the sequence and returns true.
                    if let Some(rec) = self.recording.take() {
                        match rec.finalize() {
                            Some(seq) => {
                                self.completed_sequences.push(seq);
                                1
                            }
                            None => {
                                tracing::error!("Script Error: Trying to launch an empty sequence");
                                0
                            }
                        }
                    } else {
                        tracing::error!(
                            "Script error in Thanx: End a sequence recording without ever started it"
                        );
                        0
                    }
                }
                Then => {
                    // If there's no active recording (sequence_level
                    // < 1), warn and return 0 *without mutating
                    // state*; else advance the level and return the
                    // current sequence level.
                    if let Some(rec) = &mut self.recording {
                        let level = rec.advance_level();
                        self.sequence_id = level as i32;
                        level as i32
                    } else {
                        tracing::error!(
                            "Script error in Then: called outside of a Start/Thanx sequence"
                        );
                        0
                    }
                }

                // --- pure functions ---
                IsNull => {
                    let h = stack.pop_i32();
                    if h == 0 { 1 } else { 0 }
                }
                IsActorEqual => {
                    let b = stack.pop_i32();
                    let a = stack.pop_i32();
                    if a == b { 1 } else { 0 }
                }
                IsActorDead => {
                    // Gated on `ActorExists && IsHuman`, returns
                    // life-points <= 0.  Use the life-points-based
                    // `Element::is_dead()` rather than posture so the
                    // native flips true the moment HP reaches 0,
                    // before the death animation rewrites posture.
                    let actor = stack.pop_i32();
                    self.get_entity(actor)
                        .map_or(0, |e| i32::from(e.human_data().is_some() && e.is_dead()))
                }
                IsActorKO => {
                    let actor = stack.pop_i32();
                    self.get_entity(actor).map_or(0, |e| {
                        i32::from(e.human_data().is_some_and(|h| h.unconscious))
                    })
                }
                IsActorTied => {
                    let actor = stack.pop_i32();
                    self.get_entity(actor)
                        .map_or(0, |e| i32::from(e.element_data().posture == Posture::Tied))
                }
                IsActorHS => {
                    // `ActorExists && IsHuman` then
                    // `IsDead() || IsTied() || IsUnconscious()`.
                    // (The previous arm read `in_honolulu`, an
                    // off-map flag, which broke any mission script
                    // gating on incapacitation.)
                    let actor = stack.pop_i32();
                    let Some(e) = self.get_entity(actor) else {
                        tracing::warn!("Script Error: IsActorHS with invalid actor handle {actor}");
                        return 0;
                    };
                    if !e.is_actor() {
                        tracing::warn!("Script Error: IsActorHS with non-actor handle {actor}");
                        return 0;
                    }
                    let posture = e.element_data().posture;
                    let dead = e.is_dead();
                    let tied = posture == Posture::Tied;
                    let unconscious = e.human_data().is_some_and(|h| h.unconscious);
                    i32::from(dead || tied || unconscious)
                }

                // --- actor stop / activation ---

                // Cancels the actor's current sequence element and any
                // pending sequence elements at script priority.  The
                // sequence manager lives on the engine, so we queue a
                // deferred command.
                StopActor => {
                    let actor = stack.pop_i32();
                    if self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                        self.deferred_commands
                            .push(DeferredCommand::StopActor { actor });
                    } else {
                        tracing::warn!("StopActor: invalid or non-actor handle {actor}");
                    }
                    0
                }

                // God is null (as everybody knows).  Used by scripts as
                // a sentinel actor handle in conditional logic.
                God => 0,

                // Only codes 31 (select-all) and 0 (unselect-all) are
                // supported — other values warn and return.
                Select => {
                    let code = stack.pop_i32();
                    match code {
                        31 => self.deferred_commands.push(DeferredCommand::SelectPC {
                            actor: 0,
                            select: true,
                        }),
                        0 => self.deferred_commands.push(DeferredCommand::SelectPC {
                            actor: 0,
                            select: false,
                        }),
                        _ => tracing::warn!(
                            "Select: only codes 31 (select all) and 0 (unselect all) supported, got {code}"
                        ),
                    }
                    // Returns 1 unconditionally (including the warn
                    // branch).
                    1
                }

                // Deactivate dispatch:
                //   - Mobile  → SetActiveAll(false) (propagates to sub-sprites)
                //   - PC      → SetPlayable(false) + clear quick-action icons
                //   - General → SetActive(false)
                Deactivate => {
                    let actor = stack.pop_i32();
                    if self.script_activate_actor(actor, false) {
                        1
                    } else {
                        0
                    }
                }

                // Inverse of Deactivate.
                Activate => {
                    let actor = stack.pop_i32();
                    if self.script_activate_actor(actor, true) {
                        1
                    } else {
                        0
                    }
                }

                // --- AI control ---

                // For NPCs sets the AI script-lock flag (with a
                // "remember events" bit for replaying stimuli on
                // unlock).  PCs produce a script error (no-op here).
                LockAI => {
                    let remember = stack.pop_i32() != 0;
                    let actor = stack.pop_i32();
                    self.script_lock_ai(actor, remember);
                    0
                }

                // Inverse of LockAI.  The animal-kick branch is gone
                // with the rest of the animal system.
                UnlockAI => {
                    let actor = stack.pop_i32();
                    self.script_unlock_ai(actor);
                    0
                }

                // Sets the NPC or PC freeze flag, which causes the
                // actor's per-frame Hourglass tick to early-return.
                Freeze => {
                    let freeze = stack.pop_i32() != 0;
                    let actor = stack.pop_i32();
                    self.script_freeze_actor(actor, freeze);
                    0
                }

                // Flips the engine-global freeze flag which gates AI,
                // combat, movement, and animation ticks.  Deferred to
                // avoid needing engine access.
                FreezeAll => {
                    let freeze = stack.pop_i32() != 0;
                    self.deferred_commands
                        .push(DeferredCommand::FreezeAll { freeze });
                    0
                }

                // --- location / distance ---
                NoWhere => 0,
                GetDistance => {
                    // Both arguments must be points; on a sector or
                    // null handle, warn and return 0.
                    let loc_b = stack.pop_i32();
                    let loc_a = stack.pop_i32();
                    if !self.is_script_point(loc_a) {
                        tracing::error!(
                            "Script Error: 1st argument of GetDistance is no point (handle {loc_a})"
                        );
                        0
                    } else if !self.is_script_point(loc_b) {
                        tracing::error!(
                            "Script Error: 2nd argument of GetDistance is no point (handle {loc_b})"
                        );
                        0
                    } else {
                        match (
                            self.resolve_location_pos(loc_a),
                            self.resolve_location_pos(loc_b),
                        ) {
                            (Some(pos_a), Some(pos_b)) => {
                                let dx = pos_b.0 - pos_a.0;
                                let dy = pos_b.1 - pos_a.1;
                                (dx * dx + dy * dy).sqrt() as i32
                            }
                            _ => {
                                tracing::warn!(
                                    "GetDistance: invalid location handle(s) {loc_a}, {loc_b}"
                                );
                                0
                            }
                        }
                    }
                }
                Rand => {
                    let max = stack.pop_i32();
                    crate::sim_rng::script_rand(crate::sim_rng::RngSite::ScriptRand, max)
                        .unwrap_or_else(|error| panic!("{error}"))
                }
                PrintConsole => {
                    // Originally blits "%d\n" into the in-game
                    // debug-console overlay.  Closest analogue here is
                    // a tracing line — the debug overlay is dev-only.
                    let value = stack.pop_i32();
                    tracing::info!(target: "rh_script_console", "{value}");
                    0
                }

                // --- custom values (campaign-backed) ---
                // Range-check id against script-side index 0..=19
                // (CUSTOM_VALUE_1 .. CUSTOM_VALUE_20) and warn+return
                // on out-of-range.
                GetCustomCampaignValue => {
                    let id = stack.pop_i32();
                    if !(0..=19).contains(&id) {
                        tracing::warn!(
                            "GetCustomCampaignValue: invalid index {id} (must be 0..=19)"
                        );
                        return 0;
                    }
                    *self.campaign_values.get(&id).unwrap_or(&0)
                }
                SetCustomCampaignValue => {
                    let value = stack.pop_i32();
                    let id = stack.pop_i32();
                    if !(0..=19).contains(&id) {
                        tracing::warn!(
                            "SetCustomCampaignValue: invalid index {id} (must be 0..=19)"
                        );
                        return 0;
                    }
                    self.campaign_values.insert(id, value);
                    0
                }
                // Validate id in script-side range 0..=9
                // (CUSTOM_NPC_VALUE_1 .. CUSTOM_NPC_VALUE_10),
                // ActorExists, and IsNPC; each with a warn + return
                // -1 on failure.
                GetCustomNPCValue => {
                    let id = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if !(0..=9).contains(&id) {
                        tracing::warn!("GetCustomNPCValue: invalid index {id} (must be 0..=9)");
                        return -1;
                    }
                    match self.get_entity(actor) {
                        None => {
                            tracing::warn!("GetCustomNPCValue: actor {actor} does not exist");
                            -1
                        }
                        Some(e) if !e.is_npc() => {
                            tracing::warn!("GetCustomNPCValue: actor {actor} is not an NPC");
                            -1
                        }
                        Some(_) => *self.npc_values.get(&(actor, id)).unwrap_or(&0),
                    }
                }
                SetCustomNPCValue => {
                    let value = stack.pop_i32();
                    let id = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if !(0..=9).contains(&id) {
                        tracing::warn!("SetCustomNPCValue: invalid index {id} (must be 0..=9)");
                        return 0;
                    }
                    match self.get_entity(actor) {
                        None => {
                            tracing::warn!("SetCustomNPCValue: actor {actor} does not exist");
                        }
                        Some(e) if !e.is_npc() => {
                            tracing::warn!("SetCustomNPCValue: actor {actor} is not an NPC");
                        }
                        Some(_) => {
                            self.npc_values.insert((actor, id), value);
                        }
                    }
                    0
                }

                // --- bitwise ops ---
                BitwiseAnd => {
                    let b = stack.pop_i32();
                    let a = stack.pop_i32();
                    a & b
                }
                BitwiseOr => {
                    let b = stack.pop_i32();
                    let a = stack.pop_i32();
                    a | b
                }
                BitwiseXor => {
                    let b = stack.pop_i32();
                    let a = stack.pop_i32();
                    a ^ b
                }

                // --- PC actions ---
                HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood => {
                    // Iterates the spawned PCs (not the campaign-wide
                    // gang list).  For each live PC carrying the
                    // requested action it returns true if the PC is
                    // alive; otherwise checks for a Sherwood
                    // replacement (non-VIP, portrait still displayed,
                    // profile in Sherwood).  The portrait-displayed
                    // gate tracks "death is recent / corpse still
                    // active in the UI" — i.e. the PC entity is still
                    // spawned in the level.  Iterating `pc_handles`
                    // (live PC entities, alive or corpse) is the
                    // natural mirror: once the corpse is despawned
                    // the PC drops out of the snapshot.
                    let action_code = stack.pop_i32();
                    let Ok(script_action) =
                        crate::profiles::ScriptAction::try_from(action_code as u32)
                    else {
                        tracing::warn!(
                            "Script Error: HasAnyPCAction with bad action ID {action_code}"
                        );
                        return 0;
                    };
                    let action = script_action.to_action();

                    let Some(campaign) = self.campaign.as_ref() else {
                        return 0;
                    };
                    let profiles = &self.profile_manager;

                    for handle in &self.pc_handles {
                        let Some(&profile_idx) = self.pc_profile_map.get(handle) else {
                            continue;
                        };
                        let Some(cp) = profiles.get_character(profile_idx) else {
                            continue;
                        };

                        let has_action =
                            cp.actions.contains(&action) || cp.contextual_actions.contains(&action);
                        if !has_action {
                            continue;
                        }

                        let is_dead = self.get_entity(*handle).is_some_and(|e| e.is_dead());

                        if !is_dead {
                            return 1;
                        }

                        // Dead PC — can we get a replacement from Sherwood?
                        // (Live entity proxy already covers the
                        // portrait-displayed guard.)
                        if !cp.vip && campaign.is_in_sherwood(profile_idx) {
                            return 1;
                        }
                    }

                    0
                }
                // --- profile/campaign-backed queries (reading real data) ---
                // Returns the count of PC actors currently spawned in
                // the running mission, not the campaign roster.
                GetNumberOfPCs => self.pc_handles.len() as i32,
                GetPC => {
                    // Returns a live PC actor handle that scripts pass
                    // straight into other natives.  Indexes the
                    // per-tick `pc_handles` snapshot.
                    let idx = stack.pop_i32();
                    if idx < 0 {
                        0
                    } else {
                        self.pc_handles.get(idx as usize).copied().unwrap_or(0)
                    }
                }
                GetRansomMoney => self
                    .campaign
                    .as_ref()
                    .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
                    .unwrap_or_else(|| {
                        tracing::warn!("Script Error: GetRansomMoney called outside campaign mode");
                        -1
                    }),
                SetRansomMoney => {
                    let val = stack.pop_i32();
                    if self.campaign.is_some() {
                        self.set_campaign_value(crate::campaign::CampaignValue::Ransom, val);
                    } else {
                        tracing::warn!("Script Error: SetRansomMoney called outside campaign mode");
                    }
                    0
                }
                GetDifficultyLevel => {
                    crate::player_profile::DifficultyLevel::current().to_u32() as i32
                }
                GetSizeOfMissionTeam => self
                    .campaign
                    .as_ref()
                    .map_or(0, |c| c.get_size_of_mission_team() as i32),
                // Forwards to `Campaign::is_mission_team_valid`.
                // Returns 0 when no campaign/next-mission context is
                // established (script calling before a mission is
                // chosen) so SCB doesn't see a spurious "team valid".
                IsMissionTeamValid => {
                    let profiles = self.profile_manager.clone();
                    self.campaign.as_ref().map_or(0, |c| {
                        if c.next_mission_idx.is_some() {
                            c.is_mission_team_valid(&profiles) as i32
                        } else {
                            0
                        }
                    })
                }
                GetNumberOfPCsAlive => {
                    // Iterate the loaded PC roster and count those
                    // with life-points > 0 — a per-mission, per-tick
                    // aliveness count.  Use the host-side
                    // `pc_handles` snapshot and the life-points-based
                    // `Entity::is_dead`.
                    self.pc_handles
                        .iter()
                        .filter(|&&h| self.get_entity(h).is_some_and(|e| !e.is_dead()))
                        .count() as i32
                }
                AreAllBlazonsWon => {
                    // Compares the live blazon inventory against the
                    // campaign's max, so a spent/lost blazon can flip
                    // it back to false even if its mission is still
                    // marked done.
                    let profiles = self.profile_manager.clone();
                    self.campaign.as_ref().map_or(0, |campaign| {
                        let current = campaign.get_value(crate::campaign::CampaignValue::Blazon);
                        let max = campaign.get_max_number_of_blazons(&profiles) as i32;
                        if current >= max { 1 } else { 0 }
                    })
                }
                SecretAgentsAreBackInSherwood => self
                    .campaign
                    .as_ref()
                    .map_or(0, |c| if c.are_reservists_back() { 1 } else { 0 }),
                // Returns the packed 16-bit mission ID (e.g.
                // `'A','1' → 0x3141`), NOT the sequential profile_idx.
                GetLastPlayedMission => self.campaign.as_ref().map_or(0, |campaign| {
                    campaign
                        .last_mission_idx
                        .and_then(|idx| campaign.missions.get(idx))
                        .and_then(|m| m.profile_idx)
                        .and_then(|pi| self.profile_manager.missions.get(pi as usize))
                        .map_or(0, |mp| mp.id as i32)
                }),
                GetNextPlayedMission => self.campaign.as_ref().map_or(0, |campaign| {
                    campaign
                        .next_mission_idx
                        .and_then(|idx| campaign.missions.get(idx))
                        .and_then(|m| m.profile_idx)
                        .and_then(|pi| self.profile_manager.missions.get(pi as usize))
                        .map_or(0, |mp| mp.id as i32)
                }),

                // --- entity handle / script lookup ---
                // Handles are opaque non-null VM values with 0-based payload indices.
                // The legacy mobile-element tier is dead engine code and is not modelled.
                GetActorScript => {
                    let idx = stack.pop_i32();
                    let script_count = self.entities.len();
                    if idx == -1 {
                        0
                    } else if idx < 0 {
                        panic!("GetActorScript: negative actor ID {idx}");
                    } else if (idx as usize) < script_count {
                        if self.entities[idx as usize].is_some() {
                            Self::actor_handle_from_index(idx as usize)
                        } else {
                            // legacy implementation returns `marrayElementsScript[idx]`
                            // directly; in-range NULL entries are valid
                            // placeholders (e.g. unfilled BeamMe slots)
                            // and return a null script handle without an
                            // SBError.
                            0
                        }
                    } else {
                        tracing::debug!(
                            "Script Error: invalid actor ID {idx} (max={script_count})"
                        );
                        0
                    }
                }
                GetDoorScript => Self::script_index_to_handle(
                    stack.pop_i32(),
                    self.doors.len(),
                    "door",
                    SCRIPT_HANDLE_DOOR_TAG,
                ),
                GetPatchScript => Self::script_index_to_handle(
                    stack.pop_i32(),
                    self.patches.len(),
                    "patch",
                    SCRIPT_HANDLE_PATCH_TAG,
                ),
                GetLocationScript => Self::script_index_to_handle(
                    stack.pop_i32(),
                    self.script_location_count,
                    "location",
                    SCRIPT_HANDLE_LOCATION_TAG,
                ),
                GetSoundSourceScript => {
                    // If the slot was nulled by a prior
                    // `DestroySoundSource`, log the "already been
                    // destroyed" error and return NULL.  The generic
                    // `script_index_to_handle` only bounds-checks
                    // `sources.len()`, which `delete` does not shrink
                    // — so consult the per-slot `sound_source_alive`
                    // flag too.
                    let idx = stack.pop_i32();
                    if idx == -1 {
                        0
                    } else if idx >= 0 && (idx as usize) < self.sound_source_count {
                        if self
                            .sound_source_alive
                            .get(idx as usize)
                            .copied()
                            .unwrap_or(false)
                        {
                            Self::sound_source_handle_from_index(idx as usize)
                        } else {
                            tracing::error!(
                                "Script Error: trying to get a sound source that has already been destroyed ({idx})"
                            );
                            0
                        }
                    } else {
                        tracing::error!(
                            "Script Error: invalid sound source ID {idx} (max={})",
                            self.sound_source_count
                        );
                        0
                    }
                }
                GetBuildingScript => Self::script_index_to_handle(
                    stack.pop_i32(),
                    self.script_building_count,
                    "building",
                    SCRIPT_HANDLE_BUILDING_TAG,
                ),
                GetWayScript => Self::script_index_to_handle(
                    stack.pop_i32(),
                    self.script_hiking_path_count,
                    "way",
                    SCRIPT_HANDLE_WAY_TAG,
                ),

                // --- Reverse index lookup (handle → script index) ---
                //
                // There is a separate native per object type, but the
                // All reverse lookups decode the tagged 0-based payload.
                // `GetSoundSourceIndex` also gates
                // on the sound subsystem being ready and on the
                // per-slot "still alive" flag — split out below.
                GetActorIndex | GetDoorIndex | GetPatchIndex | GetLocationIndex
                | GetBuildingIndex | GetWayIndex => {
                    let handle = stack.pop_i32();
                    let idx = match f {
                        GetActorIndex => Self::actor_handle_index(handle),
                        GetDoorIndex => Self::door_index(handle),
                        GetPatchIndex => Self::patch_index(handle),
                        GetLocationIndex => Self::location_index(handle),
                        GetBuildingIndex => Self::building_index(handle),
                        GetWayIndex => Self::way_index(handle),
                        _ => unreachable!(),
                    };
                    idx.map_or(-1, |i| i as i32)
                }
                GetSoundSourceIndex => {
                    //   - start with idx = -1
                    //   - only proceed if the sound subsystem is ready
                    //   - look up the handle against the live
                    //     sound-source array; an unknown source logs
                    //     and still returns -1.
                    let handle = stack.pop_i32();
                    let Some(idx) = Self::sound_source_index(handle) else {
                        return -1;
                    };
                    // Proxy for "sound is ready": no slots ⇒ no sound
                    // subsystem in this build/level.
                    if self.sound_source_count == 0 {
                        return -1;
                    }
                    if idx >= self.sound_source_count
                        || !self.sound_source_alive.get(idx).copied().unwrap_or(false)
                    {
                        tracing::error!(
                            "ScriptError: unknown sound source in GetSoundSourceIndex (handle {handle})"
                        );
                        return -1;
                    }
                    idx as i32
                }

                // --- camera / UI ---
                ScrollCameraTo => {
                    let loc = stack.pop_i32();
                    if Self::check_camera_location(loc, "ScrollCameraTo") {
                        self.commands.push(EngineCommand::ScrollCameraTo {
                            location_handle: loc,
                            speed: 2.0,
                        });
                    }
                    0
                }
                ScrollCameraSlowlyTo => {
                    let speed = f32::from_bits(stack.pop_i32() as u32);
                    let loc = stack.pop_i32();
                    if Self::check_camera_location(loc, "ScrollCameraSlowlyTo") {
                        self.commands.push(EngineCommand::ScrollCameraTo {
                            location_handle: loc,
                            speed,
                        });
                    }
                    0
                }
                JumpCameraTo => {
                    let loc = stack.pop_i32();
                    if Self::check_camera_location(loc, "JumpCameraTo") {
                        self.commands.push(EngineCommand::JumpCameraTo {
                            location_handle: loc,
                        });
                    }
                    0
                }
                SetZoomLevel => {
                    let zoom_bits = stack.pop_i32();
                    let zoom = f32::from_bits(zoom_bits as u32);
                    if zoom != 0.5 && zoom != 1.0 && zoom != 2.0 {
                        tracing::warn!("Script Error: SetZoomLevel with invalid zoom {zoom}");
                    } else {
                        self.commands.push(EngineCommand::SetZoomLevel { zoom });
                    }
                    0
                }
                StartDialog => {
                    let dialog_id = stack.pop_i32();
                    self.commands.push(EngineCommand::StartDialog { dialog_id });
                    0
                }
                DisplayMap => {
                    let show = stack.pop_i32();
                    self.commands
                        .push(EngineCommand::DisplayMap { show: show != 0 });
                    0
                }
                DisplayConsole => {
                    self.commands.push(EngineCommand::DisplayConsole);
                    0
                }
                CustomizeMinimapDisplay => {
                    let dot_type = stack.pop_i32();
                    let actor_handle = stack.pop_i32();
                    if actor_handle == 0 {
                        tracing::warn!(
                            "Script Error: CustomizeMinimapDisplay called with NULL actor"
                        );
                    } else {
                        self.commands.push(EngineCommand::CustomizeMinimapDisplay {
                            actor_handle,
                            dot_type,
                        });
                    }
                    0
                }
                DefineFlatTrajectoryZone => {
                    let apex_height = stack.pop_i32();
                    let location_handle = stack.pop_i32();
                    self.commands.push(EngineCommand::DefineFlatTrajectoryZone {
                        location_handle,
                        apex_height,
                    });
                    0
                }
                AddShortBriefing => {
                    let primary = stack.pop_i32();
                    let id = stack.pop_i32();
                    // Queue an engine command so the engine handles
                    // updating `short_briefings` on the Campaign.
                    self.commands.push(EngineCommand::AddShortBriefing {
                        id,
                        primary: primary != 0,
                    });
                    0
                }
                DoneShortBriefing => {
                    let id = stack.pop_i32();
                    self.commands.push(EngineCommand::DoneShortBriefing { id });
                    0
                }
                ChooseVictoryDefeatText => {
                    let id = stack.pop_i32();
                    self.commands
                        .push(EngineCommand::ChooseVictoryDefeatText { id });
                    0
                }
                DisplayPopupText => {
                    let text_id = stack.pop_i32();
                    self.commands
                        .push(EngineCommand::DisplayPopupText { text_id });
                    0
                }
                DisplaySherwoodReport => {
                    self.commands.push(EngineCommand::DisplaySherwoodReport);
                    0
                }
                FadeToBlack => {
                    let speed = stack.pop_i32();
                    self.commands.push(EngineCommand::FadeToBlack { speed });
                    0
                }
                SetOutlineDisplay => {
                    let val = stack.pop_i32();
                    let display = val != 0;
                    if self.outline_display != display {
                        self.outline_display = display;
                        self.commands
                            .push(EngineCommand::SetOutlineDisplay { display });
                    }
                    0
                }
                GetOutlineDisplay => {
                    if self.outline_display {
                        1
                    } else {
                        0
                    }
                }
                SetViewRadius => {
                    let radius = stack.pop_i32();
                    self.commands.push(EngineCommand::SetViewRadius { radius });
                    0
                }
                PlayTrapJingle => {
                    self.commands.push(EngineCommand::PlayJingle(
                        crate::sound::Jingle::TrapTriggered,
                    ));
                    0
                }

                // ═══════════════════════════════════════════════════════
                // Record / sequence — each creates a SequenceElement and
                // appends it to the current RecordingSession.
                // ═══════════════════════════════════════════════════════

                // --- Camera ---
                RecordScrollCameraTo => {
                    let loc = stack.pop_i32();
                    if !self.is_script_point(loc) {
                        tracing::warn!(
                            "Script Error: RecordScrollCameraTo wrong kind of location (handle {loc})"
                        );
                        return 0;
                    }
                    let (x, y) = match self.resolve_location_pos(loc) {
                        Some(p) => p,
                        None => {
                            tracing::warn!(
                                "Script Error: RecordScrollCameraTo unresolved location {loc}"
                            );
                            return 0;
                        }
                    };
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::CameraGoto, None);
                    elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                    // The CameraSpeed field must be an Integer (the
                    // engine reader in tick.rs only accepts Integer);
                    // a literal 0 means "default speed".
                    elem.set_property(Field::CameraSpeed, FieldValue::Integer(0));
                    self.record_element(elem)
                }
                RecordJumpCameraTo => {
                    let loc = stack.pop_i32();
                    if !self.is_script_point(loc) {
                        tracing::warn!(
                            "Script Error: RecordJumpCameraTo wrong kind of location (handle {loc})"
                        );
                        return 0;
                    }
                    let (x, y) = match self.resolve_location_pos(loc) {
                        Some(p) => p,
                        None => {
                            tracing::warn!(
                                "Script Error: RecordJumpCameraTo unresolved location {loc}"
                            );
                            return 0;
                        }
                    };
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::CameraJumpTo, None);
                    elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                    self.record_element(elem)
                }
                RecordSetZoom => {
                    let zoom = stack.pop_i32();
                    let zoom_f = f32::from_bits(zoom as u32);
                    // Reject anything but 0.5 / 1.0 / 2.0.
                    if zoom_f != 0.5 && zoom_f != 1.0 && zoom_f != 2.0 {
                        tracing::warn!(
                            "Script Error: Wanted zoom level is incorrect in RecordSetZoom (got {zoom_f})"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::ZoomLevel, None);
                    elem.set_property(Field::CameraZoomLevel, FieldValue::Float(zoom_f));
                    self.record_element(elem)
                }
                RecordDisplayMap => {
                    let show = stack.pop_i32();
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::DisplayMap, None);
                    elem.set_property(Field::MapDisplay, FieldValue::Bool(show != 0));
                    self.record_element(elem)
                }
                RecordMoveCameraTo => {
                    let speed = stack.pop_i32();
                    let loc = stack.pop_i32();
                    if !self.is_script_point(loc) {
                        tracing::warn!(
                            "Script Error: RecordMoveCameraTo wrong kind of location (handle {loc})"
                        );
                        return 0;
                    }
                    let (x, y) = match self.resolve_location_pos(loc) {
                        Some(p) => p,
                        None => {
                            tracing::warn!(
                                "Script Error: RecordMoveCameraTo unresolved location {loc}"
                            );
                            return 0;
                        }
                    };
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::CameraGoto, None);
                    elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                    // CameraSpeed must be an Integer (the engine
                    // reader in tick.rs only unwraps Integer).
                    elem.set_property(Field::CameraSpeed, FieldValue::Integer(speed as u32));
                    self.record_element(elem)
                }
                RecordLockCameraOn => {
                    let actor = stack.pop_i32();
                    // Reject non-actor handles with a warning + return 0.
                    if !self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                        tracing::warn!(
                            "Script Error: RecordLockCameraOn on illegal actor handle {actor}"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    // Interaction element with actor as antagonist, no owner.
                    let elem = SequenceElement::new_interaction(
                        level,
                        Command::LockCameraOn,
                        None,
                        self.actor_id(actor),
                    );
                    self.record_element(elem)
                }
                RecordClearCameraLock => {
                    let level = self.recording_level();
                    let elem = SequenceElement::new(level, Command::LockCameraStop, None);
                    self.record_element(elem)
                }

                // --- Dialog / UI ---
                RecordPlayDialog => {
                    let dialog_id = stack.pop_i32();
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::PlayDialog, None);
                    elem.set_property(Field::DialogId, FieldValue::Integer(dialog_id as u32));
                    self.record_element(elem)
                }
                RecordDisplayPopupText => {
                    let text_id = stack.pop_i32();
                    let level = self.recording_level();
                    let mut elem =
                        SequenceElement::new_generic(level, Command::DisplayPopupText, None);
                    elem.set_property(Field::PopupTextId, FieldValue::Integer(text_id as u32));
                    self.record_element(elem)
                }

                // --- Action / character availability ---
                RecordActionAvailable => {
                    let available = stack.pop_i32();
                    let action_id = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Reject non-actor handles before recording.
                    if !self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                        tracing::warn!(
                            "Script Error: RecordActionAvailable on illegal actor handle {actor}"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::ActionAvailable,
                        self.actor_id(actor),
                    );
                    elem.set_property(Field::ActionId, FieldValue::Integer(action_id as u32));
                    elem.set_property(Field::ActionAvailable, FieldValue::Bool(available != 0));
                    self.record_element(elem)
                }
                RecordCharacterAvailable => {
                    let available = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Reject non-actor handles.
                    if !self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                        tracing::warn!(
                            "Script Error: RecordCharacterAvailable on illegal actor handle {actor}"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::CharacterAvailable,
                        self.actor_id(actor),
                    );
                    elem.set_property(Field::CharacterAvailable, FieldValue::Bool(available != 0));
                    self.record_element(elem)
                }

                // --- Messages ---
                RecordSendMessage => {
                    let msg = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Reject non-actor, non-null handles with a
                    // warning and no record.
                    if actor != 0 && !self.is_actor_handle(actor) {
                        tracing::error!(
                            "Script Error : trying to send a message to non actor object."
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::SendMessage,
                        self.actor_id(actor),
                    );
                    elem.set_property(Field::Message, FieldValue::Integer(msg as u32));
                    elem.set_property(Field::MessageArgument, FieldValue::Integer(0));
                    elem.set_property(Field::MessageExtendedArgument, FieldValue::Integer(0));
                    self.record_element(elem)
                }
                RecordSendMessageWithArguments => {
                    let arg2 = stack.pop_i32();
                    let arg1 = stack.pop_i32();
                    let msg = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Same IsActor guard as RecordSendMessage.
                    if actor != 0 && !self.is_actor_handle(actor) {
                        tracing::error!(
                            "Script Error : trying to send a message to non actor object."
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::SendMessage,
                        self.actor_id(actor),
                    );
                    elem.set_property(Field::Message, FieldValue::Integer(msg as u32));
                    elem.set_property(Field::MessageArgument, FieldValue::Integer(arg1 as u32));
                    elem.set_property(
                        Field::MessageExtendedArgument,
                        FieldValue::Integer(arg2 as u32),
                    );
                    self.record_element(elem)
                }

                // --- Movement ---
                RecordMove => {
                    let style = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Reject null actor, non-actor handle, null /
                    // non-Point location, and any style outside 0..=3.
                    if !self.is_actor_handle(actor) {
                        tracing::error!("Script Error in RecordMove: invalid actor handle {actor}");
                        return 0;
                    }
                    let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                        tracing::error!(
                            "Script Error in RecordMove: illegal location handle {loc} (null or not a Point)"
                        );
                        return 0;
                    };
                    if !(0..=3).contains(&style) {
                        tracing::error!(
                            "Script Error in RecordMove: illegal movement style {style}"
                        );
                        return 0;
                    }
                    let dest_layer_sector = self.resolve_location_layer_sector(loc);
                    // Chained Record* moves for the same actor start
                    // from the previous target, not the actor's live
                    // position.
                    let origin =
                        self.update_motion_start_position(actor, (dx, dy), dest_layer_sector);
                    let action = Self::movement_style(style);
                    let pre_record_size = self
                        .recording
                        .as_ref()
                        .map(|r| r.current_size())
                        .unwrap_or(0);
                    // Expand the move into the sequence.
                    let (goal_layer, goal_sector) = dest_layer_sector.unwrap_or((0, 0));
                    let (sx, sy, src_layer, src_sector) =
                        origin.unwrap_or((dx, dy, goal_layer, goal_sector));
                    self.append_move_to_sequence(
                        actor,
                        action,
                        (sx, sy),
                        src_sector,
                        src_layer,
                        (dx, dy),
                        goal_sector,
                        goal_layer,
                        None,
                        0.0,
                        MoveFlags::CALLED_BY_SCRIPT,
                        1.0,
                    );
                    // NONINTERRUPTABLE walks bump every just-added
                    // element to Script priority.
                    if matches!(style, 2 | 3)
                        && let Some(rec) = self.recording.as_mut()
                    {
                        rec.bump_priority_from(
                            pre_record_size,
                            crate::sequence::SequencePriority::Script,
                        );
                    }
                    1
                }
                RecordMoveNear => {
                    let tolerance = stack.pop_i32();
                    let style = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Same validation as RecordMove (we additionally
                    // explicitly reject null actor handles, which the
                    // legacy implementation would dereference).
                    if !self.is_actor_handle(actor) {
                        tracing::error!(
                            "Script Error in RecordMoveNear: invalid actor handle {actor}"
                        );
                        return 0;
                    }
                    let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                        tracing::error!(
                            "Script Error in RecordMoveNear: illegal location handle {loc} (null or not a Point)"
                        );
                        return 0;
                    };
                    if !(0..=3).contains(&style) {
                        tracing::error!(
                            "Script Error in RecordMoveNear: illegal movement style {style}"
                        );
                        return 0;
                    }
                    let dest_layer_sector = self.resolve_location_layer_sector(loc);
                    let origin =
                        self.update_motion_start_position(actor, (dx, dy), dest_layer_sector);
                    let action = Self::movement_style(style);
                    let pre_record_size = self
                        .recording
                        .as_ref()
                        .map(|r| r.current_size())
                        .unwrap_or(0);
                    let (goal_layer, goal_sector) = dest_layer_sector.unwrap_or((0, 0));
                    let (sx, sy, src_layer, src_sector) =
                        origin.unwrap_or((dx, dy, goal_layer, goal_sector));
                    self.append_move_to_sequence(
                        actor,
                        action,
                        (sx, sy),
                        src_sector,
                        src_layer,
                        (dx, dy),
                        goal_sector,
                        goal_layer,
                        None,
                        tolerance as f32,
                        MoveFlags::CALLED_BY_SCRIPT,
                        1.0,
                    );
                    // NONINTERRUPTABLE near-walks bump every
                    // just-added element to Preference priority (one
                    // rung weaker than RecordMove's Script).
                    if matches!(style, 2 | 3)
                        && let Some(rec) = self.recording.as_mut()
                    {
                        rec.bump_priority_from(
                            pre_record_size,
                            crate::sequence::SequencePriority::Preference,
                        );
                    }
                    1
                }
                RecordMoveIntoBuilding => {
                    // Validate the location is a Point, find the
                    // nearest door within 300px of it, then synthesise
                    // a point at the door's
                    // (PointIn, LayerIn, SectorIn) and tail-call RecordMove.
                    let style = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if !self.is_actor_handle(actor) {
                        tracing::error!(
                            "Script Error in RecordMoveIntoBuilding: invalid actor handle {actor}"
                        );
                        return 0;
                    }
                    let Some((lx, ly)) = self.resolve_location_pos(loc) else {
                        tracing::error!(
                            "Script Error in RecordMoveIntoBuilding: illegal location handle {loc} (null or not a Point)"
                        );
                        return 0;
                    };
                    if !(0..=3).contains(&style) {
                        tracing::error!(
                            "Script Error in RecordMoveIntoBuilding: illegal movement style {style}"
                        );
                        return 0;
                    }

                    // Find nearest door whose mid-point is within
                    // 300px of the target; if none, return 0.
                    let max_sq_dist = 300.0_f32 * 300.0;
                    let mut best: Option<(f32, f32, f32, u16, u16)> = None;
                    for door in &self.doors {
                        let ddx = door.point_mid.x - lx;
                        let ddy = door.point_mid.y - ly;
                        let sq = ddx * ddx + ddy * ddy;
                        if sq < max_sq_dist && (best.is_none() || sq < best.unwrap().0) {
                            best = Some((
                                sq,
                                door.point_in.x,
                                door.point_in.y,
                                door.layer_in,
                                door.sector_in.0 as u16,
                            ));
                        }
                    }
                    let Some((_, ix, iy, door_layer, door_sector)) = best else {
                        tracing::error!(
                            "Script Error in RecordMoveIntoBuilding: no door within 300px of ({lx}, {ly})"
                        );
                        return 0;
                    };

                    // The original tail-calls RecordMove with a
                    // synthesised point at the door's interior.  The
                    // tail-call also runs `update_motion_start_position`
                    // on the actor, so do that here too — chained
                    // Record* see the door's interior as the new
                    // motion target.
                    let origin = self.update_motion_start_position(
                        actor,
                        (ix, iy),
                        Some((door_layer, door_sector)),
                    );
                    let action = Self::movement_style(style);
                    let pre_record_size = self
                        .recording
                        .as_ref()
                        .map(|r| r.current_size())
                        .unwrap_or(0);
                    // Drive the inner RecordMove tail call's
                    // `append_move_to_sequence`.  The goal point uses
                    // the door's interior (layer, sector).
                    let (sx, sy, src_layer, src_sector) =
                        origin.unwrap_or((ix, iy, door_layer, door_sector));
                    self.append_move_to_sequence(
                        actor,
                        action,
                        (sx, sy),
                        src_sector,
                        src_layer,
                        (ix, iy),
                        door_sector,
                        door_layer,
                        None,
                        0.0,
                        MoveFlags::CALLED_BY_SCRIPT,
                        1.0,
                    );
                    // Apply the same NONINTERRUPTABLE bump the inner
                    // RecordMove would apply.
                    if matches!(style, 2 | 3)
                        && let Some(rec) = self.recording.as_mut()
                    {
                        rec.bump_priority_from(
                            pre_record_size,
                            crate::sequence::SequencePriority::Script,
                        );
                    }
                    1
                }
                RecordEnterGame => {
                    // Immediately teleports the actor to a point just
                    // outside the map edge (opposite of its facing
                    // direction relative to the destination), then
                    // records a single movement element from the
                    // outside spawn point to the destination.
                    let style = stack.pop_i32();
                    let direction = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();

                    if !Self::validate_style(style, "RecordEnterGame") {
                        return 0;
                    }
                    if !self.actor_exists(actor) {
                        tracing::warn!("RecordEnterGame: invalid actor handle {actor}");
                        return 0;
                    }
                    let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                        tracing::warn!(
                            "RecordEnterGame: illegal location handle {loc} (not a Point)"
                        );
                        return 0;
                    };
                    // Read layer + sector from the destination point
                    // and apply them to the teleported actor.  Static
                    // script locations carry that data; computed ones
                    // do not, so we leave layer/sector untouched in
                    // that case.
                    let dest_layer_sector = self.resolve_location_layer_sector(loc);

                    // `direction == -1` means "use actor's current direction".
                    let actor_dir = self
                        .get_entity(actor)
                        .map(|e| e.element_data().direction())
                        .unwrap_or(0);
                    let effective_dir = if direction == -1 {
                        actor_dir
                    } else {
                        direction as i16
                    };

                    let (_border, (ox, oy)) = self.compute_border_point((dx, dy), effective_dir);

                    // If the actor is already in `moving_actors`,
                    // just refresh the cached destination and skip
                    // the teleport / outside-spawn block.  Only the
                    // first EnterGame call in a recording session
                    // teleports the actor outside the map; subsequent
                    // calls for the same actor only update the
                    // bookkeeping target.
                    let already_moving = self
                        .recording
                        .as_ref()
                        .is_some_and(|r| r.moving_actors.contains_key(&actor));

                    if !already_moving {
                        // Immediate teleport: write position + layer
                        // + sector on the actor.  The native can only
                        // touch the `ElementData` copy — the 3D spawn
                        // elevation at `(ox, oy)` comes from the
                        // destination's projection-area top plane and
                        // needs fast-grid access, so that composition
                        // happens inside the queued
                        // `SetActorLocation` command below.  Here we
                        // only write the 2D spawn position so the VM
                        // observes the actor as outside the map on
                        // the next script read.  Also handles the
                        // `in_honolulu` wake-up case.
                        if let Some(entity) = self.get_entity_mut(actor) {
                            let ed = entity.element_data_mut();
                            if ed.in_honolulu {
                                ed.active = true;
                                ed.in_honolulu = false;
                            }
                            ed.set_position_map(crate::coordinates::MapPoint { x: ox, y: oy });
                            if let Some((layer, sector_num)) = dest_layer_sector {
                                ed.set_layer(layer);
                                ed.set_sector(crate::position_interface::SectorHandle::new(
                                    sector_num,
                                ));
                            }
                            ed.update_grid_cell();
                        } else {
                            tracing::warn!("RecordEnterGame: invalid actor handle {actor}");
                            return 0;
                        }
                        self.commands.push(EngineCommand::SetActorLocation {
                            actor_handle: actor,
                            x: ox,
                            y: oy,
                            dest_layer_sector,
                            // The engine handler probes the
                            // destination sector's top plane at
                            // `(dx, dy)` and stamps
                            // `(ox, oy + elev, elev)` as the 3D
                            // spawn — so the actor walks in at the
                            // same altitude as the destination's
                            // ground slope.
                            spawn_elevation_probe: Some((dx, dy)),
                        });
                    }

                    // Always refresh the cached destination on both
                    // the insert and update paths.
                    if let Some(rec) = self.recording.as_mut() {
                        let (layer, sector) = dest_layer_sector.unwrap_or((0, 0));
                        rec.moving_actors.insert(
                            actor,
                            crate::sequence::RecordingMotionTarget {
                                x: dx,
                                y: dy,
                                layer,
                                sector,
                            },
                        );
                    }

                    let level = self.recording_level();
                    let action = Self::movement_style(style);
                    let mut elem = SequenceElement::new_movement(
                        level,
                        Command::Move,
                        self.actor_id(actor),
                        action,
                    );
                    if let crate::sequence::SequenceElementData::Movement {
                        destination,
                        flags,
                        ..
                    } = &mut elem.data
                    {
                        destination.x = dx;
                        destination.y = dy;
                        *flags |= MoveFlags::CALLED_BY_SCRIPT | MoveFlags::MAP;
                        // No direction is passed — the enter-game walk
                        // computes it from the movement vector when
                        // dispatched.
                    }
                    let _ = self.record_element(elem);
                    // Returns `false` (0) unconditionally; scripts
                    // don't observe it.
                    0
                }
                RecordLeaveGame => {
                    // Records two sequential movement elements: first
                    // a normal walk to the script point, then a
                    // straight-line walk from that point to a spot
                    // just outside the map edge (in the direction the
                    // actor is heading).
                    let style = stack.pop_i32();
                    let direction = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();

                    if !Self::validate_style(style, "RecordLeaveGame") {
                        return 0;
                    }
                    if !self.actor_exists(actor) {
                        tracing::warn!("RecordLeaveGame: invalid actor handle {actor}");
                        return 0;
                    }
                    let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                        tracing::warn!(
                            "RecordLeaveGame: illegal location handle {loc} (not a Point)"
                        );
                        return 0;
                    };

                    let actor_dir = self
                        .get_entity(actor)
                        .map(|e| e.element_data().direction())
                        .unwrap_or(0);
                    let effective_dir = if direction == -1 {
                        actor_dir
                    } else {
                        direction as i16
                    };

                    // `compute_border_point` wants the *opposite* of
                    // the travel direction, so the exit edge is the
                    // one the actor is walking towards — pass
                    // `(direction + 8) & 15`.
                    let opposite_dir = (effective_dir + 8) & 15;
                    let (_border, (ox, oy)) = self.compute_border_point((dx, dy), opposite_dir);

                    let action = Self::movement_style(style);

                    // When `RecordEnterGame` already recorded a
                    // destination for this actor in this session,
                    // that destination becomes the *origin* of the
                    // leave walk (not the actor's live position,
                    // which the EnterGame teleport pinned to the
                    // *outside* spawn point).
                    let dest_layer_sector = self.resolve_location_layer_sector(loc);
                    let origin =
                        self.update_motion_start_position(actor, (dx, dy), dest_layer_sector);

                    // Step 1: append_move_to_sequence(origin → script
                    // point).  Cross-sector traversals expand into
                    // ASSERT_POSITION + per-gate sub-elements;
                    // same-sector goal collapses to a single MOVE.
                    let (goal_layer, goal_sector) = dest_layer_sector.unwrap_or((0, 0));
                    let (sx, sy, src_layer, src_sector) =
                        origin.unwrap_or((dx, dy, goal_layer, goal_sector));
                    self.append_move_to_sequence(
                        actor,
                        action,
                        (sx, sy),
                        src_sector,
                        src_layer,
                        (dx, dy),
                        goal_sector,
                        goal_layer,
                        None,
                        0.0,
                        MoveFlags::CALLED_BY_SCRIPT,
                        1.0,
                    );

                    // Insert the two moves at adjacent sequence
                    // levels so they execute sequentially rather
                    // than concurrently.
                    if let Some(rec) = self.recording.as_mut() {
                        rec.advance_level();
                    }

                    // Step 2: straight MOVE to the off-map exit
                    // point (with the MAP flag).  This is a single
                    // element, not a gate-expanded path.
                    let level2 = self.recording_level();
                    let mut elem2 = SequenceElement::new_movement(
                        level2,
                        Command::Move,
                        self.actor_id(actor),
                        action,
                    );
                    if let crate::sequence::SequenceElementData::Movement {
                        destination,
                        flags,
                        ..
                    } = &mut elem2.data
                    {
                        destination.x = ox;
                        destination.y = oy;
                        *flags |= MoveFlags::CALLED_BY_SCRIPT | MoveFlags::MAP;
                        // No direction is passed — the off-map leave
                        // walk derives facing from the move vector
                        // when dispatched.
                    }
                    let _ = self.record_element(elem2);
                    // Returns `true` (1) unconditionally; scripts
                    // don't observe the value.
                    1
                }

                // --- Turn ---
                RecordTurnTo => {
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Two hard preconditions: target must be an actor
                    // and the location must be a point.  Reject
                    // either miss with `false` rather than stashing
                    // a raw integer under `CameraPoint`.
                    if !self.is_actor_handle(actor) {
                        tracing::warn!("RecordTurnTo: illegal actor handle {actor}");
                        return 0;
                    }
                    let Some((x, y)) = self.resolve_location_pos(loc) else {
                        tracing::warn!("RecordTurnTo: illegal location handle {loc}");
                        return 0;
                    };
                    let level = self.recording_level();
                    let mut elem =
                        SequenceElement::new_generic(level, Command::Turn, self.actor_id(actor));
                    elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                    self.record_element(elem)
                }

                // --- Animation ---
                RecordPlayAnim => {
                    let anim = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Reject null handles and anything that is
                    // neither an actor nor an FX target.
                    if actor == 0 || !self.is_actor_or_fx_target(actor) {
                        tracing::warn!(
                            "RecordPlayAnim: illegal actor handle {actor} (not actor/fx-target)"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::PlayAnim,
                        self.actor_id(actor),
                    );
                    elem.set_property(
                        Field::AnimationId,
                        FieldValue::Animation(anim_ordinal_to_order_type(anim, "RecordPlayAnim")),
                    );
                    self.record_element(elem)
                }
                RecordPlayAnimLoop => {
                    let anim = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Uses the full ActorExists validator (no
                    // FX-target branch like its siblings) before
                    // constructing the element.
                    if actor == 0 || !self.actor_exists(actor) {
                        tracing::warn!("RecordPlayAnimLoop: invalid actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::PlayAnimLoop,
                        self.actor_id(actor),
                    );
                    elem.set_property(
                        Field::AnimationId,
                        FieldValue::Animation(anim_ordinal_to_order_type(
                            anim,
                            "RecordPlayAnimLoop",
                        )),
                    );
                    self.record_element(elem)
                }
                RecordPlayAnimFreeze => {
                    let anim = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Omits the null-handle check but still requires
                    // actor-or-FX-target.
                    if !self.is_actor_or_fx_target(actor) {
                        tracing::warn!(
                            "RecordPlayAnimFreeze: illegal actor handle {actor} (not actor/fx-target)"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::PlayAnimFreeze,
                        self.actor_id(actor),
                    );
                    elem.set_property(
                        Field::AnimationId,
                        FieldValue::Animation(anim_ordinal_to_order_type(
                            anim,
                            "RecordPlayAnimFreeze",
                        )),
                    );
                    self.record_element(elem)
                }
                RecordReplaceAnim => {
                    let new_anim = stack.pop_i32();
                    let old_anim = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Gates on `ActorExists && IsActor`.
                    if !self.is_actor_handle(actor) {
                        tracing::warn!("RecordReplaceAnim: illegal actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::ReplaceAnim,
                        self.actor_id(actor),
                    );
                    elem.set_property(Field::OldAnimation, FieldValue::Integer(old_anim as u32));
                    elem.set_property(Field::NewAnimation, FieldValue::Integer(new_anim as u32));
                    self.record_element(elem)
                }
                RecordRestoreAnim => {
                    let old_anim = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Gates on `IsActor` after the sequence-level check.
                    if !self.is_actor_handle(actor) {
                        tracing::warn!("RecordRestoreAnim: illegal actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(
                        level,
                        Command::RestoreAnim,
                        self.actor_id(actor),
                    );
                    elem.set_property(Field::OldAnimation, FieldValue::Integer(old_anim as u32));
                    self.record_element(elem)
                }
                ResetAnim => {
                    // NOT a Record function — directly resets the actor's
                    // sprite to frame 0 of its current animation row.
                    // Rejects !ActorExists || !IsFX with a warning +
                    // false, else `reset_sprite_frame()` and true.
                    let actor = stack.pop_i32();
                    let is_fx = self.get_entity(actor).is_some_and(|e| e.is_fx());
                    if !self.actor_exists(actor) || !is_fx {
                        tracing::error!(
                            "Script error (ResetAnim): invalid animation handle {actor}"
                        );
                        0
                    } else {
                        self.deferred_commands
                            .push(DeferredCommand::ResetSpriteFrame { actor });
                        1
                    }
                }

                // --- Speech ---
                RecordSpeak => {
                    let speak_id = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Validates `IsHuman` and bound-checks
                    // `id < NUMBER_OF_REMARKS` before constructing the
                    // element.
                    if !self.get_entity(actor).is_some_and(|e| e.is_human()) {
                        tracing::warn!("RecordSpeak: illegal actor {actor} (not human)");
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem =
                        SequenceElement::new_generic(level, Command::Speak, self.actor_id(actor));
                    elem.set_property(Field::SpeakId, FieldValue::Integer(speak_id as u32));
                    // SpeakVariant = 0; SpeakFlags = SPEECH_SCRIPT |
                    // SPEECH_ALWAYS.  The ALWAYS bit is load-bearing
                    // — the speech pipeline uses it to bypass the
                    // forbidden-remark and chorus filters (see ai.rs
                    // / melee.rs consumers).
                    elem.set_property(Field::SpeakVariant, FieldValue::Integer(0));
                    const SPEECH_SCRIPT: u32 = 0x0004;
                    const SPEECH_ALWAYS: u32 = 0x0008;
                    elem.set_property(
                        Field::SpeakFlags,
                        FieldValue::Integer(SPEECH_SCRIPT | SPEECH_ALWAYS),
                    );
                    self.record_element(elem)
                }
                RecordSpeakPC => {
                    let variant = stack.pop_i32();
                    let speak_id = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Gates on `IsPC()`.
                    if !self.get_entity(actor).is_some_and(|e| e.is_pc()) {
                        tracing::warn!("RecordSpeakPC: illegal actor {actor} (not PC)");
                        return 0;
                    }
                    let level = self.recording_level();
                    let mut elem =
                        SequenceElement::new_generic(level, Command::Speak, self.actor_id(actor));
                    elem.set_property(Field::SpeakId, FieldValue::Integer(speak_id as u32));
                    elem.set_property(Field::SpeakVariant, FieldValue::Integer(variant as u32));
                    self.record_element(elem)
                }

                // --- AI / user locks ---
                RecordLockAI => {
                    let actor = stack.pop_i32();
                    // Gates on `ActorExists && IsActor`.
                    if !self.is_actor_handle(actor) {
                        tracing::warn!("RecordLockAI: illegal actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let elem = SequenceElement::new(level, Command::LockAi, self.actor_id(actor));
                    self.record_element(elem)
                }
                RecordUnlockAI => {
                    let actor = stack.pop_i32();
                    // Gates on `ActorExists && IsActor`.
                    if !self.is_actor_handle(actor) {
                        tracing::warn!("RecordUnlockAI: illegal actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let elem = SequenceElement::new(level, Command::UnlockAi, self.actor_id(actor));
                    self.record_element(elem)
                }
                RecordLockUser => {
                    let level = self.recording_level();
                    let elem = SequenceElement::new(level, Command::LockUser, None);
                    self.record_element(elem)
                }
                RecordUnLockUser => {
                    let level = self.recording_level();
                    let elem = SequenceElement::new(level, Command::UnlockUser, None);
                    self.record_element(elem)
                }
                RecordFreezeAll => {
                    let freeze = stack.pop_i32();
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::FreezeAll, None);
                    elem.set_property(Field::Freeze, FieldValue::Bool(freeze != 0));
                    self.record_element(elem)
                }

                // --- Timer ---
                RecordTimer => {
                    let frames = stack.pop_i32();
                    let level = self.recording_level();
                    let mut elem = SequenceElement::new_generic(level, Command::Timer, None);
                    elem.set_property(Field::Timer, FieldValue::Integer(frames as u32));
                    self.record_element(elem)
                }

                // --- Seeking ---
                RecordSeekActor => {
                    let distance = stack.pop_i32();
                    let style = stack.pop_i32();
                    let target = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let level = self.recording_level();
                    let action = Self::seek_style(style);
                    let mut elem = SequenceElement::new_movement(
                        level,
                        Command::Seek,
                        self.actor_id(actor),
                        action,
                    );
                    if let crate::sequence::SequenceElementData::Movement {
                        element,
                        tolerance,
                        flags,
                        ..
                    } = &mut elem.data
                    {
                        *element = self.actor_id(target);
                        *tolerance = f32::from_bits(distance as u32);
                        *flags |= MoveFlags::SEEK;
                    }
                    self.record_element(elem)
                }
                RecordSeekActorMessage => {
                    let msg_id = stack.pop_i32();
                    let msg_actor = stack.pop_i32();
                    let distance = stack.pop_i32();
                    let style = stack.pop_i32();
                    let target = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Rejects non-actor message-target handles.
                    if msg_actor != 0 && !self.is_actor_handle(msg_actor) {
                        tracing::warn!(
                            "RecordSeekActorMessage: illegal msg_actor handle {msg_actor}"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let action = Self::seek_message_style(style);
                    let mut seek_elem = SequenceElement::new_movement(
                        level,
                        Command::Seek,
                        self.actor_id(actor),
                        action,
                    );
                    // Builds a single-element sub-sequence with a
                    // SendMessage command at sub-level 1 and stashes
                    // it on the seek element as the post-seek
                    // sequence.  The post-seek sub-sequence fires
                    // only on successful seek completion, not when
                    // the seek is interrupted/aborted.
                    let mut post_seek = crate::sequence::Sequence::new();
                    post_seek.append_element(
                        self.build_send_message_element(1, msg_actor, msg_id, 0, 0),
                    );
                    if let crate::sequence::SequenceElementData::Movement {
                        element,
                        tolerance,
                        flags,
                        post_seek_sequence,
                        ..
                    } = &mut seek_elem.data
                    {
                        *element = self.actor_id(target);
                        *tolerance = f32::from_bits(distance as u32);
                        *flags |= MoveFlags::SEEK;
                        *post_seek_sequence = Some(Box::new(post_seek));
                    }
                    self.record_element(seek_elem)
                }
                RecordSeekActorMessageWithArguments => {
                    let arg2 = stack.pop_i32();
                    let arg1 = stack.pop_i32();
                    let msg_id = stack.pop_i32();
                    let msg_actor = stack.pop_i32();
                    let distance = stack.pop_i32();
                    let style = stack.pop_i32();
                    let target = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Rejects non-actor msg handles and `id < 1000`.
                    if msg_actor != 0 && !self.is_actor_handle(msg_actor) {
                        tracing::warn!(
                            "RecordSeekActorMessageWithArguments: illegal msg_actor handle {msg_actor}"
                        );
                        return 0;
                    }
                    if msg_id < 1000 {
                        tracing::warn!(
                            "RecordSeekActorMessageWithArguments: ID for custom event is {msg_id}, must be >= 1000"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let action = Self::seek_message_style(style);
                    let mut seek_elem = SequenceElement::new_movement(
                        level,
                        Command::Seek,
                        self.actor_id(actor),
                        action,
                    );
                    // Same post-seek sub-sequence wiring as
                    // `RecordSeekActorMessage` above.
                    let mut post_seek = crate::sequence::Sequence::new();
                    post_seek.append_element(
                        self.build_send_message_element(1, msg_actor, msg_id, arg1, arg2),
                    );
                    if let crate::sequence::SequenceElementData::Movement {
                        element,
                        tolerance,
                        flags,
                        post_seek_sequence,
                        ..
                    } = &mut seek_elem.data
                    {
                        *element = self.actor_id(target);
                        *tolerance = f32::from_bits(distance as u32);
                        *flags |= MoveFlags::SEEK;
                        *post_seek_sequence = Some(Box::new(post_seek));
                    }
                    self.record_element(seek_elem)
                }
                RecordStopSeek => {
                    // Returns false (no-op).  The seek system handles
                    // stopping internally.
                    let _actor = stack.pop_i32();
                    0
                }

                // --- RecordAction (polymorphic command dispatch) ---
                RecordAction => {
                    let number = stack.pop_i32();
                    let action_id = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Gates the whole dispatch on `ActorExists(actor)`.
                    if !self.actor_exists(actor) {
                        tracing::warn!("RecordAction: invalid actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let owner = self.actor_id(actor);
                    // Antagonist lookup for SHOOT / ENTER_SF /
                    // THRUST_*: `number` is a 0-based index into the
                    // script-element array; we abort with `false`
                    // when out of range. Convert that script index into
                    // an actor handle before resolving it.
                    let resolve_antagonist =
                        |number: i32, entities: &[Option<Entity>]| -> Option<Option<EntityId>> {
                            if number < 0 || (number as usize) >= entities.len() {
                                None
                            } else {
                                // Slot may be None (a null antagonist
                                // is accepted once the bounds check
                                // passes).
                                Some(self.actor_id(Self::actor_handle_from_index(number as usize)))
                            }
                        };
                    // Script command constants.
                    const WAIT: i32 = 0;
                    const TURN: i32 = 1;
                    const AIM: i32 = 2;
                    const AIM_UP: i32 = 3;
                    const SHOOT: i32 = 4;
                    const ENTER_SF: i32 = 5;
                    const LEAVE_SF: i32 = 6;
                    const PARRY: i32 = 7;
                    const THRUST_A: i32 = 8;
                    const THRUST_B: i32 = 9;
                    const THRUST_C: i32 = 10;
                    const THRUST_D: i32 = 11;
                    const THRUST_E: i32 = 12;
                    const THRUST_F: i32 = 13;
                    const THRUST_G: i32 = 14;
                    const THRUST_H: i32 = 15;
                    const THRUST_I: i32 = 16;
                    const LOOK_LEFT: i32 = 17;
                    const LOOK_RIGHT: i32 = 18;
                    const UNEQUIP_BOW: i32 = 19;
                    const CROUCH_DOWN: i32 = 20;

                    let elem = match action_id {
                        WAIT => SequenceElement::new(level, Command::Wait, owner),
                        TURN => {
                            let mut e = SequenceElement::new_generic(level, Command::Turn, owner);
                            e.set_property(
                                Field::Direction,
                                FieldValue::Integer((number % 16) as u32),
                            );
                            e
                        }
                        AIM => SequenceElement::new(level, Command::EquipBow, owner),
                        AIM_UP => SequenceElement::new(level, Command::RaiseBow, owner),
                        SHOOT => {
                            let Some(antagonist) = resolve_antagonist(number, &self.entities)
                            else {
                                tracing::warn!(
                                    "RecordAction SHOOT: illegal antagonist index {number}"
                                );
                                return 0;
                            };
                            SequenceElement::new_interaction(
                                level,
                                Command::ShootBowOnce,
                                owner,
                                antagonist,
                            )
                        }
                        ENTER_SF => {
                            let Some(antagonist) = resolve_antagonist(number, &self.entities)
                            else {
                                tracing::warn!(
                                    "RecordAction ENTER_SF: illegal antagonist index {number}"
                                );
                                return 0;
                            };
                            let mut e = SequenceElement::new_generic(
                                level,
                                Command::EnterSwordfight,
                                owner,
                            );
                            // Store the opponent unconditionally
                            // after the bounds check; a null-slot
                            // antagonist still gets recorded.
                            if let Some(ant) = antagonist {
                                e.set_property(Field::Opponent, FieldValue::Element(ant));
                            }
                            e.set_property(Field::JumplineDestination, FieldValue::Integer(0));
                            e
                        }
                        LEAVE_SF => SequenceElement::new(level, Command::QuitSwordfight, owner),
                        PARRY => SequenceElement::new(level, Command::ParrySword, owner),
                        THRUST_A | THRUST_B | THRUST_C | THRUST_D | THRUST_E | THRUST_F
                        | THRUST_G | THRUST_H | THRUST_I => {
                            let cmd = match action_id {
                                THRUST_A => Command::SwordstrikeThrustA,
                                THRUST_B => Command::SwordstrikeThrustB,
                                THRUST_C => Command::SwordstrikeThrustC,
                                THRUST_D => Command::SwordstrikeThrustD,
                                THRUST_E => Command::SwordstrikeThrustE,
                                THRUST_F => Command::SwordstrikeThrustF,
                                THRUST_G => Command::SwordstrikeThrustG,
                                THRUST_H => Command::SwordstrikeThrustH,
                                _ => Command::SwordstrikeThrustI,
                            };
                            let Some(antagonist) = resolve_antagonist(number, &self.entities)
                            else {
                                tracing::warn!(
                                    "RecordAction THRUST: illegal antagonist index {number}"
                                );
                                return 0;
                            };
                            SequenceElement::new_interaction(level, cmd, owner, antagonist)
                        }
                        LOOK_LEFT => {
                            // Rejects non-soldiers.
                            if !self.get_entity(actor).is_some_and(|e| e.is_soldier()) {
                                tracing::warn!(
                                    "RecordAction LOOK_LEFT: actor {actor} is not a soldier"
                                );
                                return 0;
                            }
                            SequenceElement::new(level, Command::LookLeft, owner)
                        }
                        LOOK_RIGHT => {
                            if !self.get_entity(actor).is_some_and(|e| e.is_soldier()) {
                                tracing::warn!(
                                    "RecordAction LOOK_RIGHT: actor {actor} is not a soldier"
                                );
                                return 0;
                            }
                            SequenceElement::new(level, Command::LookRight, owner)
                        }
                        UNEQUIP_BOW => SequenceElement::new(level, Command::UnequipBow, owner),
                        CROUCH_DOWN => SequenceElement::new(level, Command::CrouchDown, owner),
                        _ => {
                            tracing::warn!("RecordAction: unknown script command ID {action_id}");
                            return 0;
                        }
                    };
                    self.record_element(elem)
                }

                // --- Corpse handling ---
                RecordTakeCorpse => {
                    // Walk the taker to the corpse's position, then
                    // run the TakeCorpse interaction.
                    let style = stack.pop_i32();
                    let corpse = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Gates on the taker being a PC with one of the
                    // carry actions, and the corpse being an actor.
                    if !self.is_pc_carrier(actor) {
                        tracing::warn!(
                            "RecordTakeCorpse: taker {actor} is not a PC with a carry action"
                        );
                        return 0;
                    }
                    if !self.is_actor_handle(corpse) {
                        tracing::warn!("RecordTakeCorpse: corpse {corpse} is not an actor");
                        return 0;
                    }
                    let level = self.recording_level();
                    let action = Self::movement_style(style);

                    // Walk the taker to the corpse position (no-op if the
                    // corpse handle is invalid — the take element alone
                    // still makes the sequence well-formed).
                    if let Some(corpse_entity) = self.get_entity(corpse) {
                        let pos = corpse_entity.element_data().position_map();
                        let corpse_layer = corpse_entity.element_data().layer();
                        let corpse_sector = corpse_entity
                            .element_data()
                            .sector()
                            .map(u16::from)
                            .unwrap_or(0);
                        // Replays `update_motion_start_position` on
                        // the taker so a chained Record* sees the
                        // corpse as the new motion target.
                        let origin = self.update_motion_start_position(
                            actor,
                            (pos.x, pos.y),
                            Some((corpse_layer, corpse_sector)),
                        );
                        let (sx, sy, src_layer, src_sector) =
                            origin.unwrap_or((pos.x, pos.y, corpse_layer, corpse_sector));
                        // Tolerance is the per-animation stand-off
                        // for the carry-transition, matching the
                        // original GetActionDistance call.
                        let Some(animation) =
                            crate::engine::command_action_distance_animation(Command::TakeCorpse)
                        else {
                            tracing::warn!(
                                "RecordTakeCorpse: TakeCorpse has no action-distance animation"
                            );
                            return 0;
                        };
                        let Some(tolerance) = self.actor_action_distance(actor, animation) else {
                            return 0;
                        };
                        self.append_move_to_sequence(
                            actor,
                            action,
                            (sx, sy),
                            src_sector,
                            src_layer,
                            (pos.x, pos.y),
                            corpse_sector,
                            corpse_layer,
                            None,
                            tolerance,
                            MoveFlags::CALLED_BY_SCRIPT,
                            1.0,
                        );
                    }

                    // Take it.
                    let elem = SequenceElement::new_interaction(
                        level,
                        Command::TakeCorpse,
                        self.actor_id(actor),
                        self.actor_id(corpse),
                    );
                    self.record_element(elem)
                }
                RecordLeaveCorpse => {
                    let actor = stack.pop_i32();
                    // Gates on `IsPC && (LittleJohnCarry || FarmerCarry)`.
                    if !self.is_pc_carrier(actor) {
                        tracing::warn!(
                            "RecordLeaveCorpse: actor {actor} is not a PC with a carry action"
                        );
                        return 0;
                    }
                    let level = self.recording_level();
                    let elem =
                        SequenceElement::new(level, Command::DropCorpse, self.actor_id(actor));
                    self.record_element(elem)
                }

                // --- Mobile elements (dead engine code) ---
                // The discriminants stay so shipped SCB bytecode lines
                // up; no shipped level spawns a mobile, so these
                // natives are unreachable in practice.
                RecordStartMobileElement
                | RecordStopMobileElement
                | RecordActivateMobileElement
                | RecordDeactivateMobileElement => {
                    let _ = stack.pop_i32();
                    tracing::error!(
                        "Script Error: Record*MobileElement called but mobiles are not ported"
                    );
                    0
                }

                // --- Misc ---
                RecordUnBlip => {
                    let actor = stack.pop_i32();
                    // Gates on `ActorExists && IsActor`.
                    if !self.is_actor_handle(actor) {
                        tracing::warn!("RecordUnBlip: illegal actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let elem = SequenceElement::new(level, Command::Unblip, self.actor_id(actor));
                    self.record_element(elem)
                }

                // --- entity type checks ---
                // Original: original-code/RHScript.cpp, RHScript::ThisActor
                // returns the callback's pScriptThis verbatim.
                ThisActor => self.script_this,
                // Original: original-code/RHScript.cpp,
                // RHScript::GetNumberOfActorsInEngine returns
                // marrayElementsScript.Size().
                GetNumberOfActorsInEngine => self.entities.len() as i32,
                IsActorAnimation => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    match self.get_entity(handle) {
                        Some(e) if e.is_fx() => 1,
                        _ => 0,
                    }
                }
                IsActorObject => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorObject): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if self.get_entity(handle).unwrap().is_object() {
                        1
                    } else {
                        0
                    }
                }
                IsActorCharacter => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorCharacter): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if self.get_entity(handle).unwrap().is_actor() {
                        1
                    } else {
                        0
                    }
                }
                IsActorPC => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorPC): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if self.get_entity(handle).unwrap().is_pc() {
                        1
                    } else {
                        0
                    }
                }
                IsActorNPC => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorNPC): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if self.get_entity(handle).unwrap().is_npc() {
                        1
                    } else {
                        0
                    }
                }
                IsActorSoldier => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorSoldier): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if self.get_entity(handle).unwrap().is_soldier() {
                        1
                    } else {
                        0
                    }
                }
                IsActorCivilian => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorCivilian): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if self.get_entity(handle).unwrap().is_civilian() {
                        1
                    } else {
                        0
                    }
                }
                IsActorAnimal => {
                    // No animals in this port; shipped scripts never
                    // actually query this (verified across all .scb
                    // files in datadirs/fullgame_linux), but we keep
                    // the native slot so the enum discriminants align
                    // with the shipped SCB indices.  Always returns 0.
                    let _handle = stack.pop_i32();
                    0
                }
                IsActorCart => {
                    // No mobiles in this port; shipped scripts never
                    // observe a true return.  Slot kept for SCB
                    // bytecode alignment (same as `IsActorAnimal`).
                    let _handle = stack.pop_i32();
                    0
                }
                IsActorActive => {
                    let handle = stack.pop_i32();
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorActive): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if self.get_entity(handle).unwrap().is_active() {
                        1
                    } else {
                        0
                    }
                }
                IsActorRider => {
                    let handle = stack.pop_i32();
                    if handle == 0 {
                        return 0;
                    }
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsActorRider): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    let entity = self.get_entity(handle).unwrap();
                    if !entity.is_soldier() {
                        return 0;
                    }
                    if entity.soldier_data().is_some_and(|s| s.rider) {
                        1
                    } else {
                        0
                    }
                }
                IsUnblipped => {
                    let handle = stack.pop_i32();
                    if !self.actor_exists(handle) {
                        tracing::error!(
                            "Script error (IsUnblipped): invalid actor handle {handle:#x}"
                        );
                        return 0;
                    }
                    if !self.get_entity(handle).unwrap().element_data().blipped {
                        1
                    } else {
                        0
                    }
                }

                // --- actor state ---
                GetActorPosture => {
                    // Remaps the internal `Posture` enum to the
                    // script-visible `ID_*` constants.  The two
                    // numeric spaces do NOT coincide — e.g. `Upright`
                    // is internal-1 but `ID_UPRIGHT` is script-0.
                    // Two arms are conditional: LYING with
                    // `unconscious` → `ID_KO` (17); CARRIED with
                    // `life_points <= 0` → `ID_DEAD` (15).  Returns
                    // -1 on invalid / non-human actor, warns and
                    // returns -1 for unmapped variants.
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity(actor) else {
                        tracing::error!("Script Error: GetActorPosture invalid actor {actor}");
                        return -1;
                    };
                    if !entity.is_human() {
                        tracing::error!(
                            "Script Error: GetActorPosture target {actor} is not human"
                        );
                        return -1;
                    }
                    let posture = entity.element_data().posture;
                    let unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
                    let is_dead = entity.is_dead();
                    match posture {
                        Posture::Upright => 0,
                        Posture::Lying => {
                            if unconscious {
                                17
                            } else {
                                2
                            }
                        }
                        Posture::OnLadder => 4,
                        Posture::Siesta => 5,
                        Posture::Carried => {
                            if is_dead {
                                15
                            } else {
                                6
                            }
                        }
                        Posture::Flying => 8,
                        Posture::OnWall => 9,
                        Posture::Crouched => 10,
                        Posture::CarryingCorpse => 11,
                        Posture::Dead | Posture::DeadBack => 15,
                        Posture::Sitting => 16,
                        _ => {
                            tracing::warn!(
                                "GetActorPosture: unmapped posture {:?} on actor {actor}",
                                posture
                            );
                            -1
                        }
                    }
                }
                SetActorPosture => {
                    // The script-level argument uses the `ID_*`
                    // namespace, NOT the internal `Posture` enum
                    // discriminants — using `Posture::try_from` on the
                    // raw value silently corrupts every script call.
                    // This arm dispatches on the script IDs and
                    // writes the intended internal posture / clears
                    // concussion / drops life points where directly
                    // feasible, then enqueues a
                    // `DeferredCommand::LaunchWait` so the engine
                    // fires a low-priority `Wait` after each
                    // non-error arm — every successful branch of the
                    // switch ends with `Wait()`.
                    //
                    const ID_UPRIGHT: i32 = 0;
                    const ID_LYING: i32 = 2;
                    const ID_ON_LADDER: i32 = 4;
                    const ID_SIESTA: i32 = 5;
                    const ID_CARRIED: i32 = 6;
                    const ID_TIED: i32 = 7;
                    const ID_FLYING: i32 = 8;
                    const ID_CLIMBING: i32 = 9;
                    const ID_DODGED: i32 = 10;
                    const ID_CARRYING_CORPSE: i32 = 11;
                    const ID_DEAD: i32 = 15;
                    const ID_SITTING: i32 = 16;
                    const ID_KO: i32 = 17;
                    const ID_ANONYMOUS_ARCHER: i32 = 100;

                    let val = stack.pop_i32();
                    let actor = stack.pop_i32();

                    // ActorExists + IsHuman gates: warn and return on
                    // failure of either.
                    let Some(entity) = self.get_entity_mut(actor) else {
                        tracing::warn!("Script Error: SetActorPosture invalid actor {actor}");
                        return 0;
                    };
                    if !entity.is_human() {
                        tracing::warn!("Script Error: SetActorPosture target {actor} is not human");
                        return 0;
                    }

                    let is_npc = entity.is_npc();
                    let mut launch_wait = false;
                    match val {
                        ID_UPRIGHT => {
                            // Nested switch on current posture.
                            let current = entity.element_data().posture;
                            entity.set_posture(Posture::Upright);
                            // CARRYING_CORPSE branch skips the concussion
                            // clear; LYING and the default both clear it.
                            if current != Posture::CarryingCorpse
                                && let Some(h) = entity.human_data_mut()
                            {
                                h.concussion_of_the_brain = 0;
                                h.unconscious = false;
                            }
                            // From-LYING NPC branch broadcasts
                            // resurrection so other NPCs drop us
                            // from their detectable-body lists.
                            if current == Posture::Lying && is_npc {
                                self.deferred_commands
                                    .push(DeferredCommand::BroadcastResurrection { actor });
                            }
                            launch_wait = true;
                        }
                        ID_CARRIED | ID_FLYING | ID_CLIMBING | ID_ON_LADDER
                        | ID_CARRYING_CORPSE | ID_SIESTA => {
                            // Warn + return; never touches state.
                            // No Wait().
                            tracing::warn!(
                                "Script Error: SetActorPosture cannot set posture {val} from script"
                            );
                        }
                        ID_LYING => {
                            entity.set_posture(Posture::Lying);
                            if let Some(h) = entity.human_data_mut() {
                                h.concussion_of_the_brain = 0;
                                h.unconscious = false;
                            }
                            launch_wait = true;
                        }
                        ID_TIED => {
                            entity.set_posture(Posture::Tied);
                            // NPC branch fires
                            // Think(EVENT_LOSE_CONSCIOUSNESS) +
                            // detect-me broadcast before the Wait()
                            // so allies pick the body up.
                            if is_npc {
                                self.deferred_commands
                                    .push(DeferredCommand::BroadcastLoseConsciousness { actor });
                            }
                            launch_wait = true;
                        }
                        ID_SITTING => {
                            entity.set_posture(Posture::Sitting);
                            if let Some(h) = entity.human_data_mut() {
                                h.concussion_of_the_brain = 0;
                                h.unconscious = false;
                            }
                            launch_wait = true;
                        }
                        ID_KO => {
                            // ID_KO sequence:
                            // Stop(Injury) + SetPosture(LYING) +
                            // SetConcussion(CONCUSSION_SCRIPT, force=true) +
                            // (NPC) Think(EVENT_LOSE_CONSCIOUSNESS) +
                            // detect-me broadcast + Wait().
                            entity.set_posture(Posture::Lying);
                            if let Some(h) = entity.human_data_mut() {
                                // CONCUSSION_SCRIPT — saturating write to the
                                // unconscious threshold; the engine's
                                // wakeup/sleep state machine consumes this.
                                h.concussion_of_the_brain = crate::combat::CONCUSSION_MAX;
                                h.unconscious = true;
                            }
                            // Stop(Injury) before the posture stamp
                            // tears down any preference /
                            // normal-priority sequence the actor was
                            // running.  Order is Stop → SetPosture →
                            // SetConcussion → (NPC broadcasts) →
                            // Wait; the deferred queue drains in
                            // push order, so queue Stop first, then
                            // the NPC broadcasts, then LaunchWait
                            // below.
                            self.deferred_commands
                                .push(DeferredCommand::StopActorAtPriority {
                                    actor,
                                    priority: crate::sequence::SequencePriority::Injury,
                                });
                            if is_npc {
                                self.deferred_commands
                                    .push(DeferredCommand::BroadcastLoseConsciousness { actor });
                            }
                            launch_wait = true;
                        }
                        ID_DEAD => {
                            // SetLifePoints(0) +
                            // SetStates(Posture::Dead, ActionState::Waiting)
                            // + Wait().
                            entity.set_posture(Posture::Dead);
                            if let Some(a) = entity.actor_data_mut() {
                                a.action_state = ActionState::Waiting;
                            }
                            // `SetLifePoints(0)` fires the full
                            // death pipeline (sword-fight quit,
                            // dying anim, titbit cleanup).  Route
                            // through `HandleDeath`, which
                            // dispatches a synthetic lethal
                            // `ReceiveDamage` element (see
                            // engine/melee.rs::handle_death).
                            self.deferred_commands
                                .push(DeferredCommand::HandleDeath { actor });
                            launch_wait = true;
                        }
                        ID_ANONYMOUS_ARCHER => {
                            entity.set_posture(Posture::AnonymousArcher);
                            if let Some(a) = entity.actor_data_mut() {
                                a.action_state = ActionState::Waiting;
                            }
                            // Explicit AddTitbit(HIDDEN) — the
                            // script-level SetStates bypasses the
                            // stealth-command transition that
                            // normally seeds the HIDDEN titbit, so
                            // we re-add it via the deferred queue
                            // (handler resolves the per-PC phase).
                            self.deferred_commands
                                .push(DeferredCommand::AddHiddenTitbitForActor { actor });
                            launch_wait = true;
                        }
                        ID_DODGED => {
                            entity.set_posture(Posture::Crouched);
                            if let Some(a) = entity.actor_data_mut() {
                                a.action_state = ActionState::Waiting;
                            }
                            launch_wait = true;
                        }
                        _ => {
                            tracing::warn!("Script Error: SetActorPosture illegal ID {val}");
                        }
                    }
                    if launch_wait {
                        self.deferred_commands
                            .push(DeferredCommand::LaunchWait { actor });
                    }
                    0
                }
                GetActorDirection => {
                    let actor = stack.pop_i32();
                    self.get_entity(actor)
                        .map_or(0, |e| e.element_data().direction() as i32)
                }
                SetActorDirection => {
                    // Sets direction instantly; if the element is an
                    // FX target it additionally upgrades
                    // rendering_properties to NeedShadow (workaround
                    // for level-09 "tie soldier" sprite reuse).
                    // `rendering_properties` lives on `TargetData`,
                    // so the upgrade only applies to the
                    // `Entity::Target` variant.
                    let dir = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if let Some(entity) = self.get_entity_mut(actor) {
                        entity
                            .element_data_mut()
                            .set_direction_instantly(dir as i16);
                        if let Entity::Target(t) = entity {
                            t.target.rendering_properties =
                                crate::element_kinds::RenderingProperties::NeedShadow;
                        }
                    }
                    0
                }
                GetActorLocation => {
                    // Allocates a script point and stamps the
                    // actor's current (layer, sector) onto it so
                    // subsequent SetActorLocation round-trips
                    // preserve the sector.
                    let actor = stack.pop_i32();
                    match self.get_entity(actor) {
                        Some(entity) => {
                            let pos = entity.element_data().position_map();
                            let layer = entity.element_data().layer();
                            let sector = entity.element_data().sector().map(|s| s.get());
                            let meta = sector.map(|s| (layer, s));
                            self.create_computed_location_full(pos.x, pos.y, meta)
                        }
                        None => {
                            tracing::warn!("GetActorLocation: invalid actor handle {actor}");
                            0
                        }
                    }
                }
                SetActorLocation => {
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if loc == 0 {
                        // NULL location: deactivate actor (the
                        // "Honolulu" state).  Also quits swordfights,
                        // removes unconscious stars, disables PC
                        // playability, and script-locks NPC AI.
                        let is_pc;
                        let is_npc;
                        let is_human;
                        let is_playable;
                        let is_unlocked_npc;
                        if let Some(entity) = self.get_entity_mut(actor) {
                            is_pc = entity.is_pc();
                            is_npc = entity.is_npc();
                            is_human = entity.is_human();
                            is_playable = match entity {
                                Entity::Pc(e) => e.pc.playable,
                                _ => false,
                            };
                            is_unlocked_npc =
                                entity.ai_controller().is_some_and(|ai| !ai.script_locked);
                            let ed = entity.element_data_mut();
                            ed.active = false;
                            ed.in_honolulu = true;
                        } else {
                            return 0;
                        }
                        // Humans quit swordfight + remove
                        // unconscious stars.  `QuitSwordfight`'s
                        // handler already early-returns on
                        // non-humans via `human_data()` so the gate
                        // is implicit there; the unconscious-stars
                        // removal needs an explicit gate.
                        self.deferred_commands
                            .push(DeferredCommand::QuitSwordfight { actor });
                        if is_human {
                            self.deferred_commands
                                .push(DeferredCommand::RemoveUnconsciousStars { actor });
                        }
                        // Playable PCs lose playability.
                        if is_pc && is_playable {
                            self.deferred_commands.push(DeferredCommand::SetPlayable {
                                actor,
                                playable: false,
                            });
                        }
                        // Unlocked NPCs get script-locked.
                        if is_npc && is_unlocked_npc {
                            self.deferred_commands.push(DeferredCommand::ScriptLockAI {
                                actor,
                                send_back: false,
                            });
                        }
                    } else if let Some((x, y)) = self.resolve_location_pos(loc) {
                        // Read layer + sector from the resolved point
                        // and stamp them on the actor.  Static
                        // script locations carry that data; computed
                        // ones leave layer/sector untouched.
                        let dest_layer_sector = self.resolve_location_layer_sector(loc);
                        if let Some(entity) = self.get_entity_mut(actor) {
                            let ed = entity.element_data_mut();
                            if ed.in_honolulu {
                                ed.active = true;
                                ed.in_honolulu = false;
                            }
                            ed.set_position_map(crate::coordinates::MapPoint { x, y });
                            if let Some((layer, sector_num)) = dest_layer_sector {
                                ed.set_layer(layer);
                                ed.set_sector(crate::position_interface::SectorHandle::new(
                                    sector_num,
                                ));
                            }
                            ed.update_grid_cell();
                        }
                        // The engine command handles the full position update:
                        // SetObstacle, ComputePositionAll, ComputeDisplayOrder.
                        self.commands.push(EngineCommand::SetActorLocation {
                            actor_handle: actor,
                            x,
                            y,
                            dest_layer_sector,
                            // Regular SetActorLocation teleports onto an
                            // in-map point, so no spawn-elevation recompose
                            // is needed — `compute_position_all`
                            // after `set_position_map` derives Z from
                            // the sector's own plane at `(x, y)`.
                            spawn_elevation_probe: None,
                        });
                    } else {
                        tracing::warn!("SetActorLocation: invalid location handle {loc}");
                    }
                    0
                }
                IsInside => {
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if actor == 0 || loc == 0 {
                        return 0;
                    }
                    // Geometric polygon point-in-test recomputed
                    // every call so results stay correct immediately
                    // after teleport natives ("works also after
                    // teleports").  The cached `zone_occupants` map
                    // is only refreshed on explicit
                    // Add/CleanFromScriptZone natives or on the
                    // next-frame tick, so we recompute here when we
                    // have polygon geometry installed.
                    let zone_idx = Self::location_index(loc)
                        .and_then(|idx| idx.checked_sub(self.script_point_count));
                    if let Some(zi) = zone_idx
                        && let Some(zone) = self.script_zone_polygons.get(zi)
                        && let Some(entity) = self.get_entity(actor)
                    {
                        let ed = entity.element_data();
                        // Filter invisible objects.
                        if !ed.active || ed.in_honolulu {
                            return 0;
                        }
                        if zone.layer != ed.layer() {
                            return 0;
                        }
                        let pt = ed.position_map();
                        if !zone.bounding_box.contains_point(pt) {
                            return 0;
                        }
                        // Ray-casting point-in-polygon, identical to
                        // `GridSector::contains_point` (the production
                        // path used by `tick_zone_occupants`).
                        if zone.points.len() < 3 {
                            return 0;
                        }
                        let mut inside = false;
                        let n = zone.points.len();
                        let mut j = n - 1;
                        for i in 0..n {
                            let vi = zone.points[i];
                            let vj = zone.points[j];
                            if (vi.y > pt.y) != (vj.y > pt.y) {
                                let x_intersect =
                                    (vj.x - vi.x) * (pt.y - vi.y) / (vj.y - vi.y) + vi.x;
                                if pt.x < x_intersect {
                                    inside = !inside;
                                }
                            }
                            j = i;
                        }
                        i32::from(inside)
                    } else {
                        // Fall back to the cache when geometry isn't
                        // available (handle out of zone range, or
                        // pre-load test fixtures that never installed
                        // polygons).
                        i32::from(
                            self.zone_occupants
                                .get(&loc)
                                .is_some_and(|occ| occ.contains(&actor)),
                        )
                    }
                }
                IsInsideBuilding => {
                    let bld = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if actor == 0 {
                        return 0;
                    }
                    if bld == 0 {
                        // NULL building: check if actor is inside ANY building
                        i32::from(self.actor_building.contains_key(&actor))
                    } else {
                        // Check if actor is in the specific building
                        i32::from(self.actor_building.get(&actor) == Some(&bld))
                    }
                }
                UnBlip => {
                    // Gates on ActorExists (warn + false on invalid
                    // handle); returns true iff element was actually
                    // blipped before the call.
                    let actor = stack.pop_i32();
                    if !self.actor_exists(actor) {
                        tracing::error!("Script error (UnBlip): invalid actor handle {actor}");
                        return 0;
                    }
                    let was_blipped = self
                        .get_entity(actor)
                        .is_some_and(|e| e.element_data().blipped);
                    if let Some(entity) = self.get_entity_mut(actor) {
                        entity.reveal_blip();
                    }
                    i32::from(was_blipped)
                }
                GetMovementStyle => {
                    // Returns 1 if action_state == MovingFast, else 0.
                    let actor = stack.pop_i32();
                    self.get_entity(actor).map_or(0, |e| {
                        if e.actor_data()
                            .is_some_and(|a| a.action_state == ActionState::MovingFast)
                        {
                            1
                        } else {
                            0
                        }
                    })
                }
                GetCurrentAction => {
                    // (1) !ActorExists → warn + return 0
                    // (2) Object → return the object's animation
                    // (3) !IsActor → warn + return 0
                    // (4) Actor → return the front order's
                    //     `order_type`.
                    //
                    // The cached `current_animations` map carries
                    // both branches: actors stamped from the front
                    // order's `order_type`, objects stamped from
                    // `ObjectData::animation`. Missing handles fall
                    // back to the warn + 0 paths.
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity(actor) else {
                        tracing::warn!(
                            "Script Error: GetCurrentAction on invalid actor handle {actor}"
                        );
                        return 0;
                    };
                    if entity.is_object() {
                        self.current_animations
                            .get(&actor)
                            .copied()
                            .map(|a| a as i32)
                            .unwrap_or(0)
                    } else if entity.is_actor() {
                        self.current_animations
                            .get(&actor)
                            .copied()
                            .map(|a| a as i32)
                            .unwrap_or(0)
                    } else {
                        tracing::warn!(
                            "Script Error: GetCurrentAction on illegal actor handle {actor} (not actor, not object)"
                        );
                        0
                    }
                }
                InflictPain => {
                    // Launches a one-element damage sequence
                    // through the sequence manager instead of
                    // mutating life points inline — this lets the
                    // victim's `instruct` handler queue the hit
                    // animation, routes through
                    // `apply_generic_damage` for posture/death
                    // transitions, and honours sequence priority so
                    // the damage can't preempt a higher-priority
                    // non-interruptable element.
                    let pain_type = stack.pop_i32();
                    let amount = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if !self.actor_exists(actor) {
                        tracing::warn!("Script Error: InflictPain on invalid actor handle {actor}");
                        return 0;
                    }
                    // `amount` flows straight into a u16 slot;
                    // negative scripts wrap.
                    let damage = amount as u16;
                    let concussion = if pain_type != 0 { 100u16 } else { 0u16 };
                    let target = self
                        .actor_id(actor)
                        .expect("InflictPain: actor_exists check passed but actor_id None");
                    self.completed_sequences
                        .push(Sequence::single_damage(target, damage, concussion));
                    // Returns true on success.
                    1
                }
                SetCompanyNumber => {
                    let num = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if let Some(entity) = self.get_entity_mut(actor) {
                        if let Some(enemy) = entity.enemy_ai_mut() {
                            // `company_number` is a u16.
                            enemy.company_number = num as u16;
                        } else {
                            tracing::warn!(
                                "Script Error: SetCompanyNumber on non-soldier actor {actor}"
                            );
                        }
                    }
                    0
                }
                SetAlwaysAttentive => {
                    // Set the AI's `forced_attentive` flag, launch
                    // an `EnterAttentiveMode` sequence on the
                    // false→true transition, and — on the true
                    // branch with frame>1 and the NPC already on
                    // GREEN — bump the alert to YELLOW.
                    let val = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let target = val != 0;
                    let mut launch_enter = false;
                    let frame = self.frame_counter;
                    if let Some(entity) = self.get_entity_mut(actor) {
                        if let Some(enemy) = entity.enemy_ai_mut() {
                            enemy.forced_attentive = target;
                            if target && !enemy.will_be_attentive {
                                enemy.will_be_attentive = true;
                                launch_enter = true;
                            }
                        }
                        if target
                            && frame > 1
                            && let Some(enemy) = entity.enemy_ai_mut()
                            && enemy.base.current_music_alert_status == AlertLevel::Green
                        {
                            // Route through the soldier wrapper so view
                            // tracks the override; SetAlwaysAttentive
                            // already updated `forced_attentive` above.
                            enemy.set_alert_status(AlertLevel::Yellow);
                        }
                    }
                    if launch_enter && let Some(target_id) = self.actor_id(actor) {
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new(
                            1,
                            Command::EnterAttentiveMode,
                            Some(target_id),
                        ));
                        self.completed_sequences.push(seq);
                    }
                    0
                }
                SetInvisible => {
                    // Warns separately on "inexisting actor" and
                    // "non-human" so the scripting team can
                    // distinguish the two failures.
                    let val = stack.pop_i32();
                    let actor = stack.pop_i32();
                    match self.get_entity_mut(actor) {
                        None => {
                            tracing::warn!("SetInvisible: actor {actor} does not exist");
                        }
                        Some(entity) => match entity.human_data_mut() {
                            Some(human) => {
                                human.hollow_man = val != 0;
                            }
                            None => {
                                tracing::warn!("SetInvisible: actor {actor} is not human");
                            }
                        },
                    }
                    0
                }
                IsInvisible => {
                    let actor = stack.pop_i32();
                    self.get_entity(actor).map_or(0, |e| {
                        i32::from(e.human_data().is_some_and(|h| h.hollow_man))
                    })
                }
                MakePCCrouched => {
                    let actor = stack.pop_i32();
                    // Route through the engine layer so the full
                    // sequence/animation/path rewrite happens with
                    // engine-side state.  Validation (ActorExists +
                    // IsPC) and the actual `actor_make_crouched`
                    // call happen in the engine-side handler.
                    self.commands.push(EngineCommand::ScriptMakePCCrouched {
                        actor_handle: actor,
                    });
                    0
                }
                GetActorActionState => {
                    // Returns -1 on invalid actor or non-human.  The
                    // Rust `ActionState` enum discriminants coincide
                    // with the script `ID_ACTIONSTATE_*` constants
                    // 0..=17, so a direct `as i32` cast is correct
                    // on the happy path.
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity(actor) else {
                        tracing::error!("Script Error: GetActorActionState invalid actor {actor}");
                        return -1;
                    };
                    if !entity.is_human() {
                        tracing::error!(
                            "Script Error: GetActorActionState target {actor} is not human"
                        );
                        return -1;
                    }
                    entity.actor_data().map_or(-1, |a| a.action_state as i32)
                }
                SetActorActionState => {
                    // Validates ActorExists + IsHuman (warn + early
                    // return on either failure).  Every arm then
                    // calls `set_action_state(s) + Wait()`.  The
                    // trailing Wait() launches a low-priority Wait
                    // sequence element on the actor, displacing any
                    // in-flight sequence so the freshly-stamped
                    // action state actually takes hold.
                    let val = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity_mut(actor) else {
                        tracing::warn!("Script Error: SetActorActionState invalid actor {actor}");
                        return 0;
                    };
                    if !entity.is_human() {
                        tracing::warn!(
                            "Script Error: SetActorActionState target {actor} is not human"
                        );
                        return 0;
                    }
                    let Some(actor_data) = entity.actor_data_mut() else {
                        return 0;
                    };
                    let Ok(s) = ActionState::try_from(val as u32) else {
                        tracing::warn!("SetActorActionState: invalid value {val}");
                        return 0;
                    };
                    actor_data.action_state = s;
                    self.deferred_commands
                        .push(DeferredCommand::LaunchWait { actor });
                    0
                }

                // --- vision / interaction ---
                Sees => {
                    // Sees(Actor npc, Actor target) -> bool
                    //
                    // Validates both handles (NPC observer + Human
                    // target) and then returns whether the NPC's
                    // visibility computation for the target is > 0.
                    //
                    // The pre-rewrite version peeked the EventView
                    // stimulus queue, which is racy: the queue is
                    // drained by the AI state machine on the same
                    // tick, so a script call after the AI tick saw
                    // `false` for a target the NPC was actively
                    // engaging.  The synchronous `compute_visibility`
                    // call gives the right answer regardless of tick
                    // phase.
                    //
                    let target_h = stack.pop_i32();
                    let npc_h = stack.pop_i32();

                    // Four warn + return-false validation gates.
                    let Some(npc_entity) = self.get_entity(npc_h) else {
                        tracing::warn!(
                            "Script Error: Trying to test if an invalid actor element ({npc_h}) sees another actor."
                        );
                        return 0;
                    };
                    if npc_entity.ai_controller().is_none() {
                        tracing::warn!(
                            "Script Error: Trying to test if a non-NPC element ({npc_h}) sees another actor."
                        );
                        return 0;
                    }
                    let Some(target_entity) = self.get_entity(target_h) else {
                        tracing::warn!(
                            "Script Error: Trying to test if a NPC sees an invalid actor element ({target_h})."
                        );
                        return 0;
                    };
                    if target_entity.human_data().is_none() {
                        tracing::warn!(
                            "Script Error: Trying to test if a NPC sees a non-human ({target_h})."
                        );
                        return 0;
                    }

                    // Read everything we need off the live entity
                    // store for fields that move per-frame (position,
                    // direction, posture, view parameters, eye Z).
                    // For building membership, use the GameHost copy
                    // refreshed from the engine before each script call.
                    // The old entity-view cache was AI-dispatch scratch,
                    // not script or simulation state.
                    let npc_dir = npc_entity.element_data().direction();
                    let npc_layer = npc_entity.element_data().layer();
                    let Some(npc_data) = npc_entity.npc_data() else {
                        tracing::warn!(
                            "Script Error: NPC {npc_h} has no NpcData (view parameters missing)."
                        );
                        return 0;
                    };
                    let view_radius = npc_data.view_radius;
                    let eye_status = npc_data.eye_status;
                    let real_half_aperture = npc_data.real_half_aperture;
                    let view_direction = npc_data.view_direction;
                    let viewer_eye_3d = npc_entity
                        .compute_eyes_point(None)
                        .expect("Sees validated an NPC observer, which must have an eye point");
                    let viewer_eye = crate::coordinates::MapPoint::from_world_xyz(
                        viewer_eye_3d.x,
                        viewer_eye_3d.y,
                        npc_entity.element_data().position().z,
                    );

                    let viewer_building = self.actor_building.get(&npc_h).copied();
                    let viewer_building_sector = viewer_building
                        .and_then(|h| crate::position_interface::SectorHandle::new(h as u16));
                    let viewer_in_building = viewer_building.is_some();

                    // Target side.
                    let tgt_layer = target_entity.element_data().layer();
                    let tgt_posture = target_entity.element_data().posture;
                    let tgt_action_state = target_entity
                        .actor_data()
                        .expect("Sees validated a human target, which must have actor data")
                        .action_state;
                    let tgt_active = target_entity.element_data().active;
                    let target_building = self.actor_building.get(&target_h).copied();
                    let tgt_building_sector = target_building
                        .and_then(|h| crate::position_interface::SectorHandle::new(h as u16));
                    let tgt_in_building = target_building.is_some();
                    let tgt_unconscious = target_entity
                        .human_data()
                        .expect("Sees validated a human target, which must have human data")
                        .unconscious;
                    let tgt_passing_door = target_entity
                        .actor_data()
                        .expect("Sees validated a human target, which must have actor data")
                        .active_door_pass
                        .is_some();
                    let tgt_is_pc = matches!(target_entity, Entity::Pc(_));
                    // Use the same target-point helper as the authoritative
                    // AI detection pass. The 3D point supplies only the
                    // posture-adjusted Z used by the close-range test.
                    let tgt_detection_3d = target_entity
                        .compute_detection_point()
                        .expect("Sees validated a human target, which must have a detection point");
                    let target_point = crate::stealth::detection_point_xy(
                        target_entity.element_data().position_map(),
                        tgt_posture,
                        target_entity.element_data().direction(),
                    );

                    // Different layer ⇒ no LOS; the sight raycast
                    // wouldn't cross floors.  Same layer guard the
                    // AI detection path uses before
                    // VisibilityQuery construction.
                    if tgt_layer != npc_layer {
                        return 0;
                    }

                    let view_forward = (view_direction[0], view_direction[1]);
                    let golden_eye_mode = self.ai_global.golden_eye_mode;
                    let target_in_same_building =
                        viewer_in_building && tgt_building_sector == viewer_building_sector;

                    let sight_obstacles = script_sight_obstacles();
                    let sight_obstacle_list = sight_obstacles.list();
                    let target_obstacle = target_entity
                        .element_data()
                        .obstacle_index()
                        .map(|handle| {
                            sight_obstacle_list
                                .get(usize::from(handle))
                                .unwrap_or_else(|| {
                                    panic!(
                                        "Sees target {target_h} references missing sight obstacle {}",
                                        usize::from(handle)
                                    )
                                })
                        });
                    let is_night_or_fog = matches!(
                        self.ambiance,
                        crate::engine::Ambiance::Night | crate::engine::Ambiance::Fog
                    );
                    let effective_view_radius = crate::ai_vision::compute_view_radius(
                        viewer_eye,
                        viewer_eye_3d.z,
                        view_radius,
                        view_forward,
                        real_half_aperture,
                        is_night_or_fog,
                        &self.fast_grid.level,
                        sight_obstacle_list,
                        target_obstacle,
                    );
                    let forest_180_degree_view =
                        self.is_forest_level && npc_entity.camp() == Camp::Royalists;
                    if forest_180_degree_view && !npc_entity.is_active() {
                        return 0;
                    }
                    let q = crate::ai_vision::VisibilityQuery {
                        viewer: viewer_eye,
                        viewer_direction: npc_dir,
                        view_forward,
                        view_radius,
                        viewer_eye_status: eye_status,
                        real_half_aperture,
                        viewer_in_building,
                        target_in_same_building,
                        forest_180_degree_view,
                        golden_eye_mode,
                        effective_view_radius,
                        target_is_active_and_outside_building: tgt_active && !tgt_in_building,
                        target: target_point,
                        target_posture: tgt_posture,
                        target_action_state: tgt_action_state,
                        target_is_pc: tgt_is_pc,
                        viewer_eye_z: viewer_eye_3d.z,
                        target_eye_z: tgt_detection_3d.z,
                        sight_obstacles: sight_obstacle_list,
                        fast_grid: &self.fast_grid,
                        layer: npc_layer,
                        target_unconscious: tgt_unconscious,
                        target_passing_door: tgt_passing_door,
                    };
                    if crate::ai_vision::compute_visibility(&q) > 0.0 {
                        1
                    } else {
                        0
                    }
                }
                EnableViewCone => {
                    // Validates IsNPC (warn only, no early return)
                    // and then either (a) triggers the Ezekiel2517
                    // "Dies irae" cheat — a 10000 HP info-priority
                    // damage sequence — or (b) stores the actor as
                    // the engine's single selected view element.
                    // We emulate (b) by setting the per-AI debug
                    // flag on the target and clearing it on every
                    // other NPC, so calling twice on the same actor
                    // keeps it selected, and calling on a different
                    // actor replaces the selection.
                    let actor = stack.pop_i32();
                    if !self.actor_exists(actor) {
                        return 0;
                    }
                    // NPC-type check (warn, no early return).
                    let is_npc = self
                        .get_entity(actor)
                        .is_some_and(|e| e.ai_controller().is_some());
                    if !is_npc {
                        tracing::warn!(
                            "Script Error: Trying to enable the view cone of an element which is not a NPC."
                        );
                    }
                    if self.ai_global.ezekiel_2517 {
                        // "Dies irae" cheat: 10000 HP info-priority
                        // damage on the target (asserts IsHuman).
                        if let Some(target) = self.actor_id(actor)
                            && self
                                .get_entity(actor)
                                .is_some_and(|e| e.human_data().is_some())
                        {
                            self.completed_sequences
                                .push(Sequence::single_damage(target, 10000, 0));
                        }
                    } else if is_npc {
                        let target_id = self.actor_id(actor);
                        for (entity_id, entity) in self.occupied_entities_mut() {
                            let Some(ai) = entity.ai_controller_mut() else {
                                continue;
                            };
                            ai.debug_view_cone_enabled = target_id == Some(entity_id);
                        }
                    }
                    0
                }
                PrototypeFilterEvent => {
                    // Delegates to the prototype's per-actor script
                    // `FilterAIEvent(source, event)`, which runs in
                    // the *prototype's* per-actor VMCore and returns
                    // whatever int the script computes.
                    //
                    // We can't re-enter the script subsystem inline here:
                    // `&mut self` is the `GameHost` swapped into the
                    // currently-running VM, with no back-pointer to
                    // `MissionScript`.  Instead, queue a `PendingNestedCall`
                    // — the interpreter will yield with
                    // `StopReason::PendingNestedCall` (see
                    // `interp.rs::NativeCall`), and
                    // `MissionScript::call_actor_function`'s loop will
                    // dispatch the prototype's `FilterAIEvent`, then write
                    // its real return into `vm.native_return_value` and
                    // resume this VM.  The `0` we return here is a
                    // placeholder that's overwritten before any script
                    // instruction reads `native_return_value`.
                    let i_event = stack.pop_i32();
                    let actor_source = stack.pop_i32();
                    let prototype = stack.pop_i32();
                    self.pending_nested_call = Some(crate::interp::PendingNestedCall {
                        actor_handle: prototype,
                        fn_name: "FilterAIEvent".into(),
                        params: vec![actor_source, i_event],
                    });
                    0
                }
                SendMessage => {
                    let msg = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Non-null non-actor handle → warn + no dispatch.
                    if actor != 0 && !self.is_actor_handle(actor) {
                        tracing::error!(
                            "Script Error : trying to send a message to non actor object."
                        );
                        return 0;
                    }
                    // RHScript::SendMessage builds RHCOMMAND_SEND_MESSAGE and
                    // calls LaunchSequenceElement. EngineInner owns the
                    // sequence manager, so carry the launch request across
                    // the VM boundary instead of directly dispatching the
                    // target callback here.
                    self.deferred_commands.push(DeferredCommand::SendMessage {
                        actor,
                        message: msg,
                        arg1: 0,
                        arg2: 0,
                    });
                    0
                }
                SendMessageWithArguments => {
                    let arg2 = stack.pop_i32();
                    let arg1 = stack.pop_i32();
                    let msg = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Same IsActor guard as SendMessage.
                    if actor != 0 && !self.is_actor_handle(actor) {
                        tracing::error!(
                            "Script Error : trying to send a message to non actor object."
                        );
                        return 0;
                    }
                    // See SendMessage above: this is a sequence-element
                    // launch request, not a post-call ProcessMessage request.
                    self.deferred_commands.push(DeferredCommand::SendMessage {
                        actor,
                        message: msg,
                        arg1,
                        arg2,
                    });
                    0
                }

                // --- action / property ---
                SetActionAvailable => {
                    // Validates `IsPC(actor)` then `action ∈ [0, 5]`,
                    // forwards an enable/disable message, and returns
                    // true / false. The C++ message handlers do not
                    // mutate RHElementActorPC::mpbDisabledActions; real
                    // persistent disables are maintained by PC ammo /
                    // action methods.
                    let _avail = stack.pop_i32();
                    let action_idx = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity(actor) else {
                        tracing::error!("Script Error: SetActionAvailable invalid actor {actor}");
                        return 0;
                    };
                    if !entity.is_pc() {
                        tracing::error!(
                            "Script Error: SetActionAvailable target {actor} is not a PC"
                        );
                        return 0;
                    }
                    if !(0..=5).contains(&action_idx) {
                        tracing::error!(
                            "Script Error: SetActionAvailable action index {action_idx} out of range"
                        );
                        return 0;
                    }
                    1
                }
                IsActionAvailable => {
                    // Consults BOTH the persistent and temporary
                    // disabled-action masks — an action is available
                    // only if neither slot is set.
                    let action_idx = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity(actor) else {
                        tracing::error!("Script Error: IsActionAvailable invalid actor {actor}");
                        return 0;
                    };
                    let Some(pc) = entity.pc_data() else {
                        tracing::error!(
                            "Script Error: IsActionAvailable target {actor} is not a PC"
                        );
                        return 0;
                    };
                    if !(0..=5).contains(&action_idx) {
                        tracing::error!(
                            "Script Error: IsActionAvailable action index {action_idx} out of range"
                        );
                        return 0;
                    }
                    let idx = action_idx as usize;
                    let disabled_persistent =
                        pc.disabled_actions.get(idx).copied().unwrap_or(false);
                    let disabled_temp = pc.disabled_actions_temp.get(idx).copied().unwrap_or(false);
                    if disabled_persistent || disabled_temp {
                        0
                    } else {
                        1
                    }
                }
                SetPersistentProperty => {
                    let amount = stack.pop_i32();
                    let prop = stack.pop_i32();
                    let actor = stack.pop_i32();
                    self.set_persistent_property(actor, prop, amount) as i32
                }
                GetPersistentProperty => {
                    let prop = stack.pop_i32();
                    let actor = stack.pop_i32();
                    self.get_persistent_property(actor, prop)
                }
                IsAnyCivilianDead => {
                    if self.any_civilian_dead {
                        1
                    } else {
                        0
                    }
                }
                IsAnyEnemyDead => {
                    if self.any_enemy_dead {
                        1
                    } else {
                        0
                    }
                }
                GetOverallEnemyAlert => self.overall_enemy_alert,
                GetOverallCivilianAlert => self.overall_civilian_alert,
                HasPCAction => {
                    let action_code = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // Guards: ActorExists + IsPC; warn and return
                    // false on either failure.
                    let Some(entity) = self.get_entity(actor) else {
                        tracing::warn!(
                            "Script Error: Trying to call HasPCAction for invalid actor element."
                        );
                        return 0;
                    };
                    if entity.pc_data().is_none() {
                        tracing::warn!("Script Error: Trying to call HasPCAction for non-PC.");
                        return 0;
                    }
                    let Ok(script_action) =
                        crate::profiles::ScriptAction::try_from(action_code as u32)
                    else {
                        tracing::warn!(
                            "Script Error: HasPCAction with bad action ID {action_code}"
                        );
                        return 0;
                    };
                    let action = script_action.to_action();
                    // Direct PC->profile lookup (no raw-profile-index fallback for this native).
                    let Some(&profile_idx) = self.pc_profile_map.get(&actor) else {
                        return 0;
                    };
                    self.campaign.as_ref().expect("campaign required");
                    self.profile_manager
                        .get_character(profile_idx)
                        .map_or(0, |cp| {
                            let has = cp.actions.contains(&action)
                                || cp.contextual_actions.contains(&action);
                            if has { 1 } else { 0 }
                        })
                }
                HasAnyPCAction => {
                    let action_code = stack.pop_i32();
                    let Ok(script_action) =
                        crate::profiles::ScriptAction::try_from(action_code as u32)
                    else {
                        tracing::warn!(
                            "Script Error: HasAnyPCAction with bad action ID {action_code}"
                        );
                        return 0;
                    };
                    let action = script_action.to_action();
                    // Iterate the spawned-PC array (not the
                    // campaign-wide gang list).  `pc_handles` is
                    // populated from the engine each script tick.
                    let profiles = &self.profile_manager;
                    for handle in &self.pc_handles {
                        let Some(&profile_idx) = self.pc_profile_map.get(handle) else {
                            continue;
                        };
                        let Some(cp) = profiles.get_character(profile_idx) else {
                            continue;
                        };
                        if cp.actions.contains(&action) || cp.contextual_actions.contains(&action) {
                            return 1;
                        }
                    }
                    0
                }
                HasAnyActivePCAction => {
                    // Like HasAnyPCAction but also requires the PC to be "playable"
                    // (alive, active, not guarded). Check entity state for playable,
                    // then campaign profile for action availability.
                    let action_code = stack.pop_i32();
                    let Ok(script_action) =
                        crate::profiles::ScriptAction::try_from(action_code as u32)
                    else {
                        tracing::warn!(
                            "Script Error: HasAnyActivePCAction with bad action ID {action_code}"
                        );
                        return 0;
                    };
                    let action = script_action.to_action();

                    // Filter solely on `playable`, not death.  The
                    // death pipeline is responsible for clearing
                    // `playable`; do not double-filter here.
                    let playable_profiles: Vec<crate::profiles::CharacterProfileIdx> = self
                        .entities
                        .iter()
                        .filter_map(|slot| {
                            let entity = slot.as_ref()?;
                            let pc = entity.pc_data()?;
                            if !pc.playable {
                                return None;
                            }
                            Some(pc.profile_index)
                        })
                        .collect();

                    let profiles = &self.profile_manager;
                    for pi in &playable_profiles {
                        let Some(cp) = profiles.get_character(*pi) else {
                            continue;
                        };
                        if cp.actions.contains(&action) || cp.contextual_actions.contains(&action) {
                            return 1;
                        }
                    }
                    0
                }
                HasAnyActionSelected => {
                    // Checks whether the PC is selected and has a
                    // non-NoAction action selected.
                    let actor = stack.pop_i32();
                    if !self.actor_exists(actor) {
                        tracing::error!(
                            "Script Error: HasAnyActionSelected for invalid actor {actor}"
                        );
                        return 0;
                    }
                    let entity = self.get_entity(actor).unwrap();
                    if !entity.is_pc() {
                        tracing::error!("Script Error: HasAnyActionSelected for non-PC {actor}");
                        return 0;
                    }
                    // Must be selected
                    if !self.selected_pc_handles.contains(&actor) {
                        return 0;
                    }
                    // Check if any action is selected (non-NoAction)
                    if entity
                        .pc_data()
                        .is_some_and(|pc| pc.current_action != Action::NoAction)
                    {
                        1
                    } else {
                        0
                    }
                }

                // --- AI ---
                SetAIAlertStatus => {
                    // Reject (1) missing actor, (2) PCs, (3)
                    // non-NPCs, (4) illegal alert values — each with
                    // its own warning + false return.  The actual
                    // alert write + music propagation still happens
                    // via the per-frame overall-alert sweep.
                    let val = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity_mut(actor) else {
                        tracing::error!("Script Error: SetAIAlertStatus invalid actor {actor}");
                        return 0;
                    };
                    if entity.is_pc() {
                        tracing::error!("Script Error: SetAIAlertStatus target {actor} is a PC");
                        return 0;
                    }
                    if !entity.is_npc() {
                        tracing::error!(
                            "Script Error: SetAIAlertStatus target {actor} is not an NPC"
                        );
                        return 0;
                    }
                    let Ok(level) = AlertLevel::try_from(val as u32) else {
                        tracing::error!("Script Error: SetAIAlertStatus illegal alert value {val}");
                        return 0;
                    };
                    // Route soldiers through the enemy-side wrapper
                    // so the forced-attentive view-override is
                    // applied; civilians fall through to the base
                    // setter (override is soldier-only and would
                    // always be `false` for them).
                    if let Some(enemy) = entity.enemy_ai_mut() {
                        enemy.set_alert_status(level);
                    } else if let Some(ai) = entity.ai_controller_mut() {
                        ai.set_alert_status(level);
                    }
                    1
                }
                GetAIAlertStatus => {
                    let actor = stack.pop_i32();
                    // Warn + return false on missing actor / non-NPC.
                    let Some(entity) = self.get_entity(actor) else {
                        tracing::error!("Script Error: GetAIAlertStatus invalid actor {actor}");
                        return 0;
                    };
                    let Some(ai) = entity.ai_controller() else {
                        tracing::error!(
                            "Script Error: GetAIAlertStatus target {actor} is not an NPC"
                        );
                        return 0;
                    };
                    // Read the view-parameter alert status — the
                    // field that `SetAlertStatus` pins to YELLOW for
                    // forced-attentive soldiers on Green.
                    ai.view_alert_status as i32
                }
                SetAIState => {
                    // Takes `AISTATE_*` script constants; most match
                    // the internal `AiState` enum 1:1, but
                    // `AISTATE_SCRIPT_DRIVEN` (7) is NOT a state —
                    // it's an alias that writes
                    // `(Default, DefaultScriptDriven)` so scripts
                    // can park an NPC in "hands off, the script
                    // drives it" mode.
                    let val = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let Some(entity) = self.get_entity_mut(actor) else {
                        tracing::error!("Script Error: SetAIState invalid actor {actor}");
                        return 0;
                    };
                    if !entity.is_npc() {
                        tracing::error!("Script Error: SetAIState target {actor} is not an NPC");
                        return 0;
                    }
                    // Script-driven pseudo-state (7).  Park the NPC in
                    // Default/DefaultScriptDriven; no stimulus fires.
                    if val as u32 == crate::ai::AiState::SCRIPT_DRIVEN {
                        if let Some(ai) = entity.ai_controller_mut() {
                            ai.set_ai_state(crate::ai::AiState::Default);
                            ai.current_substate = crate::ai::Substate::DefaultScriptDriven;
                        }
                        return 1;
                    }
                    let Ok(state) = AiState::try_from(val as u32) else {
                        tracing::error!(
                            "Script Error: SetAIState illegal state value {val} on actor {actor}"
                        );
                        return 0;
                    };
                    // The `in_macro` flag is only set by the
                    // internal macro-VM caller, never by a script
                    // native — pass `false` here.
                    let data = entity.element_data();
                    let p = data.position_map();
                    let current_position = crate::ai::Position {
                        x: p.x,
                        y: p.y,
                        sector: data.sector(),
                        level: data.layer(),
                    };
                    let self_is_soldier = matches!(entity, Entity::Soldier(_));
                    // SEEKING on a civilian warns and returns false.
                    if state == AiState::Seeking && !self_is_soldier {
                        tracing::error!(
                            "Script Error: SetAIState(SEEKING) on civilian NPC {actor}"
                        );
                        return 0;
                    }
                    if let Some(ai) = entity.ai_controller_mut() {
                        ai.script_set_ai_state(state, current_position, false, self_is_soldier);
                    }
                    1
                }
                GetAIState => {
                    let actor = stack.pop_i32();
                    self.get_entity(actor).map_or(0, |e| {
                        e.ai_controller()
                            .map_or(0, |ai| ai.current_state.to_script_code())
                    })
                }
                SetAIAttitude => {
                    // Retired stub: just logs "attitudes are fixed
                    // in the profiles and cannot be changed" and
                    // returns false.  Profile-sourced attitudes
                    // remain authoritative.
                    let _val = stack.pop_i32();
                    let _actor = stack.pop_i32();
                    tracing::warn!(
                        "SetAIAttitude called but attitudes are fixed in profiles (no-op)"
                    );
                    0
                }
                GetAIAttitude => {
                    // Switches on the NPC's camp: Royalists → 0
                    // (FRIENDLY), Lacklandists → 1 (HOSTILE).
                    // Attitude is not a stored field at the script
                    // boundary — it is a pure function of camp
                    // membership.
                    let actor = stack.pop_i32();
                    match self.get_entity(actor) {
                        None => {
                            tracing::error!("Script Error: GetAIAttitude invalid actor {actor}");
                            0
                        }
                        Some(e) if !e.is_npc() => {
                            tracing::error!(
                                "Script Error: GetAIAttitude target {actor} is not an NPC"
                            );
                            0
                        }
                        Some(e) => match e.camp() {
                            Camp::Royalists => 0,
                            Camp::Lacklandists => 1,
                            Camp::Error => 0,
                        },
                    }
                }
                SetAILevel => {
                    // Retired stub: the body is entirely commented
                    // out.  Validate handle + NPC-ness to preserve
                    // diagnostic output, but do NOT mutate any state
                    // — no field exists and no ported caller reads a
                    // derived value.
                    let _value = stack.pop_i32();
                    let _property = stack.pop_i32();
                    let actor = stack.pop_i32();
                    match self.get_entity(actor) {
                        None => {
                            tracing::error!("Script Error: SetAILevel invalid actor {actor}");
                            return 0;
                        }
                        Some(e) if !e.is_npc() => {
                            tracing::error!(
                                "Script Error: SetAILevel target {actor} is not an NPC"
                            );
                            return 0;
                        }
                        _ => {}
                    }
                    1
                }
                StareActor => {
                    // StareActor(Actor npc, Actor target, int duration_frames)
                    // Makes npc face toward target for duration frames. 0 = stop staring.
                    let duration = stack.pop_i32();
                    let target = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let target_handle = u32::try_from(target)
                        .ok()
                        .and_then(std::num::NonZeroU32::new)
                        .map(std::num::NonZeroU32::get);
                    if duration > 0
                        && let Some(target_handle) = target_handle
                        && self.get_entity(target_handle as i32).is_none()
                    {
                        tracing::error!("Script Error: StareActor invalid target {target_handle}");
                        return 0;
                    }
                    if let Some(entity) = self.get_entity_mut(actor)
                        && let Some(ai) = entity.ai_controller_mut()
                    {
                        if duration > 0
                            && let Some(target_handle) = target_handle
                        {
                            ai.stare_target_actor = Some(target_handle);
                            ai.stare_target_position = None;
                            ai.stare_remaining = duration as u32;
                        } else {
                            ai.stare_target_actor = None;
                            ai.stare_target_position = None;
                            ai.stare_remaining = 0;
                        }
                    }
                    0
                }
                StareLocation => {
                    // StareLocation(Actor npc, Location loc, int duration_frames)
                    // Makes npc face toward a location for duration frames.
                    let duration = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let resolved_pos =
                        self.resolve_location_pos(loc)
                            .map(|(x, y)| crate::ai::Position {
                                x,
                                y,
                                ..Default::default()
                            });
                    if let Some(entity) = self.get_entity_mut(actor)
                        && let Some(ai) = entity.ai_controller_mut()
                    {
                        if duration > 0 {
                            ai.stare_target_actor = None;
                            ai.stare_target_position = resolved_pos;
                            ai.stare_remaining = duration as u32;
                        } else {
                            ai.stare_target_actor = None;
                            ai.stare_target_position = None;
                            ai.stare_remaining = 0;
                        }
                    }
                    0
                }
                AssignPath => {
                    // Three cases:
                    //   way == 0   → clear, no sit-around
                    //   way == -1  → clear, sit-around
                    //   way == idx → adopt path
                    let way = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let hiking_paths = self.hiking_paths.clone();
                    if let Some(entity) = self.get_entity_mut(actor) {
                        let data = entity.element_data();
                        let p = data.position_map();
                        let current_position = crate::ai::Position {
                            x: p.x,
                            y: p.y,
                            sector: data.sector(),
                            level: data.layer(),
                        };
                        let current_direction =
                            entity.position_iface().get_direction().as_u8() as u16;
                        if let Some(ai) = entity.ai_controller_mut() {
                            let assignment = if way == 0 {
                                crate::ai::PatrolAssignment::ClearPath
                            } else if way == -1 {
                                crate::ai::PatrolAssignment::ClearPathSitAround
                            } else {
                                match crate::ai::PathId::new(way as u16) {
                                    Some(pid) => crate::ai::PatrolAssignment::Index(pid),
                                    None => crate::ai::PatrolAssignment::ClearPath,
                                }
                            };
                            ai.assign_new_patrol_path(
                                assignment,
                                current_position,
                                current_direction,
                                &hiking_paths,
                            );
                        }
                    }
                    0
                }
                AssignPost => {
                    // AssignPost(Actor, Location, int direction) -> 0
                    // Drops the active patrol path, installs the
                    // post as the NPC's new initial-pos /
                    // view-direction anchor, clears the three
                    // authored flags, and — when not script-locked
                    // and in the default state — fires
                    // EventReturnToDuty so the NPC walks to the
                    // post.
                    let direction = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let Some(resolved_xy) = self.resolve_location_pos(loc) else {
                        tracing::warn!("AssignPost: invalid location handle {loc}");
                        return 0;
                    };
                    if let Some(entity) = self.get_entity_mut(actor) {
                        let data = entity.element_data();
                        let post_position = crate::ai::Position {
                            x: resolved_xy.0,
                            y: resolved_xy.1,
                            sector: data.sector(),
                            level: data.layer(),
                        };
                        if let Some(ai) = entity.ai_controller_mut() {
                            ai.assign_new_post(post_position, direction as u16);
                        }
                    }
                    0
                }
                ForceBattleDecision => {
                    // Soldier-only.  `decision >= 100` peels off an
                    // `always` prefix (always=true, decrement by
                    // 100) that is translated into
                    // `!reset_battle_decision` on the soldier.  The
                    // decision ID is mapped through a
                    // BATTLE_DECISION_* → `Decision` switch; unknown
                    // IDs warn and skip the mutation.
                    let mut decision_arg = stack.pop_i32();
                    let actor = stack.pop_i32();

                    let Some(entity) = self.get_entity_mut(actor) else {
                        tracing::warn!(
                            "Script Error: ForceBattleDecision on illegal actor handle {actor}"
                        );
                        return 0;
                    };
                    if !entity.is_soldier() {
                        tracing::warn!(
                            "Script Error: ForceBattleDecision on non-soldier actor {actor}"
                        );
                        return 0;
                    }

                    let b_always = if decision_arg >= 100 {
                        decision_arg -= 100;
                        true
                    } else {
                        false
                    };

                    use crate::ai::Decision;
                    let decision = match decision_arg {
                        0 => Decision::Cassos,
                        1 => Decision::Fight,
                        2 => Decision::Observe,
                        3 => Decision::Reserve,
                        4 => Decision::AlertSoldiers,
                        5 => Decision::RunAndAlertSoldiers,
                        6 => Decision::Menace,
                        7 => Decision::Shoot,
                        8 => Decision::ArcherStepBack,
                        9 => Decision::LookForHelp,
                        10 => Decision::LookForHelpIfNobodyElseDoes,
                        11 => Decision::CoverBehindShieldBearer,
                        12 => Decision::TooProudToAttack,
                        13 => Decision::TowerGuardAlert,
                        14 => Decision::TowerGuardObserve,
                        15 => Decision::ArcherObserve,
                        16 => Decision::RunToArcheryPoint,
                        99 => Decision::None,
                        other => {
                            tracing::warn!(
                                "Script Error: Illegal identifier {other} for battle decision."
                            );
                            return 0;
                        }
                    };

                    if let Some(enemy) = entity.enemy_ai_mut() {
                        enemy.forced_next_battle_decision = decision;
                        enemy.reset_battle_decision = !b_always;
                    }
                    0
                }
                MakeNoise => {
                    // MakeNoise(Location, int id) -> 0
                    // `id` is a *script-local* selector —
                    // `SCRIPT_NOISE_LOGS = 0`,
                    // `SCRIPT_NOISE_DRAWBRIDGE = 1` — NOT the
                    // `NoiseType` enum value.  Error order: id check
                    // first, then NULL-location check.  (The original
                    // would plow on with an uninitialised noise type
                    // on a bad id; we drop the noise instead, which
                    // is strictly safer.)
                    let noise_id = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let noise_type = match noise_id {
                        0 => crate::ai::NoiseType::Logs,
                        1 => crate::ai::NoiseType::Drawbridge,
                        _ => {
                            tracing::error!("Script Error: Illegal noise ID {noise_id}");
                            return 0;
                        }
                    };
                    let Some((origin_x, origin_y)) = self.resolve_location_pos(loc) else {
                        tracing::error!("Script error : MakeNoise on NULL-location (handle {loc})");
                        return 0;
                    };
                    // Emit a deferred command so the engine runs the
                    // full `broadcast_noise` path (deafness, state
                    // filter, `AddNoiseToDisplay`), identical to the
                    // gameplay callsites.
                    let layer = self
                        .resolve_location_layer_sector(loc)
                        .map(|(l, _s)| l)
                        .unwrap_or(0);
                    self.commands.push(EngineCommand::MakeNoise {
                        noise_type,
                        x: origin_x,
                        y: origin_y,
                        layer,
                    });
                    tracing::debug!(
                        "MakeNoise: scripted {noise_type:?} at ({origin_x},{origin_y}) layer {layer}"
                    );
                    0
                }
                SetPathWalkingStyle => {
                    let style = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // ActorExists + IsNPC guards, both warn +
                    // early-return on miss.
                    if !self.actor_exists(actor) {
                        tracing::error!(
                            "Script Error: Trying to set path walking style of an invalid actor element."
                        );
                        return 0;
                    }
                    let Some(entity) = self.get_entity_mut(actor) else {
                        return 0;
                    };
                    if entity.ai_controller().is_none() {
                        tracing::error!(
                            "Script Error: Trying to set path walking style of a non-NPC."
                        );
                        return 0;
                    }
                    let Some(ai) = entity.ai_controller_mut() else {
                        return 0;
                    };
                    // The original switch only names WALKING(0)→0
                    // and RUNNING(1)→GOTO_RUN — cases 2/3
                    // (WALKING_NONINTERRUPTABLE /
                    // RUNNING_NONINTERRUPTABLE) leave the flags
                    // uninitialised (a bug).  Treat 0/2 as "clear
                    // RUN" and 1/3 as "insert RUN" to match the
                    // non-buggy intent.
                    if style & 1 == 1 {
                        ai.default_path_walking_flags.insert(GotoFlags::RUN);
                    } else {
                        ai.default_path_walking_flags.remove(GotoFlags::RUN);
                    }
                    // Relaunch on flag change: if the NPC is
                    // mid-patrol on a path waypoint segment, re-issue
                    // the GoTo so the new RUN/WALK flag takes effect
                    // mid-stride rather than waiting for the next
                    // waypoint.  The engine handler builds AiContext
                    // and looks up the current waypoint via the
                    // level's hiking_paths.
                    let needs_relaunch = ai.has_patrol_path
                        && matches!(
                            ai.current_substate,
                            crate::ai::Substate::DefaultGotoRoute
                                | crate::ai::Substate::DefaultEnroute
                        );
                    if needs_relaunch {
                        self.deferred_commands
                            .push(DeferredCommand::RelaunchPathAtNewSpeed { actor });
                    }
                    0
                }
                GetSoldierRank => {
                    let actor = stack.pop_i32();
                    self.get_entity(actor).map_or(0, |e| {
                        if let Some(soldier) = e.soldier_data() {
                            self.profile_manager
                                .get_soldier(soldier.soldier_profile_index)
                                .map_or(0, |p| p.rank as i32)
                        } else {
                            0
                        }
                    })
                }
                SwitchToAlertPath => {
                    // Gates on ActorExists + IsSoldier, then:
                    //   if (alert_path_id is some) {
                    //       changed_to_alert_path = true;
                    //       path.init(alert_path_id);
                    //       has_patrol_path = true;
                    //   }
                    //   if (state == Default) {
                    //       return_to_duty();
                    //   }
                    let actor = stack.pop_i32();
                    let hiking_paths = self.hiking_paths.clone();

                    let Some(entity) = self.get_entity(actor) else {
                        tracing::error!(
                            "Script Error: SwitchToAlertPath with invalid soldier ({actor})"
                        );
                        return 0;
                    };
                    if !entity.is_soldier() {
                        tracing::error!(
                            "Script Error: SwitchToAlertPath with non-soldier ({actor})"
                        );
                        return 0;
                    }

                    if let Some(entity) = self.get_entity_mut(actor) {
                        // Snapshot the values needed before splitting the
                        // mutable borrow between `ai_controller_mut` and
                        // `enemy_ai_mut`.
                        let alert_path_id = entity.ai_controller().and_then(|ai| ai.alert_path_id);
                        let in_default = entity
                            .ai_controller()
                            .is_some_and(|ai| ai.current_state == crate::ai::AiState::Default);

                        if let Some(alert_path_id) = alert_path_id {
                            // Init the patrol path and mark
                            // `has_patrol_path = true`.
                            if let Some(ai) = entity.ai_controller_mut() {
                                ai.path_id = Some(alert_path_id);
                                ai.patrol_path =
                                    crate::ai::PatrolPath::new(alert_path_id, &hiking_paths);
                                ai.has_patrol_path = ai.patrol_path.is_some();
                            }
                            // `changed_to_alert_path = true` — only set
                            // when an alert path was configured.
                            if let Some(enemy) = entity.enemy_ai_mut() {
                                enemy.changed_to_alert_path = true;
                            }
                        }

                        // ReturnToDuty fires regardless of whether
                        // an alert path was configured, as long as
                        // the soldier is in the Default state.
                        if in_default && let Some(ai) = entity.ai_controller_mut() {
                            ai.fire_self_stimulus(crate::ai::StimulusType::EventReturnToDuty);
                        }
                    }
                    0
                }
                SetNPCEmoticon => {
                    let duration = stack.pop_i32();
                    let emoticon_type = stack.pop_i32();
                    let actor = stack.pop_i32();
                    let frame = self.frame_counter;
                    let Some(entity) = self.get_entity_mut(actor) else {
                        tracing::warn!("Script Error: SetNPCEmoticon invalid actor {actor}");
                        return 0;
                    };
                    if !entity.is_npc() {
                        tracing::warn!("Script Error: SetNPCEmoticon target {actor} is not an NPC");
                        return 0;
                    }
                    let Ok(et) = EmoticonType::try_from(emoticon_type as u32) else {
                        tracing::warn!(
                            "Script Error: SetNPCEmoticon invalid emoticon id {emoticon_type}"
                        );
                        return 0;
                    };
                    if let Some(ai) = entity.ai_controller_mut() {
                        // NONE clears the expiration flag and
                        // ignores `duration`; otherwise the
                        // expiration is always written from
                        // `frame + duration` (u16 cast — negative
                        // wraps to a huge unsigned, zero expires
                        // next frame).
                        ai.current_emoticon_type = et;
                        if et == EmoticonType::None {
                            ai.emoticon_has_expiration_date = false;
                        } else {
                            ai.emoticon_has_expiration_date = true;
                            ai.emoticon_expiration_date = frame + (duration as u16) as u32;
                        }
                    }
                    0
                }
                ForbidNPCRemark => {
                    // ForbidNPCRemark(Actor, int remark_id, bool forbid)
                    // Adds or removes a remark ID from this NPC's forbidden list.
                    let forbid = stack.pop_i32();
                    let remark_id = stack.pop_i32();
                    let actor = stack.pop_i32();
                    if let Some(entity) = self.get_entity_mut(actor)
                        && let Some(ai) = entity.ai_controller_mut()
                    {
                        let id = remark_id as u32;
                        if forbid != 0 {
                            if !ai.forbidden_remark_ids.contains(&id) {
                                ai.forbidden_remark_ids.push(id);
                            }
                        } else {
                            ai.forbidden_remark_ids.retain(|&r| r != id);
                        }
                    }
                    0
                }
                DeclareAsCombatTrainer => {
                    // DeclareAsCombatTrainer(Actor) -> 0
                    // Two field sets on a soldier:
                    // `set_combat_trainer(true)` on the AI and
                    // `set_invulnerable(true)` on the human base
                    // (the damage/concussion pipeline reads the
                    // flag).
                    let actor = stack.pop_i32();
                    if let Some(entity) = self.get_entity_mut(actor) {
                        if let Some(enemy_ai) = entity.enemy_ai_mut() {
                            enemy_ai.combat_trainer = true;
                        } else {
                            tracing::warn!(
                                "DeclareAsCombatTrainer: actor {actor} is not a soldier"
                            );
                        }
                        if let Some(human) = entity.human_data_mut() {
                            human.invulnerable = true;
                        }
                    }
                    0
                }
                AddAsSubordinate => {
                    // Gates on eight conditions before mutating,
                    // then appends the subordinate to the chief's
                    // theoretical patrol (deduped) and triggers an
                    // `initialize_patrol` to rebuild the active
                    // patrol / missed-members lists and stamp the
                    // chief on every accepted minion.
                    let subordinate = stack.pop_i32();
                    let actor = stack.pop_i32();

                    // Guard 1: subordinate exists.
                    let Some(sub_entity) = self.get_entity(subordinate) else {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with invalid subordinate ({subordinate})"
                        );
                        return 0;
                    };
                    // Guard 2: subordinate is an NPC.
                    if !sub_entity.is_npc() {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with non-NPC subordinate ({subordinate})"
                        );
                        return 0;
                    }
                    // Guard 3: subordinate has no existing chief.
                    let sub_has_chief = sub_entity
                        .ai_controller()
                        .is_some_and(|ai| ai.patrol_chief.is_some());
                    if sub_has_chief {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with subordinate ({subordinate}) who already is in a patrol"
                        );
                        return 0;
                    }
                    // Guard 4: subordinate is not itself a chief
                    // (HasPatrol == !theoretical_patrol.is_empty()).
                    let sub_has_patrol = sub_entity
                        .ai_controller()
                        .is_some_and(|ai| !ai.theoretical_patrol.is_empty());
                    if sub_has_patrol {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with subordinate ({subordinate}) who is himself a patrol chief"
                        );
                        return 0;
                    }

                    // Guard 5: chief exists.
                    let Some(chief_entity) = self.get_entity(actor) else {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with invalid chief ({actor})"
                        );
                        return 0;
                    };
                    // Guard 6: chief is an NPC.
                    if !chief_entity.is_npc() {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with non-NPC chief ({actor})"
                        );
                        return 0;
                    }
                    // Guard 7: chief has no chief of its own.
                    let chief_has_chief = chief_entity
                        .ai_controller()
                        .is_some_and(|ai| ai.patrol_chief.is_some());
                    if chief_has_chief {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with chief ({actor}) who is himself in a patrol"
                        );
                        return 0;
                    }
                    // Guard 8: subordinate ≠ chief.
                    if subordinate == actor {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with subordinate ({subordinate}) == chief"
                        );
                        return 0;
                    }

                    let Some(sub_id) = self.actor_id(subordinate) else {
                        tracing::error!(
                            "Script Error: AddAsSubordinate with invalid subordinate handle {subordinate}"
                        );
                        return 0;
                    };
                    if let Some(entity) = self.get_entity_mut(actor)
                        && let Some(ai) = entity.ai_controller_mut()
                    {
                        // Dedup before pushing — same as the
                        // upstream `add_patrol_member` helper.
                        if !ai.theoretical_patrol.contains(&sub_id) {
                            ai.theoretical_patrol.push(sub_id);
                            // Force the chief's active patrol to be
                            // rebuilt on the next `tick_patrol_coordination`
                            // pass (engine/ai/mod.rs:5121).  The
                            // deferred pass rebuilds the active
                            // patrol lists and stamps the chief on
                            // every accepted member via
                            // `chief_assigns`.
                            ai.patrol.clear();
                            ai.missed_patrol_members.clear();
                            ai.needs_patrol_reinit = true;
                        }
                    }
                    0
                }
                RemoveAllSubordinates => {
                    // ClearPatrol body:
                    //   for each member of theoretical_patrol:
                    //     member.set_patrol_chief(None);
                    //     if state == Default: force_return_to_duty();
                    //   theoretical_patrol.clear();
                    //   missed_patrol_members.clear();
                    //   patrol.clear();
                    let actor = stack.pop_i32();
                    // Phase 1: snapshot the minion handles so we can free
                    // the chief's mutable borrow before iterating them.
                    let minion_ids: Vec<EntityId> = if let Some(entity) = self.get_entity(actor) {
                        entity
                            .ai_controller()
                            .map(|ai| ai.theoretical_patrol.clone())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    // Phase 2: clear each minion's `patrol_chief`,
                    // and for minions in the Default state, fire
                    // the EventReturnToDuty self-stimulus.  The
                    // `fire_self_stimulus` path queues a re-dispatch
                    // of the stimulus on the next think tick, which
                    // is the same end-state as the event-hook
                    // approach.
                    for minion_id in minion_ids {
                        let minion_actor = Self::actor_handle(minion_id);
                        if let Some(entity) = self.get_entity_mut(minion_actor)
                            && let Some(ai) = entity.ai_controller_mut()
                        {
                            ai.patrol_chief = None;
                            if ai.current_state == crate::ai::AiState::Default {
                                ai.fire_self_stimulus(crate::ai::StimulusType::EventReturnToDuty);
                            }
                        }
                    }
                    // Phase 3: clear the chief's three patrol lists via
                    // `clear_patrol()` (now also clears
                    // `missed_patrol_members`).
                    if let Some(entity) = self.get_entity_mut(actor)
                        && let Some(ai) = entity.ai_controller_mut()
                    {
                        ai.clear_patrol();
                    }
                    0
                }
                AddRepulsivePoint => {
                    // AddRepulsivePoint(Location, float radius, float action_radius, int flags) -> int
                    // Creates a repulsive point that NPCs avoid during pathfinding.
                    // Returns the auto-generated ID for the new point.
                    //
                    // Gates on `is_script_point(loc)` and warns +
                    // returns 0 for sector-typed locations.
                    //
                    // Repulsive points carry their script-point
                    // layer through `Position.level` so
                    // `gather_static_repulsive_points`'s layer
                    // filter compares against the authored layer.
                    let flags = stack.pop_i32();
                    let action_radius = f32::from_bits(stack.pop_i32() as u32);
                    let radius = f32::from_bits(stack.pop_i32() as u32);
                    let loc = stack.pop_i32();
                    if !self.is_script_point(loc) {
                        tracing::error!(
                            "Script Error: AddRepulsivePoint requires a point location (got handle {loc})"
                        );
                        return 0;
                    }
                    let Some((x, y)) = self.resolve_location_pos(loc) else {
                        tracing::error!(
                            "Script Error: AddRepulsivePoint cannot resolve location {loc}"
                        );
                        return 0;
                    };
                    let (level, sector_num) =
                        self.resolve_location_layer_sector(loc).unwrap_or((0, 0));
                    let position = crate::ai::Position {
                        x,
                        y,
                        level,
                        sector: crate::position_interface::SectorHandle::new(sector_num),
                    };
                    let id = self.ai_global.next_repulsive_point_id;
                    self.ai_global.next_repulsive_point_id += 1;
                    self.ai_global
                        .repulsive_points
                        .push(crate::ai::RepulsivePoint {
                            id,
                            position,
                            radius,
                            action_radius,
                            flags,
                        });
                    id
                }
                DeleteRepulsivePoint => {
                    // DeleteRepulsivePoint(int id) -> 0
                    // Removes a repulsive point by its ID.
                    let id = stack.pop_i32();
                    let before = self.ai_global.repulsive_points.len();
                    self.ai_global.repulsive_points.retain(|p| p.id != id);
                    if self.ai_global.repulsive_points.len() == before {
                        tracing::warn!("DeleteRepulsivePoint: no point with id {id}");
                    }
                    0
                }

                // --- animation / patch ---
                IsAnimationActive => {
                    let actor_h = stack.pop_i32();
                    if actor_h == 0 {
                        tracing::warn!("Script error: IsAnimationActive with null handle");
                        0
                    } else {
                        i32::from(self.entity_active.get(&actor_h).copied().unwrap_or(false))
                    }
                }
                SetAnimationState => {
                    // Rejects !ActorExists || !IsFX with a warning
                    // + false, else `set_active(state)` and true.
                    let state = stack.pop_i32();
                    let actor_h = stack.pop_i32();
                    let is_fx = self.get_entity(actor_h).is_some_and(|e| e.is_fx());
                    if !self.actor_exists(actor_h) || !is_fx {
                        tracing::error!(
                            "Script error (SetAnimationState): invalid animation handle {actor_h}"
                        );
                        0
                    } else {
                        let on = state != 0;
                        self.entity_active.insert(actor_h, on);
                        // Mirror onto the entity so intra-tick
                        // `IsActorActive` reads (which go through
                        // `is_active()` directly) reflect the write.
                        if let Some(entity) = self.get_entity_mut(actor_h) {
                            entity.element_data_mut().active = on;
                        }
                        1
                    }
                }
                IsPatchApplied => {
                    let h = stack.pop_i32();
                    self.get_patch(h).map_or(0, |p| i32::from(p.is_applied()))
                }
                ApplyPatch => {
                    let h = stack.pop_i32();
                    if let Some(patch_index) = Self::patch_index(h)
                        && let Some(patch) = self.patches.get_mut(patch_index)
                    {
                        let effects = patch.apply();
                        if !effects.is_empty()
                            && let Some(patch_index) =
                                crate::patch::PatchIndex::new(patch_index as u32)
                        {
                            self.deferred_commands
                                .push(DeferredCommand::ProcessPatchEffects {
                                    patch_index,
                                    effects,
                                });
                        }
                    }
                    1
                }
                ResetPatch => {
                    let h = stack.pop_i32();
                    if let Some(patch_index) = Self::patch_index(h)
                        && let Some(patch) = self.patches.get_mut(patch_index)
                    {
                        let effects = patch.force_reset();
                        if !effects.is_empty()
                            && let Some(patch_index) =
                                crate::patch::PatchIndex::new(patch_index as u32)
                        {
                            self.deferred_commands
                                .push(DeferredCommand::ProcessPatchEffects {
                                    patch_index,
                                    effects,
                                });
                        }
                    }
                    1
                }
                LockPatch => {
                    let val = stack.pop_i32();
                    let h = stack.pop_i32();
                    if let Some(patch) = self.get_patch_mut(h) {
                        if val != 0 {
                            patch.lock();
                        } else {
                            patch.unlock();
                        }
                    }
                    0
                }
                SetPatchAnimationActive => {
                    let active = stack.pop_i32();
                    let patch_h = stack.pop_i32();
                    // The patch's animation is an FX entity
                    // referenced by handle; flip its active flag.
                    let idx = Self::patch_index(patch_h);
                    if let Some(animation_h) =
                        idx.and_then(|i| self.patch_animation_entities.get(i).copied().flatten())
                    {
                        self.entity_active.insert(animation_h, active != 0);
                    }
                    // If no animation entity is mapped, this is a no-op (patch has no animation)
                    0
                }
                LinkTargetToFX => {
                    // Gates on `is_fx_target()` and `is_fx()` —
                    // either failing logs and skips the link.  The
                    // link is stored on the target's `linked_fx`
                    // array, which the focus-highlight code
                    // iterates to call a highlight-animation hook
                    // on each linked FX while the target is
                    // hovered.  Note: that highlight-animation
                    // setter is a no-op in shipped builds (the body
                    // is commented out), so the cascade is dead
                    // code in practice — but we mirror the storage
                    // so a future port of the highlight effect has
                    // somewhere to read.
                    let fx_h = stack.pop_i32();
                    let target_h = stack.pop_i32();
                    let fx_id = match self.actor_id(fx_h) {
                        Some(id) if self.get_entity(fx_h).is_some_and(|entity| entity.is_fx()) => {
                            id
                        }
                        None => {
                            tracing::warn!(
                                "Script error (LinkTargetToFX): null/invalid FX handle {fx_h}"
                            );
                            return 0;
                        }
                        Some(_) => {
                            tracing::warn!(
                                "Script error (LinkTargetToFX): handle {fx_h} is not an FX"
                            );
                            return 0;
                        }
                    };
                    let Some(fx_entity) = self.get_entity(fx_h) else {
                        tracing::warn!("Script error (LinkTargetToFX): invalid FX handle {fx_h}");
                        return 0;
                    };
                    if !fx_entity.is_fx() {
                        tracing::warn!(
                            "HALT STEHENBLEIBEN ! Script error (LinkTargetToPatch) : Invalid FX"
                        );
                        return 0;
                    }
                    let Some(target_entity) = self.get_entity_mut(target_h) else {
                        tracing::warn!(
                            "Script error (LinkTargetToFX): invalid target handle {target_h}"
                        );
                        return 0;
                    };
                    if !target_entity.is_fx_target() {
                        tracing::warn!(
                            "HALT STEHENBLEIBEN ! Script error (LinkTargetToPatch) : Invalid target"
                        );
                        return 0;
                    }
                    let Entity::Target(t) = target_entity else {
                        // is_fx_target() already gated this; unreachable.
                        return 0;
                    };
                    t.target.linked_fx.push(fx_id);
                    0
                }

                // --- sound ---
                SuspendAllSoundSources => {
                    self.sound_commands.push(SoundCommand::SuspendAll);
                    1
                }
                ResumeAllSoundSources => {
                    self.sound_commands.push(SoundCommand::ResumeAll);
                    1
                }
                ActivateSoundSource => {
                    let ss_h = stack.pop_i32();
                    if ss_h != 0 {
                        self.sound_commands.push(SoundCommand::Activate(ss_h));
                    }
                    1
                }
                DeactivateSoundSource => {
                    let ss_h = stack.pop_i32();
                    self.sound_commands.push(SoundCommand::Deactivate(ss_h));
                    1
                }
                DestroySoundSource => {
                    // Flip the liveness flag on `GameHost` eagerly
                    // so a same-call `GetSoundSourceScript(N)` sees
                    // the destroy.  The actual slot null-out still
                    // happens via the queued `SoundCommand::Destroy`
                    // after the script call.
                    let ss_h = stack.pop_i32();
                    if let Some(idx) = Self::sound_source_index(ss_h)
                        && let Some(alive) = self.sound_source_alive.get_mut(idx)
                    {
                        *alive = false;
                    }
                    self.sound_commands.push(SoundCommand::Destroy(ss_h));
                    1
                }

                // --- building / teleport ---
                CleanFromHisBuildingBeforeTeleport => {
                    let actor_h = stack.pop_i32();
                    // Remove actor from their current building's occupant list
                    if let Some(&bld_h) = self.actor_building.get(&actor_h) {
                        if let Some(idx) = Self::building_index(bld_h)
                            && let Some(occupants) = self.building_occupants.get_mut(idx)
                        {
                            occupants.retain(|&a| a != actor_h);
                        }
                        self.actor_building.remove(&actor_h);
                        1
                    } else {
                        tracing::warn!(
                            "Script error: CleanFromHisBuildingBeforeTeleport: \
                         actor {actor_h} not in a building"
                        );
                        0
                    }
                }
                CleanFromScriptZoneBeforeTeleport => {
                    let loc_h = stack.pop_i32();
                    let actor_h = stack.pop_i32();
                    if loc_h == 0 {
                        return 0;
                    }
                    if let Some(occupants) = self.zone_occupants.get_mut(&loc_h) {
                        if let Some(pos) = occupants.iter().position(|&a| a == actor_h) {
                            occupants.remove(pos);
                            1
                        } else {
                            tracing::warn!(
                                "Script error: CleanFromScriptZoneBeforeTeleport: \
                             actor {actor_h} not in zone {loc_h}"
                            );
                            0
                        }
                    } else {
                        tracing::warn!(
                            "Script error: CleanFromScriptZoneBeforeTeleport: \
                         invalid zone {loc_h}"
                        );
                        0
                    }
                }
                AddToScriptZoneAfterTeleport => {
                    let loc_h = stack.pop_i32();
                    let actor_h = stack.pop_i32();
                    if loc_h == 0 {
                        return 0;
                    }
                    self.zone_occupants.entry(loc_h).or_default().push(actor_h);
                    1
                }
                SetCorpseExistsInBuilding => {
                    let _actor = stack.pop_i32();
                    // Asserts false — "DESPERADOS STUFF", unused in
                    // this game.
                    0
                }
                PutActorInBuilding => {
                    let bld_h = stack.pop_i32();
                    let actor_h = stack.pop_i32();
                    if let Some(idx) = Self::building_index(bld_h) {
                        if idx >= self.building_occupants.len() {
                            self.building_occupants.resize(idx + 1, Vec::new());
                        }
                        self.building_occupants[idx].push(actor_h);
                        self.actor_building.insert(actor_h, bld_h);
                    }
                    // EngineInner applies positioning (inactive + special layer +
                    // building sector + gate point_in + DisableAllActionsTemp
                    // for PCs) after the script step.
                    self.deferred_commands
                        .push(DeferredCommand::PutActorInBuilding {
                            actor: actor_h,
                            building: bld_h,
                        });
                    0
                }
                SetBuildingActive => {
                    let val = stack.pop_i32();
                    let bld_h = stack.pop_i32();
                    let active = val != 0;
                    if let Some(idx) = Self::building_index(bld_h) {
                        if idx < self.building_active.len() {
                            self.building_active[idx] = active;
                        }
                        // Activate/deactivate all gates for this building
                        if let Some(gates) = self.building_gates.get(idx).cloned() {
                            for &gate_h in &gates {
                                if let Some(door) = self.get_door_mut(gate_h) {
                                    door.set_active(active);
                                }
                            }
                        }
                    }
                    0
                }
                GetAnyActorInsideBuilding => {
                    // The original declared the parameter as a
                    // building handle at the SCB API level but then
                    // cast it to a script-sector type and probed for
                    // OBJECT_SCRIPT_SECTOR.  Building sectors do not
                    // derive from script-objects, so the cast was UB
                    // and the type check almost always failed,
                    // routing real callers through an error path
                    // (effective return: 0).  We follow the declared
                    // API intent and query the building occupant
                    // list; if any mission script actually depended
                    // on the "always 0" behaviour, this would start
                    // returning real occupants.  No shipped SCB
                    // appears to rely on it.
                    let bld_h = stack.pop_i32();
                    Self::building_index(bld_h)
                        .and_then(|idx| self.building_occupants.get(idx))
                        .and_then(|occ| occ.first().copied())
                        .unwrap_or(0)
                }
                AreAllPCsInside => {
                    let loc_h = stack.pop_i32();
                    if loc_h == 0 {
                        return 0;
                    }
                    let all_inside = self.pc_handles.iter().all(|&pc| {
                        self.zone_occupants
                            .get(&loc_h)
                            .is_some_and(|occ| occ.contains(&pc))
                    });
                    i32::from(all_inside)
                }
                AreAllEnemiesInsideHS => {
                    // Returns false if any active Lacklandist
                    // soldier inside the zone is still alive,
                    // conscious, untied, and not carried.
                    let loc_h = stack.pop_i32();
                    if loc_h == 0 {
                        return 0;
                    }
                    let has_living_enemy =
                        self.zone_occupants.get(&loc_h).is_some_and(|occupants| {
                            occupants
                                .iter()
                                .any(|&handle| match self.get_entity(handle) {
                                    Some(Entity::Soldier(s)) => {
                                        s.element.active
                                            && s.soldier.cached_camp == Camp::Lacklandists
                                            && s.npc.life_points > 0
                                            && !s.human.unconscious
                                            && s.element.posture != Posture::Tied
                                            && s.human.carrier.is_none()
                                    }
                                    _ => false,
                                })
                        });
                    i32::from(!has_living_enemy)
                }
                AreAllPCsAliveInside => {
                    let loc_h = stack.pop_i32();
                    if loc_h == 0 {
                        return 0;
                    }
                    let all_alive_inside = self.pc_handles.iter().all(|&pc| {
                        // Dead PCs are exempt (the check is
                        // `!is_dead before is_inside`).  Use the
                        // life-points-based `is_dead` (= life_points
                        // <= 0) rather than the posture-derived
                        // check, which only flips true once the
                        // death animation has begun.
                        let is_dead = self.get_entity(pc).is_some_and(|e| e.is_dead());
                        if is_dead {
                            true
                        } else {
                            self.zone_occupants
                                .get(&loc_h)
                                .is_some_and(|occ| occ.contains(&pc))
                        }
                    });
                    i32::from(all_alive_inside)
                }

                // --- door ---
                IsDoorLockedPC => {
                    let h = stack.pop_i32();
                    self.get_door(h).map_or(0, |d| i32::from(d.is_locked_pc()))
                }
                IsDoorUnlockable => {
                    let h = stack.pop_i32();
                    self.get_door(h).map_or(0, |d| i32::from(d.is_unlockable()))
                }
                IsDoorLockedNPCCivilian => {
                    let h = stack.pop_i32();
                    self.get_door(h)
                        .map_or(0, |d| i32::from(d.is_locked_npc_civilian()))
                }
                IsDoorLockedNPCVillain => {
                    let h = stack.pop_i32();
                    self.get_door(h)
                        .map_or(0, |d| i32::from(d.is_locked_npc_villain()))
                }
                SetDoorLockedPC => {
                    let val = stack.pop_i32();
                    let h = stack.pop_i32();
                    if let Some(door) = self.get_door_mut(h) {
                        let locked = val != 0;
                        door.set_locked_pc(locked);
                        // Unlocking also activates the door.
                        if !locked {
                            door.set_active(true);
                        }
                    }
                    0
                }
                SetDoorUnlockable => {
                    let val = stack.pop_i32();
                    let h = stack.pop_i32();
                    if let Some(door) = self.get_door_mut(h) {
                        door.set_unlockable(val != 0);
                    }
                    0
                }
                SetDoorLockedNPCCivilian => {
                    let val = stack.pop_i32();
                    let h = stack.pop_i32();
                    if let Some(door) = self.get_door_mut(h) {
                        let locked = val != 0;
                        door.set_locked_npc_civilian(locked);
                        if !locked {
                            door.set_active(true);
                        }
                    }
                    0
                }
                SetDoorLockedNPCVillain => {
                    let val = stack.pop_i32();
                    let h = stack.pop_i32();
                    if let Some(door) = self.get_door_mut(h) {
                        let locked = val != 0;
                        door.set_locked_npc_villain(locked);
                        if !locked {
                            door.set_active(true);
                        }
                    }
                    0
                }
                SetDoorSpecialAutorisation => {
                    let direct = stack.pop_i32();
                    let actor_h = stack.pop_i32();
                    let door_h = stack.pop_i32();
                    let pc_bit = self.pc_auth_bits.get(&actor_h).copied().unwrap_or(0);
                    if let Some(door) = self.get_door_mut(door_h) {
                        door.grant_special_authorisation(pc_bit, direct != 0);
                    }
                    0
                }
                ActivateDoorMouseSector => {
                    let door_h = stack.pop_i32();
                    let active = stack.pop_i32();
                    if self.get_door(door_h).is_none() {
                        tracing::warn!(
                            "Script Error: ActivateDoorMouseSector: door {door_h} not found"
                        );
                        return 0;
                    }
                    // The clickable polygon lives in `fast_grid.sector_active`;
                    // queue an engine-side flip so script calls actually
                    // disable the click region.
                    self.commands.push(EngineCommand::ActivateDoorMouseSector {
                        door_handle: door_h,
                        active: active != 0,
                    });
                    0
                }

                // --- scroll ---
                ThisScroll => self.current_scroll,
                GetScrollStatus => {
                    // Null → warn + 0; non-object or non-scroll →
                    // "not a scroll" warn + 0; scroll → its status.
                    let scroll_h = stack.pop_i32();
                    if scroll_h == 0 {
                        tracing::warn!("Script Error: GetScrollStatus with null element");
                        0
                    } else {
                        let is_scroll = self
                            .get_entity(scroll_h)
                            .is_some_and(|e| e.kind() == ElementKind::ObjectScroll);
                        if !is_scroll {
                            tracing::warn!(
                                "Script Error: GetScrollStatus on non-scroll element {scroll_h}"
                            );
                            return 0;
                        }
                        self.scroll_status.get(&scroll_h).copied().unwrap_or(0)
                    }
                }
                SetScrollStatus => {
                    // Null/non-object/non-scroll → warn + return;
                    // status outside [0, MaxStatus) → warn + return.
                    // The setter stores the status, forces the
                    // BonusThree animation on Opened, and refreshes
                    // the minimap dot.
                    let status = stack.pop_i32();
                    let scroll_h = stack.pop_i32();
                    if scroll_h == 0 {
                        tracing::warn!("Script Error: SetScrollStatus with null element");
                        return 0;
                    }
                    let is_scroll = self
                        .get_entity(scroll_h)
                        .is_some_and(|e| e.kind() == ElementKind::ObjectScroll);
                    if !is_scroll {
                        tracing::warn!(
                            "Script Error: SetScrollStatus on non-scroll element {scroll_h}"
                        );
                        return 0;
                    }
                    if !(0..=3).contains(&status) {
                        tracing::warn!(
                            "Script Error: SetScrollStatus status {status} out of range (must be 0..=3)"
                        );
                        return 0;
                    }
                    self.commands.push(EngineCommand::SetScrollStatus {
                        scroll_handle: scroll_h,
                        status,
                    });
                    0
                }
                AttachScrollToNPC => {
                    // Four branches:
                    //   1. !ActorExists || !IsNPC -> warn + early return
                    //   2. scroll == NULL          -> attach NULL (detach)
                    //   3. !IsObject || GetObjectType() != SCROLL
                    //                              -> warn but FALL THROUGH (legacy bug)
                    //   4. valid                   -> attach scroll
                    // `attach_scroll` strips the previous SPEAK
                    // titbit and installs a fresh one whenever the
                    // attached scroll pointer differs (relevant for
                    // any titbit-index-bound consumer).
                    let scroll_h = stack.pop_i32();
                    let npc_h = stack.pop_i32();
                    // Branch 1: bad NPC handle.
                    let npc_is_npc = self.get_entity(npc_h).is_some_and(|e| e.is_npc());
                    if !npc_is_npc {
                        tracing::warn!(
                            "Script Error: AttachScrollToNPC with non-NPC actor handle {npc_h}"
                        );
                        return 0;
                    }
                    if scroll_h == 0 {
                        // Branch 2: detach.
                        if self.scroll_attachments.remove(&npc_h).is_some() {
                            self.scroll_attachment_dirty.insert(npc_h);
                        }
                    } else {
                        // Branch 3: log if not an object/scroll, but match the
                        // Match the legacy fall-through and still
                        // record the attachment.
                        let scroll_ok = self
                            .get_entity(scroll_h)
                            .is_some_and(|e| e.kind() == ElementKind::ObjectScroll);
                        if !scroll_ok {
                            tracing::warn!(
                                "Script Error: AttachScrollToNPC element {scroll_h} is not a scroll object"
                            );
                        }
                        // Branch 4: replace-or-insert; mark dirty when the value
                        // changes so the SPEAK titbit gets re-installed.
                        let prev = self.scroll_attachments.insert(npc_h, scroll_h);
                        if prev != Some(scroll_h) {
                            self.scroll_attachment_dirty.insert(npc_h);
                        }
                    }
                    0
                }

                // --- mission team ---
                GetPCFromMissionTeam => {
                    let idx = stack.pop_i32();
                    self.campaign.as_ref().map_or(0, |campaign| {
                        campaign
                            .mission_team_indices
                            .get(idx as usize)
                            .and_then(|&char_idx| campaign.characters.get(char_idx))
                            .and_then(|desc| desc.character_profile_idx)
                            .map_or(0, |pi| u32::from(pi) as i32)
                    })
                }
                AddPCToMissionTeam => {
                    let actor = stack.pop_i32();
                    // If the handle refers to a live entity, it
                    // must actually be a PC; non-PCs warn and skip
                    // the campaign update + mark.  In a Sherwood HUD
                    // context there's no live entity, so the
                    // `resolve_profile` fallback via raw profile
                    // index is the only signal available.
                    let entity_is_pc = self.get_entity(actor).map(|e| e.is_pc());
                    let mut added = false;
                    if entity_is_pc == Some(false) {
                        tracing::warn!("AddPCToMissionTeam: actor {actor} is not a PC");
                    } else {
                        let profile_idx = self.resolve_profile(actor);
                        if let Some(campaign) = self.campaign.as_mut() {
                            if let Some(pi) = profile_idx {
                                if let Some(char_idx) = campaign.get_character_by_profile(pi) {
                                    campaign.add_to_mission_team(char_idx);
                                    added = true;
                                }
                            } else {
                                tracing::warn!("AddPCToMissionTeam: cannot resolve actor {actor}");
                            }
                        }
                    }
                    // Mark only on the success branch.
                    if added {
                        self.commands.push(EngineCommand::MarkPc {
                            actor_handle: actor,
                        });
                    }
                    0
                }
                RemovePCFromMissionTeam => {
                    let actor = stack.pop_i32();
                    // Reject non-PC actors with a warning and skip
                    // the update.
                    let entity_is_pc = self.get_entity(actor).map(|e| e.is_pc());
                    if entity_is_pc == Some(false) {
                        tracing::warn!("RemovePCFromMissionTeam: actor {actor} is not a PC");
                    } else {
                        let profile_idx = self.resolve_profile(actor);
                        if let Some(campaign) = self.campaign.as_mut() {
                            if let Some(pi) = profile_idx {
                                if let Some(char_idx) = campaign.get_character_by_profile(pi) {
                                    campaign.remove_from_mission_team(char_idx);
                                }
                            } else {
                                tracing::warn!(
                                    "RemovePCFromMissionTeam: cannot resolve actor {actor}"
                                );
                            }
                        }
                    }
                    0
                }
                GetNumberOfObligatoryPCsInMissionTeam => {
                    self.campaign.as_ref().map_or(0, |campaign| {
                        let profiles = &self.profile_manager;
                        campaign
                            .next_mission_idx
                            .and_then(|mi| campaign.missions.get(mi))
                            .and_then(|m| m.profile_idx)
                            .and_then(|pi| profiles.missions.get(pi as usize))
                            .map_or(0, |mp| mp.required_character_indices.len() as i32)
                    })
                }
                GetObligatoryPCFromMissionTeam => {
                    let idx = stack.pop_i32();
                    // Returns a live PC actor handle (not a profile
                    // index) for the indexed required-character
                    // slot.  Resolve via the inverse of
                    // `pc_profile_map`.
                    let profile_manager = self.profile_manager.clone();
                    let required_profile: Option<u32> = self.campaign.as_ref().and_then(|c| {
                        c.next_mission_idx
                            .and_then(|mi| c.missions.get(mi))
                            .and_then(|m| m.profile_idx)
                            .and_then(|pi| profile_manager.missions.get(pi as usize))
                            .and_then(|mp| mp.required_character_indices.get(idx as usize))
                            .copied()
                    });
                    if let Some(char_profile_idx) = required_profile {
                        let needle = crate::profiles::CharacterProfileIdx(char_profile_idx);
                        if let Some(&handle) = self
                            .pc_profile_map
                            .iter()
                            .find_map(|(h, pi)| if *pi == needle { Some(h) } else { None })
                        {
                            handle
                        } else {
                            tracing::warn!(
                                "GetObligatoryPCFromMissionTeam: no live PC actor for profile {char_profile_idx}"
                            );
                            0
                        }
                    } else {
                        0
                    }
                }
                IsPCObligatoryInMissionTeam => {
                    let actor = stack.pop_i32();
                    let profile_idx = self.resolve_profile(actor);
                    self.campaign.as_ref().map_or(0, |campaign| {
                        let profiles = &self.profile_manager;
                        let Some(pi) = profile_idx else { return 0 };
                        let is_required = campaign
                            .next_mission_idx
                            .and_then(|mi| campaign.missions.get(mi))
                            .and_then(|m| m.profile_idx)
                            .and_then(|mpi| profiles.missions.get(mpi as usize))
                            .is_some_and(|mp| {
                                mp.required_character_indices.contains(&u32::from(pi))
                            });
                        if is_required { 1 } else { 0 }
                    })
                }
                IsMenToBlazonConversionMode => {
                    if self.men_to_blazon_conversion_mode {
                        1
                    } else {
                        0
                    }
                }

                // --- beam-me / spawning ---
                GetNumberOfBeamMes => {
                    self.campaign.as_ref().map_or(5, |campaign| {
                        let profiles = &self.profile_manager;
                        // Only valid from the Sherwood HQ mission.
                        let current_loc = campaign
                            .current_mission_idx
                            .and_then(|idx| campaign.missions.get(idx))
                            .and_then(|m| m.profile_idx)
                            .and_then(|pi| profiles.missions.get(pi as usize))
                            .map(|mp| mp.location);
                        if current_loc != Some(crate::profiles::MissionLocation::Sherwood) {
                            tracing::warn!(
                                "Script error: GetNumberOfBeamMes called from non-Sherwood mission"
                            );
                            return 0;
                        }
                        campaign
                            .next_mission_idx
                            .and_then(|idx| campaign.missions.get(idx))
                            .and_then(|m| m.profile_idx)
                            .and_then(|pi| profiles.missions.get(pi as usize))
                            .map_or(5, |mp| mp.number_of_beam_mes as i32)
                    })
                }
                MoveBeamMe => {
                    let loc = stack.pop_i32();
                    let idx = stack.pop_i32();
                    self.move_beam_me(idx, loc);
                    0
                }
                GetActorForBeamMe => {
                    let idx = stack.pop_i32();
                    self.get_actor_for_beam_me(idx)
                }

                // --- production / sector ---
                //
                // The engine drains these queues in
                // `apply_production_registrations` (engine/script.rs) —
                // it resolves each location handle to a script zone
                // sector, sets the sector's production type, and pushes
                // per-sector geometry into the campaign production
                // table.  Nothing to do here beyond queuing.
                RegisterAsProductionSector => {
                    let speed = stack.pop_i32();
                    let loc = stack.pop_i32();
                    let prod_type = stack.pop_i32();
                    self.production_registrations.push((prod_type, loc, speed));
                    0
                }
                AddProductionPoint => {
                    let loc = stack.pop_i32();
                    let prod_type = stack.pop_i32();
                    self.production_points.push((prod_type, loc));
                    0
                }
                GetNumberOfActorsInSector => {
                    // Warn when the handle is not a script-sector.
                    // Script-location handles are tagged indices laid
                    // out `[points..., sectors...]`; a sector payload
                    // index is in `[point_count, location_count)`.
                    let loc = stack.pop_i32();
                    if loc == 0 {
                        return 0;
                    }
                    if !self.is_script_sector_handle(loc) {
                        tracing::warn!(
                            "Script Error: GetNumberOfActorsInSector on non-sector handle {loc}"
                        );
                        return 0;
                    }
                    self.zone_occupants
                        .get(&loc)
                        .map_or(0, |occ| occ.len() as i32)
                }
                GetActorInSector => {
                    // Same sector-handle type guard as
                    // `GetNumberOfActorsInSector`.
                    let idx = stack.pop_i32();
                    let loc = stack.pop_i32();
                    if loc == 0 {
                        return 0;
                    }
                    if !self.is_script_sector_handle(loc) {
                        tracing::warn!("Script Error: GetActorInSector on non-sector handle {loc}");
                        return 0;
                    }
                    match self.zone_occupants.get(&loc) {
                        Some(occ) => {
                            if idx >= 0 && (idx as usize) < occ.len() {
                                occ[idx as usize]
                            } else {
                                tracing::warn!(
                                    "GetActorInSector: index {idx} out of range (max={})",
                                    occ.len()
                                );
                                0
                            }
                        }
                        None => 0,
                    }
                }

                // --- blazon / campaign ---
                WinBlazon => {
                    let actor = stack.pop_i32();
                    self.win_blazon(actor);
                    0
                }
                LoseBlazon => {
                    let actor = stack.pop_i32();
                    self.lose_blazon(actor);
                    0
                }
                IsBlazonWon => {
                    let actor = stack.pop_i32();
                    self.is_blazon_won(actor)
                }
                IsBonusItemPickedUp => {
                    let actor = stack.pop_i32();
                    self.is_bonus_item_picked_up(actor)
                }
                ConfiscateMoney => {
                    let actor = stack.pop_i32();
                    self.confiscate_money(actor);
                    0
                }
                AddPCToGang => {
                    let actor = stack.pop_i32();
                    let profile_idx = self.resolve_profile(actor);
                    let profiles = self.profile_manager.clone();
                    if let Some(campaign) = self.campaign.as_mut() {
                        if let Some(pi) = profile_idx {
                            if let Some(char_idx) = campaign.get_character_by_profile(pi) {
                                campaign.add_to_gang(char_idx, &profiles);
                                // Also calls `mission_stat.add_new_pc`
                                // — required for the post-mission
                                // stat screen and save data.
                                //
                                // The original passes the PC's
                                // `Status->wsName`, which by then has
                                // either the localized peasant name
                                // from `GenerateName` or the
                                // SPECIAL_PEASANT override stamped
                                // in by `SetPersistentProperty(NAME,
                                // …)`.  We haven't ported
                                // `generate_name`, and `name_override`
                                // resolves through `MenuTextLookup`
                                // at display time, so we capture the
                                // stable profile name as the
                                // fallback (matching the
                                // `mission_stat.remove_new_pc(profile_name)`
                                // key used by the kill cascade in
                                // `engine/melee.rs`) plus the
                                // override slot for the debriefing
                                // render to resolve.
                                if let Some(desc) = campaign.characters.get(char_idx) {
                                    let fallback = profiles
                                        .get_character(pi)
                                        .map(|cp| cp.profile_name.clone())
                                        .unwrap_or_default();
                                    let name_override = desc.status.name_override;
                                    self.mission_stat.add_new_pc(fallback, name_override);
                                }
                            }
                        } else {
                            tracing::warn!("AddPCToGang: cannot resolve actor {actor}");
                        }
                    }
                    0
                }
                AddFarmerToGang => {
                    let bow_exp = stack.pop_i32();
                    let sword_exp = stack.pop_i32();
                    let farmer_type = stack.pop_i32();
                    let profiles = self.profile_manager.clone();
                    if let Some(campaign) = self.campaign.as_mut() {
                        // 1-indexed script → 0-indexed profile.
                        let char_idx = campaign
                            .add_new_peasant_to_gang(Some((farmer_type - 1) as u16), &profiles);
                        if let Some(desc) = campaign.characters.get_mut(char_idx) {
                            desc.status.human_status.set_capacity(
                                crate::pc_status::SkillName::HandToHand,
                                sword_exp as u32,
                            );
                            desc.status
                                .human_status
                                .set_capacity(crate::pc_status::SkillName::Bow, bow_exp as u32);
                        }
                    }
                    0
                }
                SetExperiences => {
                    let bow_exp = stack.pop_i32();
                    let sword_exp = stack.pop_i32();
                    let actor = stack.pop_i32();
                    // The original has one backing status here, not separate live and
                    // persistent copies. `RHElementActorPC` is constructed with
                    // `&pDescription->PCStatus` (`RHelementactorpc.cpp`), and
                    // `RHElementActorHuman::SetCapacity` writes through that pointer
                    // (`RHelementactorhuman.cpp`). `RHCampaign::Serialize` then writes
                    // the same PC descriptions. Rust likewise keeps the PC status on
                    // the campaign description, so this actor-scoped call must update
                    // that serialized backing state.
                    //
                    // Validate the actor is a PC handle to surface
                    // script bugs that pass NPCs.
                    if !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                        tracing::warn!("Script error: SetExperiences passed non-PC actor {actor}");
                        return 0;
                    }
                    let profile_idx = self.resolve_profile(actor);
                    if let Some(campaign) = self.campaign.as_mut()
                        && let Some(pi) = profile_idx
                        && let Some(char_idx) = campaign.get_character_by_profile(pi)
                        && let Some(desc) = campaign.characters.get_mut(char_idx)
                    {
                        desc.status.human_status.set_capacity(
                            crate::pc_status::SkillName::HandToHand,
                            sword_exp as u32,
                        );
                        desc.status
                            .human_status
                            .set_capacity(crate::pc_status::SkillName::Bow, bow_exp as u32);
                    }
                    0
                }
                TransformHandleTargetToTakeTarget => {
                    let actor = stack.pop_i32();
                    self.transform_handle_target_to_take_target(actor);
                    0
                }

                // --- PC queries ---
                GetRobin => {
                    // Returns the first spawned PC where `is_robin`
                    // is true, else 0.  The result is a live Actor
                    // handle — never a profile index.
                    self.robin_handle
                }
                GetRelic => {
                    let idx = stack.pop_i32();
                    self.get_relic(idx)
                }
                GetPCType => {
                    let actor = stack.pop_i32();
                    // Gate on `ActorExists` and `IsPC()` before
                    // reading the profile, so passing a junk handle
                    // or a non-PC actor surfaces as a distinct
                    // warning rather than falling through to the
                    // "unknown filename" path.
                    if !self.actor_exists(actor) {
                        tracing::warn!(
                            "Script Error: Trying to get the PC type of an invalid actor!"
                        );
                        return -1;
                    }
                    if !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                        tracing::warn!("Script Error: Trying to get the PC type of a non-PC!");
                        return -1;
                    }
                    self.campaign.as_ref().expect("campaign required");
                    let profile_idx = self.resolve_profile(actor);
                    profile_idx
                        .and_then(|pi| self.profile_manager.get_character(pi))
                        .map_or(-1, |cp| {
                            match cp.filename.as_str() {
                                "RobinTown" | "RobinHood" => 0, // PC_TYPE_ROBIN
                                "LittleJohn" => 1,              // PC_TYPE_JOHN
                                "Friar Tuck" => 2,              // PC_TYPE_TUCK
                                "Stuteley" => 3,                // PC_TYPE_STUTELEY
                                "WillScarlet" => 4,             // PC_TYPE_SCARLET
                                "LadyMarian" => 5,              // PC_TYPE_MARIAN
                                "MerryManA" => 6,               // PC_TYPE_FARMER_A
                                "MerryManB" => 7,               // PC_TYPE_FARMER_B
                                "MerryManC" => 8,               // PC_TYPE_FARMER_C
                                _ => {
                                    tracing::warn!(
                                        "Script Error: PC with unknown type! (filename '{}')",
                                        cp.filename
                                    );
                                    -1
                                }
                            }
                        })
                }
                SelectActorPC => {
                    let select = stack.pop_i32();
                    let actor = stack.pop_i32();
                    self.deferred_commands.push(DeferredCommand::SelectPC {
                        actor,
                        select: select != 0,
                    });
                    0
                }
                IsPCSelected => {
                    let actor = stack.pop_i32();
                    // Returns true on validation failure (invalid
                    // actor or non-PC handle) after warning, so
                    // scripts that null-check the PC don't
                    // infinite-loop.
                    if !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                        tracing::warn!("Script Error: The Actor in IsPCSelected is invalid.");
                        return 1;
                    }
                    if self.selected_pc_handles.contains(&actor) {
                        1
                    } else {
                        0
                    }
                }
                GetNumberOfSelectedPCs => self.selected_pc_handles.len() as i32,
                GetSelectedPC => {
                    let idx = stack.pop_i32();
                    // Logs a warning when the index is out of range
                    // before returning NULL.  Treat negative indices
                    // as out-of-range too.
                    if idx < 0 || (idx as usize) >= self.selected_pc_handles.len() {
                        tracing::error!(
                            "Script Error: GetSelectedPC index {idx} out of range (count {})",
                            self.selected_pc_handles.len()
                        );
                        return 0;
                    }
                    self.selected_pc_handles
                        .get(idx as usize)
                        .copied()
                        .unwrap_or(0)
                }
                // ── Spellforge / Lua-only natives ──
                Reveal => {
                    // Spellforge name for "make this actor visible
                    // (un-blip)". Behaves like `UnBlip` but is the
                    // imperative-style name surfaced to Lua. Returns
                    // 1 iff the actor was previously blipped.
                    let actor = stack.pop_i32();
                    if !self.actor_exists(actor) {
                        tracing::warn!("Script Error: Reveal with invalid actor handle {actor}");
                        return 0;
                    }
                    let was_blipped = self
                        .get_entity(actor)
                        .is_some_and(|e| e.element_data().blipped);
                    if let Some(entity) = self.get_entity_mut(actor) {
                        entity.reveal_blip();
                    }
                    i32::from(was_blipped)
                }
                SequenceReveal => {
                    // Sequence-recorded variant of `Reveal`. Same
                    // semantics as `RecordUnBlip` — emits a `Unblip`
                    // sequence element at the current recording
                    // level.
                    let actor = stack.pop_i32();
                    if !self.is_actor_handle(actor) {
                        tracing::warn!("Script Error: SequenceReveal illegal actor handle {actor}");
                        return 0;
                    }
                    let level = self.recording_level();
                    let elem = SequenceElement::new(level, Command::Unblip, self.actor_id(actor));
                    self.record_element(elem)
                }
                IsActorOutOfAction => {
                    // Spellforge's English name for `IsActorHS`
                    // (Hors Service). Returns true iff the actor is
                    // dead, tied, or unconscious. Same semantics as
                    // `IsActorHS` — kept in lockstep to avoid drift
                    // if scripts mix the two names.
                    let actor = stack.pop_i32();
                    let Some(e) = self.get_entity(actor) else {
                        tracing::warn!(
                            "Script Error: IsActorOutOfAction with invalid actor handle {actor}"
                        );
                        return 0;
                    };
                    if !e.is_actor() {
                        tracing::warn!(
                            "Script Error: IsActorOutOfAction with non-actor handle {actor}"
                        );
                        return 0;
                    }
                    let posture = e.element_data().posture;
                    let dead = e.is_dead();
                    let tied = posture == Posture::Tied;
                    let unconscious = e.human_data().is_some_and(|h| h.unconscious);
                    i32::from(dead || tied || unconscious)
                }
                AddObjective => {
                    // Queues a UI objective for the host to surface
                    // in the objectives panel. Implementation is in
                    // the host (objectives panel doesn't exist on
                    // engine side); this just records the request.
                    // TODO: define the objectives UI on the host
                    // and consume `pending_objective_changes`.
                    let is_main = stack.pop_i32();
                    let id = stack.pop_i32();
                    self.pending_objective_changes.push(ObjectiveChange::Add {
                        id,
                        is_main: is_main != 0,
                    });
                    0
                }
                CompleteObjective => {
                    let id = stack.pop_i32();
                    self.pending_objective_changes
                        .push(ObjectiveChange::Complete { id });
                    0
                }
                SetPatrolShouldRun => {
                    // Toggles whether a patrolling NPC walks or
                    // runs along its assigned path. The patrol
                    // walk/run flag lives on the patrol
                    // descriptor; we stamp it via a deferred
                    // command so the engine reads a consistent
                    // value post-script.
                    // TODO: route into the patrol subsystem once
                    // the patrol walk/run flag is wired up.
                    let should_run = stack.pop_i32();
                    let actor = stack.pop_i32();
                    self.deferred_commands
                        .push(DeferredCommand::SetPatrolShouldRun {
                            actor,
                            should_run: should_run != 0,
                        });
                    0
                }
                ComputeLocationBetween => {
                    // Both args must be points; both must share
                    // layer and sector.  The result inherits pA's
                    // layer/sector.
                    let lambda_bits = stack.pop_i32();
                    let loc_b = stack.pop_i32();
                    let loc_a = stack.pop_i32();
                    let lambda = f32::from_bits(lambda_bits as u32);
                    if !self.is_script_point(loc_a) || !self.is_script_point(loc_b) {
                        tracing::error!(
                            "Script Error in ComputeLocationBetween: non-point handle(s) {loc_a}, {loc_b}"
                        );
                        return 0;
                    }
                    let layer_sector_a = self.resolve_location_layer_sector(loc_a);
                    let layer_sector_b = self.resolve_location_layer_sector(loc_b);
                    // If both sides resolve to a layer/sector, they
                    // must match.  If only one (or neither) resolves
                    // — e.g. computed locations that inherited no
                    // metadata — we accept the call and inherit
                    // whatever metadata is available; the source
                    // point just carries its own layer/sector
                    // forward.
                    if let (Some(a), Some(b)) = (layer_sector_a, layer_sector_b)
                        && a != b
                    {
                        tracing::error!(
                            "Script Error in ComputeLocationBetween: locations span different layers/sectors (a={a:?}, b={b:?})"
                        );
                        return 0;
                    }
                    match (
                        self.resolve_location_pos(loc_a),
                        self.resolve_location_pos(loc_b),
                    ) {
                        (Some(pos_a), Some(pos_b)) => {
                            let x = pos_a.0 + lambda * (pos_b.0 - pos_a.0);
                            let y = pos_a.1 + lambda * (pos_b.1 - pos_a.1);
                            // Inherit layer/sector from pA.  If pA
                            // is a computed location with no
                            // metadata, the result also has none.
                            self.create_computed_location_full(x, y, layer_sector_a)
                        }
                        _ => {
                            tracing::warn!(
                                "ComputeLocationBetween: invalid location handle(s) {loc_a}, {loc_b}"
                            );
                            0
                        }
                    }
                }
            }
        } else {
            // Index out of range (>= 265). We can't drain the stack
            // because we don't know the expected parameter count for
            // an unknown native. The VM state will be inconsistent but
            // a malformed SCB calling an unknown index is already a bug.
            tracing::error!("Unknown native function index {index}");
            0
        }
    }
}
