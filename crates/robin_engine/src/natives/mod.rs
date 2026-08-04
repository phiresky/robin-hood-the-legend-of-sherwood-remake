//! Native function registry for the Robin Hood scripting VM.
//!
//! The script VM core registers 265 host functions that scripts
//! call via `NativeCall <index>`. This module provides:
//!
//! 1. A name table mapping each index to its registered name (for logging
//!    and debugging).
//! 2. A `ScriptEffects` struct implementing `interp::HostFunctions` that
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
//!   - 195/196: GetCustomCampaignValue/SetCustomCampaignValue — canonical campaign slots
//!   - 197/198: GetCustomNPCValue/SetCustomNPCValue — canonical NPC slots
//!   - 206/207/208: BitwiseAnd/Or/Xor
//!   - 214: DeclareAsCombatTrainer — flag soldier as trainer
//!   - 221/222: IsActorRider, IsUnblipped — entity state checks
//!   - 224/227: AddRepulsivePoint/DeleteRepulsivePoint — NPC avoidance zones
//!   - 240: IsActorActive — entity active state
//!   - 252: MakePCCrouched, 259/260: GetActorActionState/SetActorActionState
//!   - 264: ForbidNPCRemark — suppress NPC remark categories

mod bindings;
mod commands;
mod context;
mod defs;
mod dispatch;
mod handle_codec;
mod signatures;
mod state;
#[cfg(test)]
mod tests;

pub use bindings::{AttachedScriptBindings, ScriptBindings, ScriptNameBindings};
pub use commands::{DeferredCommand, EngineCommand, ScriptCommandDomain, SoundCommand};
pub use context::{NativeContext, NativeSessionCapabilities, ScriptCallFrame};
pub use defs::{NativeFn, ORIGINAL_NATIVE_COUNT, RUST_EXTENSION_NATIVE_START, native_name};
pub use handle_codec::ScriptHandleCodec;
pub use signatures::{
    NATIVE_REGISTRY, NATIVE_SIGNATURES, NativeDefinition, NativeNamespace, NativeParamSig,
    NativeSignature, native_definition_by_index, native_definition_by_name,
    native_signature_by_index, native_signature_by_name,
};
pub use state::{ComputedScriptLocation, ScriptState, SequenceRecorderState};

use handle_codec::ScriptHandleKind;

// BTreeMap (not BTreeMap) so iteration order is deterministic across
// clients/processes — required for rollback multiplayer determinism.
use crate::ai::{AlertLevel, EmoticonType, GotoFlags};
use crate::coordinates::MapBBox;
use crate::element::{ActionState, Camp, Command, Entity, EntityId, Posture, TargetFilter};
use crate::element_kinds::ElementKind;
use crate::gate::Door;
use crate::interp::{
    HostFunctions, NativeCallOutcome, NativeOperation, NativeStack, NativeYield,
    NestedCallScriptThis, ResumePolicy, ScriptCallRequest,
};
use crate::order::OrderType;
use crate::patch::Patch;
use crate::profiles::Action;
use crate::sequence::{Field, FieldValue, MoveFlags, RecordingSession, Sequence, SequenceElement};

/// Convert a raw script-supplied animation ordinal to an
/// [`OrderType`].  Script-authored data should always be a valid
/// `OrderType` ordinal; if a script passes a value outside the enum
/// range, that's a data bug and we panic with context so it surfaces
/// immediately rather than silently corrupting the sequence element.
fn anim_ordinal_to_order_type(anim: i32, native: &str) -> OrderType {
    OrderType::try_from(anim as u32)
        .unwrap_or_else(|_| panic!("{native}: script passed invalid animation ordinal {anim}"))
}

/// One entry in the globally ordered script-effect stream. Domain typing is
/// retained in the enum rather than by separate queues: a later sound or
/// simulation barrier must never overtake an earlier command from another
/// domain.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum ScriptEffect {
    Presentation(EngineCommand),
    ExternalSound(SoundCommand),
    Simulation(SimulationEffect),
}

/// Wider-context deterministic mutations in the globally ordered stream.
/// Native-local mutations never enter this enum: only work that needs the
/// owning `EngineInner` boundary belongs here.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum SimulationEffect {
    Engine(EngineCommand),
    Deferred(DeferredCommand),
}

/// Serialized script output shell. This is an effect buffer, not a host and
/// not a world owner; deterministic state queried by natives lives in the
/// engine capabilities borrowed by [`NativeContext`].
#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScriptEffects {
    pub ordered: std::collections::VecDeque<ScriptEffect>,
}

/// Maximum allowed depth of nested script calls (e.g. one
/// `PrototypeFilterEvent` whose target itself calls
/// `PrototypeFilterEvent`). Beyond this, the sole engine driver reports an
/// error instead of fabricating a native result. Picked at 4 to absorb
/// realistic A → B → A → B chains without turning an accidental cycle into
/// unbounded host recursion.
pub const MAX_NESTED_CALL_DEPTH: u8 = 4;

impl ScriptEffects {
    pub fn new() -> Self {
        Self {
            ordered: std::collections::VecDeque::new(),
        }
    }

    pub fn emit_engine(&mut self, command: EngineCommand) {
        let effect = match command.domain() {
            ScriptCommandDomain::Presentation => ScriptEffect::Presentation(command),
            ScriptCommandDomain::SimulationBarrier => {
                ScriptEffect::Simulation(SimulationEffect::Engine(command))
            }
        };
        self.ordered.push_back(effect);
    }

    pub fn emit_sound(&mut self, command: SoundCommand) {
        self.ordered.push_back(ScriptEffect::ExternalSound(command));
    }

    pub fn emit_barrier(&mut self, command: DeferredCommand) {
        self.ordered
            .push_back(ScriptEffect::Simulation(SimulationEffect::Deferred(
                command,
            )));
    }

    pub fn pop_front(&mut self) -> Option<ScriptEffect> {
        self.ordered.pop_front()
    }

    pub fn take_tail(&mut self) -> std::collections::VecDeque<ScriptEffect> {
        std::mem::take(&mut self.ordered)
    }

    pub fn restore_tail(&mut self, tail: std::collections::VecDeque<ScriptEffect>) {
        self.ordered.extend(tail);
    }

    pub fn engine_commands(&self) -> Vec<EngineCommand> {
        self.ordered
            .iter()
            .filter_map(|effect| match effect {
                ScriptEffect::Presentation(command)
                | ScriptEffect::Simulation(SimulationEffect::Engine(command)) => {
                    Some(command.clone())
                }
                _ => None,
            })
            .collect()
    }

    pub fn sound_commands(&self) -> Vec<SoundCommand> {
        self.ordered
            .iter()
            .filter_map(|effect| match effect {
                ScriptEffect::ExternalSound(command) => Some(command.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn simulation_barriers(&self) -> Vec<DeferredCommand> {
        self.ordered
            .iter()
            .filter_map(|effect| match effect {
                ScriptEffect::Simulation(SimulationEffect::Deferred(command)) => {
                    Some(command.clone())
                }
                _ => None,
            })
            .collect()
    }
}

impl NativeContext<'_, '_> {
    /// Look up an entity by actor handle in the canonical Engine store.
    fn get_entity(&self, handle: i32) -> Option<&Entity> {
        let idx = Self::actor_handle_index(handle)?;
        self.entities
            .get_legacy_slot(idx as u32)
            .map(|(_, entity)| entity)
    }

    /// Look up an entity mutably by actor handle in the canonical Engine store.
    fn get_entity_mut(&mut self, handle: i32) -> Option<&mut Entity> {
        let idx = Self::actor_handle_index(handle)?;
        self.entities
            .get_legacy_slot_mut(idx as u32)
            .map(|(_, entity)| entity)
    }

    fn occupied_entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> + '_ {
        self.entities.occupied()
    }

    fn occupied_entities_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut Entity)> + '_ {
        self.entities.occupied_mut()
    }

    fn pc_handles(&self) -> Vec<i32> {
        self.occupied_entities()
            .filter_map(|(id, entity)| entity.is_pc().then_some(Self::actor_handle(id)))
            .collect()
    }

    fn pc_profile_index(&self, actor: i32) -> Option<crate::profiles::CharacterProfileIdx> {
        self.get_entity(actor)?.pc_data().map(|pc| pc.profile_index)
    }

    fn robin_handle(&self) -> i32 {
        self.occupied_entities()
            .find_map(|(id, entity)| {
                entity
                    .pc_data()
                    .is_some_and(|pc| pc.robin)
                    .then_some(Self::actor_handle(id))
            })
            .unwrap_or(0)
    }

    fn pc_authorisation_bit(&self, actor: i32) -> u16 {
        self.occupied_entities()
            .filter(|(_, entity)| entity.is_pc())
            .enumerate()
            .find_map(|(index, (id, _))| {
                (Self::actor_handle(id) == actor).then(|| {
                    1u16.checked_shl(index as u32).unwrap_or_else(|| {
                        panic!("PC authorization index {index} exceeds the 16-bit door mask")
                    })
                })
            })
            .unwrap_or(0)
    }

    /// `(any civilian dead, any soldier dead, max living soldier alert,
    /// max living civilian alert)` derived from the live NPC entities.
    fn npc_status_aggregates(&self) -> (bool, bool, i32, i32) {
        let mut result = (false, false, 0, 0);
        for (_, entity) in self
            .occupied_entities()
            .filter(|(_, entity)| entity.is_npc())
        {
            let dead = entity.is_dead();
            let alert = entity
                .ai_controller()
                .map(|ai| ai.current_music_alert_status as i32)
                .unwrap_or(0);
            if entity.is_soldier() {
                result.1 |= dead;
                if !dead {
                    result.2 = result.2.max(alert);
                }
            } else {
                result.0 |= dead;
                if !dead {
                    result.3 = result.3.max(alert);
                }
            }
        }
        result
    }

    fn actor_exists(&self, handle: i32) -> bool {
        self.get_entity(handle).is_some()
    }

    fn is_actor_handle(&self, handle: i32) -> bool {
        self.get_entity(handle)
            .is_some_and(|entity| entity.is_actor())
    }

    fn is_actor_or_fx_target(&self, handle: i32) -> bool {
        self.get_entity(handle)
            .is_some_and(|entity| entity.is_actor() || entity.is_fx_target())
    }

    fn actor_action_distance(&self, actor: i32, animation: OrderType) -> Option<f32> {
        let Some(entity) = self.get_entity(actor) else {
            tracing::warn!(
                actor,
                ?animation,
                "NativeContext::actor_action_distance: actor handle is missing"
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
                    "NativeContext::actor_action_distance: missing sprite action distance"
                );
                None
            }
        }
    }

    /// Mutate the canonical campaign value and mission statistics inline,
    /// matching `RHCampaign::AddValue` in the Original.
    fn add_campaign_value(
        &mut self,
        name: crate::campaign::CampaignValue,
        amount: i32,
        frame_counter: u32,
    ) {
        let campaign = self
            .campaign
            .as_mut()
            .expect("AddCampaignValue requires an active campaign");
        campaign.values[name] += amount;
        match name {
            crate::campaign::CampaignValue::Ransom => {
                self.mission_stat
                    .as_mut()
                    .expect("RANSOM campaign mutation requires live mission statistics")
                    .add_collected_money(amount);
                if amount > 0 && frame_counter > 0 {
                    self.emit_sound(SoundCommand::PlayJingle(crate::sound::Jingle::CashWon));
                }
            }
            crate::campaign::CampaignValue::Score => {
                self.mission_stat
                    .as_mut()
                    .expect("SCORE campaign mutation requires live mission statistics")
                    .add_score(amount);
            }
            _ => {}
        }
    }

    /// Force a value on the canonical campaign owner. The Original does not
    /// credit mission statistics for `SetValue`, but does emit the cash jingle
    /// for an increased ransom after frame zero.
    fn set_campaign_value(
        &mut self,
        name: crate::campaign::CampaignValue,
        value: i32,
        frame_counter: u32,
    ) {
        let campaign = self
            .campaign
            .as_mut()
            .expect("SetCampaignValue requires an active campaign");
        let old = campaign.values[name];
        campaign.values[name] = value;
        if name == crate::campaign::CampaignValue::Ransom && value > old && frame_counter > 0 {
            self.emit_sound(SoundCommand::PlayJingle(crate::sound::Jingle::CashWon));
        }
    }

    fn zone_index(&self, loc: i32) -> Option<usize> {
        ScriptHandleCodec::location_index(loc)?.checked_sub(self.bindings.script_point_count)
    }

    fn zone_occupant_handles(&self, loc: i32) -> Option<Vec<i32>> {
        let zone = self
            .zone_index(loc)
            .and_then(|idx| self.script_domains.zones.scripts.get(idx))?;
        Some(
            zone.occupant_indices
                .iter()
                .copied()
                .map(ScriptHandleCodec::actor_handle)
                .collect(),
        )
    }

    fn frame_counter(&self) -> u32 {
        *self
            .frame_counter
            .expect("script native requires a live simulation-clock query view")
    }

    fn selected_pc_handles(&self) -> Vec<i32> {
        self.selected_pcs
            .as_deref()
            .expect("script native requires a live player-selection query view")
            .iter()
            .copied()
            .map(Self::actor_handle)
            .collect()
    }

    /// Original `RHMessenger::ForwardMessage` applies PC selection before
    /// returning to the script VM. Keep that query-visible mutation on the
    /// canonical local-seat vector; the deferred command remains only for
    /// engine/sequence side effects that cannot run under this borrow set.
    fn apply_script_selection(&mut self, actor: i32, select: bool) {
        if actor == 0 {
            let selected = if select {
                let pc_ids: Vec<EntityId> = self
                    .entities
                    .pcs()
                    .map(|(id, _)| EntityId::Pc(id))
                    .collect();
                let mut selected = Vec::new();
                for id in pc_ids {
                    if !self.pc_is_selectable(id) {
                        continue;
                    }
                    let is_robin = matches!(
                        self.entities.get(id),
                        Some(Entity::Pc(pc)) if pc.pc.robin
                    );
                    if is_robin {
                        selected.insert(0, id);
                    } else {
                        selected.push(id);
                    }
                }
                selected
            } else {
                Vec::new()
            };
            *self
                .selected_pcs
                .as_deref_mut()
                .expect("script native requires a live player-selection query view") = selected;
            return;
        }

        let id = self
            .actor_id(actor)
            .expect("SelectActorPC validates the actor before applying selection");
        if select {
            if self.pc_is_selectable(id) {
                let selected = self
                    .selected_pcs
                    .as_deref_mut()
                    .expect("script native requires a live player-selection query view");
                selected.clear();
                selected.push(id);
            }
        } else {
            self.selected_pcs
                .as_deref_mut()
                .expect("script native requires a live player-selection query view")
                .retain(|&selected| selected != id);
        }
    }

    fn pc_is_selectable(&self, id: EntityId) -> bool {
        let Some(Entity::Pc(pc)) = self.entities.get(id) else {
            return false;
        };
        let posture = pc.element.posture;
        let in_coma = self
            .campaign
            .as_deref()
            .and_then(|campaign| campaign.characters.get(usize::from(pc.pc.list_index)))
            .filter(|description| description.character_profile_idx == Some(pc.pc.profile_index))
            .is_some_and(|description| description.status.in_coma);
        if pc.pc.life_points == 0
            || pc.human.unconscious
            || pc.human.stuck_under_nets_counter > 0
            || matches!(posture, Posture::Tied | Posture::Carried)
            || in_coma
            || !pc.pc.playable
        {
            return false;
        }

        let in_building = if pc.element.active {
            false
        } else {
            let position = pc.element.position_map();
            let point = crate::coordinates::MapPoint::new(position.x, position.y);
            matches!(
                self.fast_grid.get_sector(point, point, pc.element.layer()),
                crate::fast_find_grid::SectorHit::Found { sector_idx, .. }
                    if self
                        .fast_grid
                        .level
                        .sectors
                        .get(usize::from(sector_idx))
                        .is_some_and(|sector| sector.sector_type.is_building())
            )
        };
        if !pc.element.active && !in_building {
            return false;
        }

        let is_vip = self
            .bindings
            .profile_manager
            .get_character(pc.pc.profile_index)
            .is_some_and(|profile| profile.vip);
        !is_vip || !self.script_domains.mission_ui.men_to_blazon_conversion_mode
    }

    fn sound_source_count(&self) -> usize {
        self.sound_sources
            .as_ref()
            .expect("script native requires a live SoundSourceManager query view")
            .num_sources()
    }

    fn sound_source_alive(&self, index: usize) -> bool {
        if index >= self.sound_source_count() {
            return false;
        }
        self.sound_sources
            .as_ref()
            .expect("script native requires a live SoundSourceManager query view")
            .get(index)
            .is_some()
    }

    /// Launch a script-built sequence on the live manager before returning to
    /// the VM. `RHScript::Thanx` calls `LaunchSequence` inline; buffering the
    /// sequence until callback exit made later natives observe stale sequence
    /// state and assigned sequence IDs at the wrong command boundary.
    fn launch_script_sequence(&mut self, mut sequence: Sequence, native_return: i32) {
        for element in &mut sequence.elements {
            if element.priority == crate::sequence::SequencePriority::NotYetSet {
                element.priority = if element.executed_immediately() {
                    crate::sequence::SequencePriority::Normal
                } else {
                    match element.owner.and_then(|id| self.entities.get(id)) {
                        Some(entity) if entity.kind().is_actor() => {
                            crate::element_priority::determine_priority(
                                crate::element_priority::ActorPriorityContext {
                                    kind: entity.kind(),
                                    is_dead: entity.is_dead(),
                                    is_unconscious: entity
                                        .human_data()
                                        .is_some_and(|human| human.unconscious),
                                },
                                element,
                            )
                        }
                        _ => crate::sequence::SequencePriority::Normal,
                    }
                };
            }
        }
        let sequence_manager = self
            .sequence_manager
            .as_mut()
            .expect("script sequence launch requires a live SequenceManager");
        sequence_manager.launch_sequence(sequence);
        if let Some(action) = sequence_manager.pop_pending_immediate_action() {
            let continuation = sequence_manager.take_pending_synchronous_actions();
            self.pending_yield = Some(crate::interp::NativeYield {
                operation: crate::interp::NativeOperation::SequenceAction(
                    crate::interp::SynchronousSequenceOperation {
                        action,
                        continuation,
                    },
                ),
                resume: crate::interp::ResumePolicy::Fixed(native_return),
            });
        }
    }

    /// Whether this call may yield an engine-owned synchronous operation.
    ///
    /// Arguments use source order, not VM pop order. Lua uses this before
    /// invoking `HostFunctions`, so a rejected direct-host call cannot mutate
    /// the recorder, entities, or sequence manager first.
    pub fn requires_engine_driver(&self, native: NativeFn, args: &[i32]) -> bool {
        match native {
            NativeFn::Thanx
            | NativeFn::SendMessage
            | NativeFn::SendMessageWithArguments
            | NativeFn::PrototypeFilterEvent
            | NativeFn::SetActorPosture
            | NativeFn::SetActorLocation
            | NativeFn::SetActorActionState => true,
            NativeFn::SetAIState => {
                let Some((&actor, &state)) = args.first().zip(args.get(1)) else {
                    return false;
                };
                let Some(entity) = self.get_entity(actor).filter(|entity| entity.is_npc()) else {
                    return false;
                };
                match (state, entity) {
                    (1 | 5 | 7, Entity::Soldier(s)) => s.npc.ai_brain.enemy().is_some(),
                    (1 | 5 | 7, Entity::Civilian(c)) => c.npc.ai_brain.friendly().is_some(),
                    (3, Entity::Soldier(s)) => s.npc.ai_brain.enemy().is_some(),
                    _ => false,
                }
            }
            NativeFn::SetPersistentProperty => {
                matches!(args.get(1), Some(2 | 3))
                    && args.first().is_some_and(|actor| {
                        self.get_entity(*actor)
                            .is_some_and(|entity| entity.human_data().is_some())
                    })
            }
            NativeFn::InflictPain => args.first().is_some_and(|actor| self.actor_exists(*actor)),
            NativeFn::SetAlwaysAttentive => {
                let Some((&actor, &value)) = args.first().zip(args.get(1)) else {
                    return false;
                };
                value != 0
                    && self
                        .get_entity(actor)
                        .and_then(|entity| entity.enemy_ai())
                        .is_some_and(|enemy| !enemy.will_be_attentive)
            }
            NativeFn::EnableViewCone => {
                self.ai_global().ezekiel_2517
                    && args.first().is_some_and(|actor| {
                        self.get_entity(*actor)
                            .is_some_and(|entity| entity.human_data().is_some())
                    })
            }
            _ => false,
        }
    }

    fn current_animation(&self, actor: i32) -> Option<OrderType> {
        let entity = self.get_entity(actor)?;
        if let Some(object) = entity.object_data() {
            return Some(object.animation);
        }
        if !entity.is_actor() {
            return None;
        }
        Some(
            entity
                .actor_data()
                .expect("checked actor")
                .installed_order
                .map(|order| order.order_type)
                .unwrap_or(OrderType::NonanimationEnd),
        )
    }

    /// True iff `handle` resolves to a PC whose profile carries a corpse
    /// carrying action.
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
        self.bindings
            .profile_manager
            .get_character(pc.profile_index)
            .is_some_and(|profile| profile.can_carry())
    }

    // ── Recording helpers ───────────────────────────────────────

    /// Get the current recording command level, or 1 if not recording.
    fn recording_level(&self) -> u16 {
        self.script_state
            .sequence_recorder
            .recording
            .as_ref()
            .map_or(1, |r| r.command_level)
    }

    /// Convert a script actor handle to the live typed entity ID.
    /// 0 (null handle) or stale handles map to `None`.
    fn actor_id(&self, handle: i32) -> Option<EntityId> {
        let idx = ScriptHandleCodec::actor_handle_index(handle)?;
        self.entities.id_at_legacy_slot(idx as u32)
    }

    /// Resolve a script mobile-array index to the first masked FX child used
    /// as the Rust sequence owner. The original owner is the non-rendered
    /// RHElementMobile master; dispatch maps this child back to that master.
    fn mobile_owner_id(&self, mobile_index: i32) -> Option<EntityId> {
        let mobile_index = u16::try_from(mobile_index).ok()?;
        self.occupied_entities().find_map(|(id, entity)| {
            entity
                .as_fx()
                .is_some_and(|fx| fx.fx.mobile_index == Some(mobile_index))
                .then_some(id)
        })
    }

    /// Mobile masters are appended after the normal script-element array in
    /// C++. Rust stores only their masked child FX entities, created at the
    /// same boundary. Return that boundary so script indices remain those of
    /// the original arrays rather than leaking the child entity slot layout.
    fn standard_actor_script_count(&self) -> usize {
        self.entities
            .occupied()
            .find_map(|(id, entity)| {
                entity
                    .as_fx()
                    .is_some_and(|fx| fx.fx.mobile_index.is_some())
                    .then_some(id.index() as usize)
            })
            .unwrap_or(self.entities.len())
    }

    fn actor_script_index(&self, handle: i32) -> Option<usize> {
        let entity_index = Self::actor_handle_index(handle)?;
        let (_, entity) = self.entities.get_legacy_slot(entity_index as u32)?;
        if let Some(mobile_index) = entity.as_fx().and_then(|fx| fx.fx.mobile_index) {
            // The original does not add the normal-array length here: after
            // its first Find fails it returns the raw mobile-array index.
            Some(usize::from(mobile_index))
        } else {
            Some(entity_index)
        }
    }

    /// Add a sequence element to the current recording session.
    /// Returns 1 on success, 0 if not currently recording.
    fn record_element(&mut self, element: SequenceElement) -> i32 {
        if let Some(rec) = &mut self.script_state.sequence_recorder.recording {
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
        if !is_first && let Some(rec) = self.script_state.sequence_recorder.recording.as_mut() {
            rec.advance_level();
        }
        self.record_element(elem);
    }

    fn sector_kind(&self, sector: u16) -> Option<&crate::fast_find_grid::GridSector> {
        self.fast_grid
            .level
            .sectors
            .iter()
            .find(|candidate| u16::from(candidate.sector_number) == sector)
    }

    fn sector_is_building(&self, sector: u16) -> bool {
        self.sector_kind(sector)
            .is_some_and(|kind| kind.sector_type.is_building())
    }

    fn building_sector_is_authorized(&self, sector: crate::sector::SectorNumber) -> bool {
        let sector_data = self
            .sector_kind(u16::from(sector))
            .unwrap_or_else(|| panic!("building door references missing sector {sector}"));
        let occupant_count = if let Some(building_index) = sector_data.building_index {
            self.script_domains
                .buildings
                .occupants
                .get(usize::from(building_index.get()))
                .unwrap_or_else(|| {
                    panic!(
                        "building sector {sector} references missing building {}",
                        building_index.get()
                    )
                })
                .len()
        } else {
            // TODO(original-parity): attach every door-authored building
            // sector to canonical BuildingState during level loading. A few
            // loaded sectors lack the attachment; count their live actors by
            // sector rather than fabricating an empty building.
            self.occupied_entities()
                .filter(|(_, entity)| {
                    entity
                        .element_data()
                        .sector()
                        .is_some_and(|actor_sector| u16::from(actor_sector) == u16::from(sector))
                })
                .count()
        };
        occupant_count < usize::from(u16::MAX)
    }

    fn sector_is_ladder_lift(&self, sector: u16) -> bool {
        self.sector_kind(sector)
            .is_some_and(|kind| kind.lift_type == Some(crate::sector::LiftType::Ladder))
    }

    fn sector_lift_type(
        &self,
        sector: crate::sector::SectorNumber,
    ) -> Option<crate::sector::LiftType> {
        self.sector_kind(u16::from(sector))
            .and_then(|kind| kind.lift_type)
    }

    fn sector_is_door(&self, sector: u16) -> bool {
        self.sector_kind(sector)
            .is_some_and(|kind| kind.sector_type.is_door())
    }

    fn door_index_for_goal_sector(
        &self,
        goal_sector: u16,
        goal: (f32, f32),
    ) -> Option<crate::gate::DoorIndex> {
        self.script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
            .find_map(|(idx, door)| {
                let matches_endpoint =
                    door.sector_out == goal_sector || door.sector_in == goal_sector;
                let matches_click_sector = door.click_polygon_contains(goal.0, goal.1);
                (matches_endpoint || matches_click_sector)
                    .then_some(crate::gate::DoorIndex(idx as u32))
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
    /// trailing MOVE) are driven from script domains plus the canonical grid
    /// and entity owners borrowed by this native resume.
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

        if self.script_state.sequence_recorder.recording.is_none() {
            return false;
        }

        let owner = self.actor_id(actor_handle);
        let to_pt = |(x, y): (f32, f32)| crate::coordinates::MapPoint { x, y };

        // Original AppendMoveToSequence rewrites the source when the
        // actor is currently straddling a gate (`pTarget->GetDoor()`).
        // Do this before the same-sector fast path and path lookup so
        // recorded/script movement starts from the gate's far side.
        if let Some((door_handle, door_direction)) = self
            .get_entity(actor_handle)
            .map(crate::engine::current_door_for_route_source)
            && let Some((adapted_source, adapted_sector, adapted_layer)) =
                crate::engine::adapt_source_to_current_door(
                    &self.script_domains.interactables.doors,
                    door_handle,
                    door_direction,
                )
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
                        &self.script_domains.interactables.doors,
                        source,
                        source_sector,
                        door_idx,
                        auth.as_ref(),
                        allow_leave_map,
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| self.sector_lift_type(sector),
                    )
                })
        } else {
            find_path_gates(
                &self.script_domains.interactables.doors,
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
                self.emit_engine(EngineCommand::HeroSpeak {
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
            self.script_domains
                .interactables
                .doors
                .get(usize::from(step.door_index))
                .filter(|d| d.is_jump())
                .map(|_| i)
        });

        // Snapshot per-gate data into a local struct so the per-gate
        // emission loop can run without re-borrowing `self.script_domains.interactables.doors`.
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
                let door = self
                    .script_domains
                    .interactables
                    .doors
                    .get(usize::from(step.door_index))?;
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
            .script_state
            .sequence_recorder
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
                    .script_state
                    .sequence_recorder
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
                // Script recording receives the engine's explicit simulation
                // context, so this consumes the same deterministic stream as
                // runtime gate routing.
                let r: u32 = crate::sim_rng::u32(
                    self.simulation,
                    crate::sim_rng::RngSite::SequenceRecordingBuildingExitWait,
                    0..16,
                ) + crate::sim_rng::u32(
                    self.simulation,
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
                let mut jump_elem = SequenceElement::new_generic(0, Command::JumpCmd, owner);
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
                    .script_domains
                    .interactables
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

        let rec = self.script_state.sequence_recorder.recording.as_mut()?;
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
    /// (0) or RUNNING (1) with an error; we warn-log and let the caller
    /// short-circuit so scripts that pass a bogus style don't silently
    /// default to RUNNING.
    fn validate_style(style: i32, native_name: &str) -> bool {
        if style == 0 || style == 1 {
            true
        } else {
            tracing::warn!(
                "{native_name}: illegal movement style {style} (expected 0=WALKING or 1=RUNNING)"
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
    /// `inside` is assumed to be strictly inside the level map bounds;
    /// panics if no edge is reached (shouldn't happen for a valid
    /// inside point and non-zero direction vector).
    fn compute_border_point(&self, inside: (f32, f32), direction: i16) -> ((f32, f32), (f32, f32)) {
        compute_border_point_bbox(self.fast_grid.level.map_bbox, inside, direction)
    }
}

/// Compute the map-edge "border" point reached by walking from
/// `inside` in the opposite of `direction`, and an "outside" point
/// a small margin further so the actor's sprite box sits entirely
/// off the map. Kept standalone so level loading and native dispatch share the
/// same computation without involving an effect buffer.
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
    // Preserve the rounded second point used by
    // `SBGeoHalfLine2D(ptInside, ptInside - vDirection)`: Original's
    // intersection code promotes those two stored float points to double,
    // rather than promoting the unrounded direction vector directly.
    let (dx, dy) = crate::element::direction_vector_16(direction);
    let half_b = (inside.0 - dx, inside.1 - dy);
    let hx = half_b.0 - inside.0;

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

    // `SBGeoSegment2D ^ SBGeoHalfLine2D` delegates to the geometry
    // library's line intersection. It calculates line slopes/intercepts in
    // DOUBLE, casts the resulting point back to GEOTYPE/FLOAT, and tests the
    // four edges in top/right/bottom/left order. This arithmetic order is
    // authoritative: an all-f32 `t = delta / direction` formulation differs
    // by several ULPs for large map coordinates.
    let line_dx = f64::from(half_b.0) - f64::from(ix);
    let line_dy = f64::from(half_b.1) - f64::from(iy);
    let line_slope = (line_dx != 0.0).then(|| line_dy / line_dx);
    let line_intercept = line_slope.map(|slope| f64::from(iy) - f64::from(ix) * slope);

    let half_line_contains = |x: f32, y: f32| {
        if hx != 0.0 {
            if half_b.0 > ix { x >= ix } else { x <= ix }
        } else if half_b.1 > iy {
            y >= iy
        } else {
            y <= iy
        }
    };

    let horizontal_intersection = |y: f32| {
        let x = match line_slope {
            Some(0.0) => return None,
            Some(slope) => {
                let intercept = line_intercept.expect("non-vertical half-line has an intercept");
                // Horizontal edge: line1a=0, line1b=y in the Original
                // formula.
                ((intercept - f64::from(y)) / -slope) as f32
            }
            None => ix,
        };
        ((x_min..=x_max).contains(&x) && half_line_contains(x, y)).then_some((x, y))
    };

    let vertical_intersection = |x: f32| {
        let slope = line_slope?;
        let intercept = line_intercept.expect("non-vertical half-line has an intercept");
        let y = (slope * f64::from(x) + intercept) as f32;
        ((y_min..=y_max).contains(&y) && half_line_contains(x, y)).then_some((x, y))
    };

    // Preserve ComputeBorderPoint's segment order.
    for candidate in [
        horizontal_intersection(y_min),
        vertical_intersection(x_max),
        horizontal_intersection(y_max),
        vertical_intersection(x_min),
    ]
    .into_iter()
    .flatten()
    {
        try_edge(candidate.0, candidate.1);
    }

    let (_, bx, by) = best.expect("compute_border_point: no map-edge intersection");

    // Compute the outside point by stepping along the half-line in
    // 10-unit increments along the direction vector until a rough
    // sprite bounding box centred on the outside point no longer
    // touches the map box.  The sprite box `(-50, -70, 50, 20)` is a
    // conservative estimate of actor silhouette size.
    // `vShift = vDirection; vShift *= -10.f` uses the original direction
    // vector, not the rounded half-line delta above.
    let shift_x = -dx * 10.0;
    let shift_y = -dy * 10.0;
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

impl NativeContext<'_, '_> {
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
        let profile_manager = self.bindings.profile_manager.clone();
        if let Some(campaign) = self.campaign.as_mut() {
            campaign.add_value(crate::campaign::CampaignValue::Blazon, quantity);

            if let Some(idx) = campaign.current_mission_idx {
                let mission_type = campaign.missions[idx]
                    .profile(&profile_manager)
                    .mission_type;
                let current_blazons = campaign.get_value(crate::campaign::CampaignValue::Blazon);
                match mission_type {
                    crate::profiles::MissionType::Attack => {
                        // ATTACK missions win as soon as the collected
                        // total meets `number_of_blazons_to_win`.
                        let to_win = campaign.missions[idx]
                            .profile(&profile_manager)
                            .number_of_blazons_to_win;
                        if to_win as i32 <= current_blazons {
                            self.emit_engine(EngineCommand::Win { show_window: true });
                        }
                    }
                    crate::profiles::MissionType::Tactical => {
                        // The blazon mission caps what the player may
                        // *bring* into it, so tactical overflow past
                        // `win - to_be_collected` is clamped and the
                        // excess is flashed on the bar.
                        if let Some(bm_idx) = campaign.blazon_mission_idx {
                            let bp = campaign.missions[bm_idx].profile(&profile_manager);
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
            let frame_counter = self.frame_counter();
            self.script_domains
                .mission_ui
                .set_blinking_blazons(n, frame_counter);
        }

        // `UpdateInformationBars` / `UpdateBlazons` only fire in
        // campaign mode; in single-mission mode the update is skipped.
        if self.campaign.is_some() {
            self.emit_engine(EngineCommand::UpdateInformationBars);
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
                    self.emit_engine(EngineCommand::UpdateInformationBars);
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
            let frame_counter = self.frame_counter();
            self.add_campaign_value(crate::campaign::CampaignValue::Ransom, money, frame_counter);
        }
    }

    // ═══════════════════════════════════════════════════════════════
    //  Beam-me implementations
    // ═══════════════════════════════════════════════════════════════

    /// MoveBeamMe: relocate the PC at beam-me index `idx` to location `loc`.
    fn move_beam_me(&mut self, idx: i32, loc: i32) {
        // Find PC with matching beam_me_index
        let target_handle = self
            .pc_handles()
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
        self.pc_handles()
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
                            .bindings
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
                            .and_then(|campaign| {
                                let idx = usize::try_from(e.pc.campaign_description_index?).ok()?;
                                campaign.characters.get(idx)
                            })
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
                    .and_then(|campaign| {
                        let idx = usize::try_from(pc.campaign_description_index?).ok()?;
                        campaign.characters.get(idx)
                    })
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
            // 2: life points — the engine driver runs the complete setter
            // synchronously before the VM can execute its next opcode.
            2 => {
                match self.get_entity(actor) {
                    Some(entity) if entity.is_human() => {}
                    Some(_) => {
                        tracing::warn!(
                            "Script Error: SetPersistentProperty 'life points' on non-human"
                        );
                        return false;
                    }
                    None => {
                        tracing::warn!("Script Error: SetPersistentProperty invalid actor");
                        return false;
                    }
                }
                let request = crate::interp::SynchronousScriptRequest::SetPersistentLifePoints {
                    actor,
                    amount,
                    native_return: 1,
                };
                self.pending_yield = Some(crate::interp::NativeYield {
                    resume: crate::interp::ResumePolicy::Fixed(request.native_return()),
                    operation: crate::interp::NativeOperation::EngineAction(request),
                });
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
                let description_index = match self.get_entity(actor).and_then(|e| e.pc_data()) {
                    Some(pc) => pc.campaign_description_index,
                    None => return false,
                };
                if let Some(campaign) = self.campaign.as_mut()
                    && let Some(char_idx) =
                        description_index.and_then(|idx| usize::try_from(idx).ok())
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
            // 3: concussion — same full synchronous engine boundary.
            3 => {
                match self.get_entity(actor) {
                    Some(entity) if entity.is_human() => {}
                    Some(_) => {
                        tracing::warn!(
                            "Script Error: SetPersistentProperty 'concussion' on non-human"
                        );
                        return false;
                    }
                    None => {
                        tracing::warn!("Script Error: SetPersistentProperty invalid actor");
                        return false;
                    }
                }
                let request = crate::interp::SynchronousScriptRequest::SetPersistentConcussion {
                    actor,
                    amount,
                    native_return: 1,
                };
                self.pending_yield = Some(crate::interp::NativeYield {
                    resume: crate::interp::ResumePolicy::Fixed(request.native_return()),
                    operation: crate::interp::NativeOperation::EngineAction(request),
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
            let (profile_index, description_index) = match self.get_entity(actor) {
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
                        Some(pc) => (pc.profile_index, pc.campaign_description_index),
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
                                let Some(actor_index) = Self::actor_handle_index(actor) else {
                                    tracing::warn!(
                                        "Script Error: SetPersistentProperty invalid actor {actor}"
                                    );
                                    return false;
                                };
                                let Some((_, Entity::Soldier(s))) =
                                    self.entities.get_legacy_slot_mut(actor_index as u32)
                                else {
                                    panic!(
                                        "SetPersistentProperty: validated soldier handle {actor} no longer resolves to a soldier"
                                    );
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
                    .bindings
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
            let difficulty = self.simulation.config().difficulty;
            let Some(profile) = self.bindings.profile_manager.get_character(profile_index) else {
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
                if let Some(char_idx) = description_index.and_then(|idx| usize::try_from(idx).ok())
                {
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
        let Some(profile) = self.bindings.profile_manager.get_soldier(profile_index) else {
            tracing::warn!(
                ?profile_index,
                "soldier_has_bow_profile: missing soldier profile"
            );
            return false;
        };
        self.bindings
            .profile_manager
            .get_bow(profile.shooting_weapon_id)
            .is_some()
    }
}

impl Default for ScriptEffects {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeContext<'_, '_> {
    fn get_door(&self, handle: i32) -> Option<&Door> {
        ScriptHandleCodec::door_index(handle)
            .and_then(|idx| self.script_domains.interactables.doors.get(idx))
    }

    fn get_door_mut(&mut self, handle: i32) -> Option<&mut Door> {
        ScriptHandleCodec::door_index(handle)
            .and_then(|idx| self.script_domains.interactables.doors.get_mut(idx))
    }

    fn get_patch(&self, handle: i32) -> Option<&Patch> {
        ScriptHandleCodec::patch_index(handle)
            .and_then(|idx| self.script_domains.interactables.patches.get(idx))
    }

    fn get_patch_mut(&mut self, handle: i32) -> Option<&mut Patch> {
        ScriptHandleCodec::patch_index(handle)
            .and_then(|idx| self.script_domains.interactables.patches.get_mut(idx))
    }

    fn is_script_sector_handle(&self, loc: i32) -> bool {
        let Some(idx) = ScriptHandleCodec::location_index(loc) else {
            return false;
        };
        idx >= self.bindings.script_point_count && idx < self.bindings.script_location_count
    }

    /// Attach a production type to its canonical script sector and campaign
    /// owner before returning to the VM. RHScript.cpp calls
    /// `SectorProduction::AttachToScriptSector` directly; there is no
    /// post-Initialize registration phase in the Original.
    fn register_production_sector(&mut self, prod_type: i32, loc: i32, speed: i32) {
        let Some(kind) = crate::sector_production::Type::from_script_i32(prod_type) else {
            panic!("RegisterAsProductionSector: invalid production type {prod_type}");
        };
        let loc_idx = ScriptHandleCodec::location_index(loc)
            .unwrap_or_else(|| panic!("RegisterAsProductionSector: invalid location {loc}"));
        assert!(
            loc_idx >= self.bindings.script_point_count
                && loc_idx < self.bindings.script_location_count,
            "RegisterAsProductionSector: location {loc} is not a script sector"
        );
        let zone_idx = loc_idx - self.bindings.script_point_count;
        let zone = self
            .script_domains
            .zones
            .scripts
            .get_mut(zone_idx)
            .unwrap_or_else(|| {
                panic!("RegisterAsProductionSector: missing script zone {zone_idx}")
            });

        let campaign = self
            .campaign
            .as_mut()
            .expect("RegisterAsProductionSector requires the live Campaign");
        let production = campaign
            .production_sectors
            .get_mut(prod_type as usize)
            .unwrap_or_else(|| {
                panic!("RegisterAsProductionSector: missing campaign production type {prod_type}")
            });
        assert!(
            production.script_zone.is_none(),
            "RegisterAsProductionSector: production type {prod_type} is already attached to script zone {:?}",
            production.script_zone
        );

        // Original order inside AttachToScriptSector: attach/set speed, then
        // publish the type on the script sector.
        production.script_zone = Some(zone_idx);
        production.speed = speed as u16;
        production.prod_type = kind;
        zone.production_sector_type = kind;
    }

    /// Append a production point directly to the matching canonical campaign
    /// sector. The projection-area lookup is identical to
    /// `EngineInner::get_projection_area_index` and uses the already-attached
    /// immutable/runtime obstacle views.
    fn add_production_point(&mut self, prod_type: i32, loc: i32) {
        let Some(kind) = crate::sector_production::Type::from_script_i32(prod_type) else {
            panic!("AddProductionPoint: invalid production type {prod_type}");
        };
        assert!(
            self.is_script_point(loc),
            "AddProductionPoint: location {loc} is not a script point"
        );
        let (x, y) = self
            .resolve_location_pos(loc)
            .unwrap_or_else(|| panic!("AddProductionPoint: invalid location {loc}"));
        let (layer, sector) = self
            .resolve_location_layer_sector(loc)
            .unwrap_or_else(|| panic!("AddProductionPoint: location {loc} has no sector"));
        let point = crate::coordinates::MapPoint::new(x, y);
        let mut best: Option<(u16, f32)> = None;
        let obstacles = self
            .sight_obstacles
            .expect("AddProductionPoint requires live sight obstacles");
        for (index, obstacle) in obstacles.iter_indexed() {
            if !obstacle.is_projection_area()
                || obstacle.sector != sector
                || obstacle.layer != layer
                || !obstacle.box_projection.contains_point(point)
                || !obstacle.contains_point_projection(point)
            {
                continue;
            }
            let candidate = (index as u16, obstacle.box_3d_max[2]);
            if best.is_none_or(|(_, height)| candidate.1 > height) {
                best = Some(candidate);
            }
        }

        let campaign = self
            .campaign
            .as_mut()
            .expect("AddProductionPoint requires the live Campaign");
        let production = campaign
            .production_sectors
            .get_mut(prod_type as usize)
            .unwrap_or_else(|| {
                panic!("AddProductionPoint: missing campaign production type {prod_type}")
            });
        production.script_zone.unwrap_or_else(|| {
            panic!("AddProductionPoint: production type {prod_type} has no attached script sector")
        });
        production.prod_type = kind;
        production
            .production_points
            .push(crate::sector_production::Point {
                x,
                y,
                layer,
                sector,
                obstacle: best.map_or(0xFFFF, |(index, _)| index),
            });
    }

    fn resolve_profile(&self, actor: i32) -> Option<crate::profiles::CharacterProfileIdx> {
        self.pc_profile_index(actor).or_else(|| {
            self.campaign.as_ref()?;
            let idx = crate::profiles::CharacterProfileIdx(actor as u32);
            self.bindings
                .profile_manager
                .get_character(idx)
                .is_some()
                .then_some(idx)
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
        let Some(idx) = ScriptHandleCodec::location_index(handle) else {
            return false;
        };
        if idx < self.bindings.script_point_count {
            return true;
        }
        // Computed locations live past `script_location_count` and are
        // always points.
        idx >= self.bindings.script_location_count
            && (idx - self.bindings.script_location_count)
                < self.script_state.computed_locations.len()
    }

    /// Resolve a location handle to its (x, y) position.
    /// Handles 1..=script_location_count are static locations from level data.
    /// Handles beyond that are dynamically computed by script natives.
    fn resolve_location_pos(&self, handle: i32) -> Option<(f32, f32)> {
        let idx = ScriptHandleCodec::location_index(handle)?;
        if idx < self.bindings.script_location_count {
            self.bindings.location_positions.get(idx).copied()
        } else {
            let computed_idx = idx - self.bindings.script_location_count;
            self.script_state
                .computed_locations
                .get(computed_idx)
                .and_then(Option::as_ref)
                .map(|location| location.position)
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
        let idx = ScriptHandleCodec::location_index(handle)?;
        if idx < self.bindings.script_location_count {
            return Some((
                *self.bindings.location_layers.get(idx)?,
                *self.bindings.location_sectors.get(idx)?,
            ));
        }
        let computed_idx = idx - self.bindings.script_location_count;
        self.script_state
            .computed_locations
            .get(computed_idx)?
            .as_ref()
            .and_then(|location| location.layer.zip(location.sector))
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
        self.script_state
            .computed_locations
            .push(Some(ComputedScriptLocation {
                position: (x, y),
                layer: layer_sector.map(|(layer, _)| layer),
                sector: layer_sector.map(|(_, sector)| sector),
                active: true,
                legacy_dummy: false,
            }));
        ScriptHandleCodec::location_handle_from_index(
            self.bindings.script_location_count + self.script_state.computed_locations.len() - 1,
        )
    }

    /// Validate a 0-based script-object index and return its opaque script
    /// handle, or 0 with an error log if out of range. Common shape for
    /// the `GetXScript` family of natives (doors, patches, locations,
    /// sound sources, buildings, hiking paths).
    ///
    /// `-1` means "no script reference" and silently returns null.
    fn script_index_to_handle(idx: i32, count: usize, name: &str, kind: ScriptHandleKind) -> i32 {
        if idx == -1 {
            return 0;
        }
        if idx >= 0 && (idx as usize) < count {
            return ScriptHandleCodec::encode(kind, idx as usize);
        }
        tracing::error!("Script Error: invalid {name} ID {idx} (max={count})");
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
            Mobile(u16),
            General,
            Invalid,
        }

        let action = match self.get_entity(actor) {
            Some(entity) if entity.is_pc() => Action::Pc,
            Some(entity) => match entity.as_fx().and_then(|fx| fx.fx.mobile_index) {
                Some(index) => Action::Mobile(index),
                None => Action::General,
            },
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
                self.emit_barrier(DeferredCommand::SetPlayable {
                    actor,
                    playable: activate,
                });
                // On deactivate, also queue engine-side cleanup of QA
                // titbits and macro-store slots.
                if !activate {
                    self.emit_barrier(DeferredCommand::ClearAllQuickActionSlots { actor });
                }
            }
            Action::General => {
                // Soldiers, civilians, animals, objects, etc.
                if let Some(entity) = self.get_entity_mut(actor) {
                    entity.element_data_mut().active = activate;
                }
            }
            Action::Mobile(mobile_index) => {
                for (_, entity) in self.entities.occupied_mut() {
                    if entity
                        .as_fx()
                        .is_some_and(|fx| fx.fx.mobile_index == Some(mobile_index))
                    {
                        entity.element_data_mut().active = activate;
                    }
                }
                self.emit_engine(EngineCommand::SetMobileActive {
                    mobile_index,
                    active: activate,
                });
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

    /// Validate and yield the `LockAI` script native to the engine.
    ///
    /// Original `ScriptLockAI` sets the lock and calls `actor.Stop()` before
    /// the script executes its next native. The full stop path needs
    /// `EngineInner`, so the VM must pause here; merely queuing an AI halt
    /// would let a subsequent scripted sequence launch first and then be
    /// stopped instead of the outgoing movement.
    fn script_lock_ai(&mut self, actor: i32, remember_events: bool) {
        let Some(owner) = self.actor_id(actor) else {
            tracing::warn!("LockAI: invalid actor handle {actor}");
            return;
        };

        if !self
            .entities
            .get(owner)
            .expect("LockAI resolved actor disappeared during native dispatch")
            .is_npc()
        {
            tracing::warn!("LockAI: tried to lock the AI of a PC ({actor})");
            return;
        }

        self.pending_yield = Some(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::LockAi {
                    actor,
                    remember_events,
                    native_return: 0,
                },
            ),
            resume: crate::interp::ResumePolicy::Fixed(0),
        });
    }

    /// Common body for the `UnlockAI` script native.
    fn script_unlock_ai(&mut self, actor: i32) {
        let Some(owner) = self.actor_id(actor) else {
            tracing::warn!("UnlockAI: invalid actor handle {actor}");
            return;
        };

        let entity = self
            .entities
            .get(owner)
            .expect("UnlockAI resolved actor disappeared during native dispatch");

        if !entity.is_npc() {
            tracing::warn!("UnlockAI: tried to unlock the AI of a PC ({actor})");
            return;
        }

        // The native only reaches the AI when the actor is currently
        // script-locked; unlocking an already-unlocked NPC is a script
        // error and does nothing. (The authored `UnlockAi` sequence
        // command has no such guard and always runs the unlock.)
        let script_locked = entity
            .ai_controller()
            .expect("UnlockAI target validated as NPC must own an AI controller")
            .ai_is_script_locked();
        if !script_locked {
            tracing::warn!("UnlockAI: tried to unlock the AI of an NPC which is not locked");
            return;
        }

        self.pending_yield = Some(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::UnlockAi {
                    actor,
                    native_return: 0,
                },
            ),
            resume: crate::interp::ResumePolicy::Fixed(0),
        });
    }

    /// Common body for the `Freeze` script native. Original stores the
    /// command's value on both PC and NPC owners. The NPC latch is not
    /// consulted by Original's AI tick, but it remains serialized state.
    fn script_freeze_actor(&mut self, actor: i32, freeze: bool) {
        let Some(entity) = self.get_entity_mut(actor) else {
            tracing::warn!("Freeze: invalid actor handle {actor}");
            return;
        };

        if !entity.is_human() {
            tracing::warn!("Freeze: target {actor} is not human");
            return;
        }

        match entity {
            Entity::Pc(pc) => pc.pc.fried_psykokwack = freeze,
            Entity::Soldier(soldier) => soldier.npc.fried_pikachu = freeze,
            Entity::Civilian(civilian) => civilian.npc.fried_pikachu = freeze,
            _ => unreachable!("Freeze target was validated as human"),
        }
    }
}

impl HostFunctions for NativeContext<'_, '_> {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
        if index == NativeFn::PrototypeFilterEvent as u32 {
            // The VM must yield before the outer script can observe a return
            // value. MissionScript resolves this call synchronously and
            // writes the result into native_return_value before resuming at
            // Aff1NativeGetReturn.
            let i_event = stack.pop_i32();
            let actor_source = stack.pop_i32();
            let prototype = stack.pop_i32();
            return NativeCallOutcome::Yield(NativeYield {
                operation: NativeOperation::ScriptCall(ScriptCallRequest {
                    actor_handle: prototype,
                    fn_name: "FilterAIEvent".into(),
                    params: vec![actor_source, i_event],
                    script_this: NestedCallScriptThis::PreserveCaller,
                }),
                resume: ResumePolicy::OperationResult,
            });
        }

        let value = dispatch::call_immediate(self, index, stack);
        match self.pending_yield.take() {
            Some(request) => NativeCallOutcome::Yield(request),
            None => NativeCallOutcome::Return(value),
        }
    }
}
