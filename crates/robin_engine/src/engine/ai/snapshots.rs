//! Per-tick scratch types and snapshot-builder phases for `tick_enemy_ai`.
//!
//! Each phase here builds a read-only view (or, in P2, clears a flag) at
//! the top of the AI tick.  The orchestrator passes references to the
//! resulting Vec/Map into the per-NPC inner loops in [`super::detection`]
//! and [`super::post_detection`] so those passes can iterate the
//! snapshots without re-borrowing `self.entities`.

use super::*;
use crate::coordinates::MapPoint;
use crate::element::{Camp, Entity, EntityId};
use serde::{Deserialize, Serialize};

/// `RHArtificialMalignity::IsArcher` is exactly `GetBow() != NULL`.
/// A loaded bow remains a bow even when its normal-shot range is zero.
pub(super) fn is_archer_from_bow(bow: Option<&crate::profiles::BowProfile>) -> bool {
    bow.is_some()
}

// ── Per-tick scratch types for `tick_enemy_ai`. ─────────────────────
//
// These structs are private read-only views built once per detection
// tick and consumed by every per-NPC inner loop.  They were originally
// defined inline at the top of `tick_enemy_ai` but live at module scope
// now so the per-phase methods (extracted progressively in this module)
// can share them without nesting type definitions.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PcSnapshot {
    pub(super) id: EntityId,
    /// Raw `RHElement::IsActive()` analogue. Living inactive PCs stay in
    /// this snapshot because Enemy detectables are only cleaned up when the
    /// target dies, and an inactive PC in the viewer's building remains
    /// optically visible through the original same-sector short-circuit.
    pub(super) active: bool,
    /// Script-controlled PC participation flag. Non-playable PCs remain
    /// optical targets but cannot reveal an NPC blip through `SeesBlip`.
    pub(super) playable: bool,
    pub(super) position: MapPoint,
    /// PC viewer eye-point XY. Differs from feet position for
    /// LeaningOut and lying/dead postures, matching C++
    /// `ComputeEyesPoint` in `SeesBlip` / bonus discovery.
    pub(super) eye_position: MapPoint,
    pub(super) layer: u16,
    pub(super) posture: crate::element::Posture,
    pub(super) action_state: crate::element::ActionState,
    /// Currently-running animation (top of the order queue).
    /// Used by `pc_noise_volume` for the per-animation noise
    /// lookup during the produced-noise refresh.
    pub(super) order_type: crate::order::OrderType,
    /// `Some(sector_idx)` when the PC is currently inside a
    /// BUILDING sector.
    pub(super) building_sector: Option<crate::position_interface::SectorHandle>,
    /// Per-PC visual-detection-speed multiplier (percentage)
    /// from the character profile.  Used to scale visibility
    /// during detection refresh — a low-profile hero (e.g. a
    /// scout) is slower to spot, a loud hero faster.
    pub(super) detection_speed_in_forest: u16,
    /// Per-PC visual-detection-speed multiplier for city (non-
    /// forest) levels.
    pub(super) detection_speed_in_city: u16,
    // -- Combat-context fields (for FighterSnapshot) --
    pub(super) direction: u16,
    pub(super) able_to_fight: bool,
    pub(super) sword_range_default: u16,
    pub(super) sword_range_maximal: u16,
    pub(super) sword_range_uber: u16,
    pub(super) fighting_ability: u16,
    pub(super) in_recovery: bool,
    /// HtH weapon profile id, used at snapshot build time to clone
    /// the full profile into a FighterSnapshot for damage estimation.
    pub(super) hth_weapon_id: u32,
    /// VIP flag from character profile — VIPs are main heroes.
    pub(super) is_vip: bool,
    /// Whether this PC is Robin Hood.
    pub(super) is_robin: bool,
    /// Eye-point Z coordinate (elevation + posture offset).
    /// Used by the 3D sphere/cone detection check —
    /// this is the PC-as-viewer position.
    pub(super) eye_z: f32,
    /// Detection-point Z coordinate.  Used as the *target* side of
    /// a `VisibilityQuery` when an NPC is checking visibility of this
    /// PC.  Differs from `eye_z` in lying/carried postures
    /// (+2 vs +5; +25 vs default).
    pub(super) detection_z: f32,
    /// PC's current melee target for FighterSnapshot.
    pub(super) melee_target: Option<EntityId>,
    /// Active swordfight principal opponent, i.e. first entry of the
    /// human opponent list.
    pub(super) principal_opponent: u32,
    /// Active swordfight opponents in the same order as the live human
    /// opponent list.
    pub(super) opponent_handles: Vec<u32>,
    /// True when the PC is unconscious.  Unconscious PCs still
    /// flow through the detection pipeline (so NPCs can see
    /// bodies / sleeping heroes), but are split off from
    /// `list_them` into the `unconscious_enemies` tick-data
    /// list so the "approach sleeping enemy" branch in
    /// `battle_decisions` can pick them up.
    pub(super) unconscious: bool,
    /// True when the PC is being carried by another entity.
    /// Carried PCs are skipped from the sleeping-enemy
    /// list — you can't walk up and finish off someone slung
    /// over a buddy's shoulder.
    pub(super) carried: bool,
    /// True when the PC is mid-`Command::PassDoor` — i.e.
    /// `active_door_pass.is_some()`.  Used by visibility
    /// computation to short-circuit same-building sight to 0.0
    /// while the target is transitioning through a door.
    pub(super) passing_door: bool,
    /// Produced-noise volume for this PC this frame.  Computed
    /// once per PC during the produced-noise refresh, then
    /// sampled by every hearing NPC.  Caching on the snapshot
    /// avoids recomputing per (NPC, PC) pair and lets the
    /// shadow-stage carry-over work (`pc_noise_volume` reads
    /// the stored prev-frame value from `actor.last_noise_volume`).
    pub(super) noise_volume: u16,
    /// PC's current sector number (0 if unknown).  Fed into the
    /// `Noise.origin.sector` produced by the noise-refresh pass,
    /// which the hearing AI uses for sector-aware investigation
    /// pathing.
    pub(super) sector_num: u16,
    /// PC's ground elevation — note this is ground elevation,
    /// not the eye-point Z used for visual checks.
    pub(super) ground_elevation: u16,
    /// True when the PC has any active melee opponent
    /// (`opponents` list non-empty).  Used by hearing
    /// classification to mark the generated noise as ZINGZING
    /// (swordfight) or TAPTAPTAP (footsteps etc.).
    pub(super) is_swordfighting: bool,
    /// `pc.guard.is_some()`.  Set when an enemy soldier takes
    /// the PC into custody.  Used by predetection handling to
    /// suppress shadow events for already-guarded PCs.
    pub(super) guarded: bool,
    /// The projection-obstacle the PC is currently standing on
    /// (e.g. a roof, ledge, or tree platform), if any.
    /// Threaded into per-target `compute_view_radius` calls so
    /// detection radius accounts for the target's elevation in
    /// night/fog.
    pub(super) obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SoldierSnapshot {
    pub(super) id: EntityId,
    pub(super) position: MapPoint,
    pub(super) layer: u16,
    pub(super) camp: Camp,
    pub(super) ai_state: crate::ai::AiState,
    pub(super) ai_substate: crate::ai::Substate,
    pub(super) posture: crate::element::Posture,
    pub(super) rank: crate::profiles::ProfileRank,
    pub(super) company_number: u16,
    pub(super) pride: u16,
    pub(super) primary_target: u32,
    /// Active swordfight principal opponent, i.e. first entry of the
    /// human opponent list.
    pub(super) principal_opponent: u32,
    /// Active swordfight opponents in the same order as the live human
    /// opponent list.
    pub(super) opponent_handles: Vec<u32>,
    pub(super) able_to_fight: bool,
    pub(super) able_to_help: bool,
    /// Current music alert level.  Used by seek-area friend
    /// coordination to count friends in alert > Green.
    pub(super) alert_status: crate::ai::AlertLevel,
    /// Whether this soldier's seek has the
    /// `LOOK_FOR_HELP_AFTER_SEEKING` flag set. Used by seek-area
    /// friend coordination to clear our local flag if a friend is
    /// already going to ask for help.
    pub(super) seek_flag_look_for_help: bool,
    // -- Combat-context fields (for FighterSnapshot) --
    pub(super) direction: u16,
    pub(super) action_state: crate::element::ActionState,
    /// True when the soldier has any active melee opponent
    /// (`opponents` list non-empty).
    pub(super) is_swordfighting: bool,
    pub(super) sword_range_default: u16,
    pub(super) sword_range_maximal: u16,
    pub(super) sword_range_uber: u16,
    pub(super) fighting_ability: u16,
    pub(super) has_formation: bool,
    pub(super) is_shield_bearer: bool,
    pub(super) is_archer_unit: bool,
    pub(super) left_combat_neighbour: u32,
    pub(super) right_combat_neighbour: u32,
    pub(super) in_recovery: bool,
    /// HtH weapon profile id, used to clone the full profile into
    /// a FighterSnapshot for damage estimation.
    pub(super) hth_weapon_id: u32,
    /// VIP flag from soldier profile — VIPs can only attack Robin.
    pub(super) is_vip: bool,
    /// Tower guard flag from level data.
    pub(super) is_tower_guard: bool,
    /// AI's seek_position — where the soldier is heading. Used by
    /// `propose_good_combat_position` friend scoring.
    pub(super) seek_position: MapPoint,
    /// Handle of the shield bearer this archer is hiding behind (0 = none).
    pub(super) shield_bearer_before_me: u32,
    /// Handle of the archer hiding behind this shield bearer (0 = none).
    /// Derived from a reverse scan of `shield_bearer_before_me` links
    /// after all snapshots are built so the filter in
    /// `get_nearest_free_shield_bearer` is always consistent.
    pub(super) archer_behind_me: u32,
    /// Shield bearer facing direction (stored when running to phalanx).
    pub(super) shield_bearer_direction: u16,
    /// Bow max range from the bow profile.
    pub(super) bow_max_range: u16,
    /// Whether this soldier's AI is script-locked.
    pub(super) script_locked: bool,
    /// Reconnaissance report type from the soldier's AI brain.
    pub(super) report_type: crate::ai::ReportType,
    /// Seek position from the soldier's reconnaissance report.
    pub(super) report_seek_position: crate::ai::Position,
    /// Seen bodies from the soldier's reconnaissance report.
    pub(super) report_seen_bodies: Vec<u32>,
    /// Charly handle from the soldier's reconnaissance report.
    pub(super) report_charly: u32,
    /// The soldier's alert_soldiers_point.
    pub(super) alert_soldiers_point: crate::ai::Position,
    /// Ground-plane elevation (`element.position.z`).
    pub(super) elevation: u16,
    /// Exact render-space ground Z used by visibility geometry.
    pub(super) ground_z: f32,
    /// Soldier's patrol chief, if any.
    pub(super) patrol_chief: Option<EntityId>,
    /// Soldier's current antagonist handle.
    pub(super) antagonist: u32,
    /// Soldier profile duty flag — part of the
    /// "shall I stay on my post" decision.
    pub(super) duty_flag: bool,
    /// Whether this soldier is currently inside a building sector.
    /// Used by `alert_officer` to gate the layer-change penalty.
    pub(super) in_building: bool,
    /// Forecasted destination for this soldier (from
    /// `forecast_destination_for_ia`).  Used by `alert_officer`
    /// so the running soldier homes on where the officer will be,
    /// not where the officer is right now.
    pub(super) forecast_destination: crate::ai::Position,
    /// Body handles still on this soldier's body-detectable list —
    /// corpses they have *not yet* reacted to.  Mirror of the live
    /// `detectable_lists[DetectableType::Body]`.  Consumed by
    /// `near_officer_who_is_informed_about_this_body`.
    pub(super) detectable_bodies: Vec<u32>,
    /// Soldier's own AI seek position — distinct from
    /// `report_seek_position`.  Consumed by
    /// `near_officer_who_is_wondering_about_the_same_noise`.
    pub(super) ai_seek_position: crate::ai::Position,
    /// Live `current_task_priority` / `minimal_task_priority` —
    /// consumed by the officer's alert-soldiers gate to implement
    /// the "has the new task priority" check.
    pub(super) current_task_priority: u16,
    pub(super) minimal_task_priority: u16,
    /// View direction post-view-refresh — unit forward.
    /// Plumbed onto `CampSoldierInfo` for the officer-side cone+LOS
    /// gate in `maybe_officer_sees_me_fighting`'s ≥350² band.
    pub(super) view_direction: [f32; 2],
    /// View radius post-view-refresh.
    pub(super) view_radius: u16,
    /// View half-aperture post-view-refresh.
    pub(super) real_half_aperture: f32,
    /// Whether the soldier's eyes are blind (EYES_CLOSED /
    /// EYES_DIE_OR_GET_UNCONSCIOUS).
    pub(super) eye_blind: bool,
    /// Building containing this soldier, if any.
    pub(super) building_sector: Option<crate::position_interface::SectorHandle>,
    pub(super) is_rider: bool,
    pub(super) passing_door: bool,
    pub(super) obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
}

/// Per-tick read-only snapshot of a human-typed detection target —
/// shared across the `DetectableType::Body / Friend / MissedFriend /
/// Beggar` per-type passes since all four feed `compute_visibility`
/// against a human target.  Captures the per-target gate inputs each
/// kind needs (e.g. `able_to_help` for Friend, dead/unconscious flags
/// for MissedFriend / Beggar, `is_true_or_false_beggar` for the
/// Beggar cleanup-detectables predicate).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct HumanTarget {
    pub(super) position: MapPoint,
    pub(super) layer: u16,
    pub(super) eye_z: f32,
    pub(super) posture: crate::element::Posture,
    /// 16-sector facing.  Used for the `LeaningOut` arm of
    /// `compute_detection_point`: the detection point projects
    /// `direction × 40` forward.
    pub(super) direction: i16,
    pub(super) action_state: crate::element::ActionState,
    pub(super) building_sector: Option<crate::position_interface::SectorHandle>,
    pub(super) unconscious: bool,
    pub(super) active: bool,
    pub(super) is_pc: bool,
    /// `is_able_to_help`: alive, conscious, not in a few
    /// state-machine arms that mean "busy with current task".
    /// Used to gate the Friend pass.
    pub(super) able_to_help: bool,
    /// True for a civilian whose profile flags it as a beggar OR
    /// a PC currently in `Posture::SimulatingBeggar`.  Used by
    /// the cleanup-detectables Beggar predicate — entries lose
    /// detectability the moment the target stops being a beggar.
    pub(super) is_true_or_false_beggar: bool,
    /// Whether the target is mid-door-pass.  Used by the
    /// same-building visibility short-circuit.
    pub(super) passing_door: bool,
    /// `pc.guard.is_some()`.  Only meaningful for PCs (false for
    /// soldiers / civilians / non-PC entities).  Used by
    /// predetection handling to suppress shadow events for
    /// already-guarded PCs.
    pub(super) guarded: bool,
    /// The projection-obstacle this human is currently standing
    /// on (e.g. a roof, ledge, balcony, or tree platform).
    /// Threaded into the per-target `compute_view_radius` re-call
    /// inside `run_human_detectable_pass` so detection radius
    /// accounts for the target's elevation in night/fog.
    pub(super) obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
}

/// Per-tick read-only snapshot of an object target — anything that may
/// appear in an NPC's `DetectableType::Object` list (coins, ales,
/// money bags, etc.).  Captures the data the object-visibility
/// computation reads.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct ObjectTarget {
    pub(super) position: MapPoint,
    pub(super) layer: u16,
    pub(super) belongs_to_beggar: bool,
    pub(super) active: bool,
}

/// Immutable start-of-tick PC and combat view consumed by AI phases.
///
/// The original engine exposes stable live pointers throughout one actor's
/// `RHElementActorNPC::RefreshDetection` call. PC data is captured once because
/// building it also updates produced-noise state. Volatile NPC human/object
/// target data is intentionally rebuilt at each NPC creation slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AiWorldView {
    pub(super) pcs: Vec<PcSnapshot>,
    pub(super) pc_forecasts: std::collections::HashMap<u32, crate::ai::ForecastedDestination>,
    pub(super) primary_target_multiplicity: std::collections::BTreeMap<EntityId, u32>,
    pub(super) npc_jump_lines: std::collections::HashMap<EntityId, Option<u32>>,
    pub(super) soldiers: Vec<SoldierSnapshot>,
    pub(super) ko_money_fight_soldiers: Vec<(EntityId, Camp)>,
}

impl EngineInner {
    /// Build the single immutable AI world view for this tick.
    ///
    /// Original provenance: `RHelementactorsoldier.cpp::Hourglass` performs
    /// `AttackingReactiontimeEnemyNearTest` before delegating to
    /// `RHElementActorNPC::Hourglass`; the latter runs `RefreshView` then
    /// `RefreshDetection`. This method does not invoke detection or Think; the
    /// orchestrator below retains that behavioral phase order when consuming
    /// the captured view.
    pub(super) fn tick_enemy_ai_build_world_view(&mut self, assets: &LevelAssets) -> AiWorldView {
        let pcs = self.tick_enemy_ai_build_pc_snapshots(assets);
        let pc_forecasts = self.tick_enemy_ai_build_pc_forecasts();
        let primary_target_multiplicity = self.tick_enemy_ai_build_primary_target_multiplicity();
        let npc_jump_lines = self.tick_enemy_ai_build_jump_lines(assets);
        let soldiers = self.tick_enemy_ai_build_soldier_snapshots(assets);
        let ko_money_fight_soldiers = self.tick_enemy_ai_build_ko_money_fight_soldiers();
        AiWorldView {
            pcs,
            pc_forecasts,
            primary_target_multiplicity,
            npc_jump_lines,
            soldiers,
            ko_money_fight_soldiers,
        }
    }

    /// P1 — snapshot every alive PC for the per-tick detection pass.
    ///
    /// Computes the per-frame produced-noise volume plus the
    /// eye-point / weapon-range lookups the per-NPC detection
    /// refresh needs.  The freshly computed `noise_volume` is also
    /// written back onto each PC so the next frame's shadow-stage
    /// walk/run carry-over reads the correct prior value.
    pub(super) fn tick_enemy_ai_build_pc_snapshots(
        &mut self,
        assets: &LevelAssets,
    ) -> Vec<PcSnapshot> {
        use crate::element::Posture;

        let mut pc_snapshots: Vec<PcSnapshot> = Vec::with_capacity(self.pc_ids.len());
        for &pc_id in &self.pc_ids {
            let Some(Entity::Pc(pc)) = self.entities.get(pc_id) else {
                continue;
            };
            // `is_able_to_fight` requires alive, but unconscious PCs are
            // still detectable — they drop out as unable-to-fight in the
            // cleanup path of `battle_decisions`, not here.  Filtering them
            // at snapshot-build time would make NPCs blind to sleeping
            // heroes entirely, breaking the "approach sleeping enemy" +
            // "kill nearby sleeping enemies" branches.
            if pc.pc.life_points <= 0 {
                continue;
            }
            let element_active = pc.element.active;
            let is_unconscious = pc.human.unconscious;
            let is_carried = pc.human.carrier.is_some();
            let is_passing_door = pc.actor.active_door_pass.is_some();
            // Eye-point XY: shift the eye 40 units forward along the
            // facing vector for LeaningOut.  Every other posture uses
            // the feet position — the Z offset is layered on below.
            let pos = {
                let mut p = pc.element.position_map();
                if pc.element.posture == Posture::LeaningOut {
                    let (dx, dy) = crate::element::direction_vector_16(pc.element.direction());
                    p.x += 40.0 * dx;
                    p.y += 40.0 * dy;
                }
                p
            };
            let layer = pc.element.layer();
            let building_sector = self.entity_building_sector(pc.element.sector());
            let material = pc.element.sprite.position_iface.get_material();
            // PCs reaching this point are alive.  Unconscious PCs are
            // *not* able to fight — FighterSnapshots and combat
            // scoring should treat them as non-combatants.
            let alive = !is_unconscious;
            // Look up the PC's HtH weapon profile for combat ranges and
            // fighting ability.
            // Original: `RHProfileManager.h::GetCharacterProfile` asserts the
            // profile index and `RHelementactorpc.cpp::Initialize` immediately
            // initializes weapons from it. A live PC cannot have a synthetic
            // profile or weapon.
            let character = assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {} requires missing character profile {}",
                        pc_id.index(),
                        u32::from(pc.pc.profile_index)
                    )
                });
            let hth_weapon_id = character.hth_weapon_id;
            let fighting_ability = character.fighting;
            // Per-PC detection-speed multipliers — scale the
            // visibility of this PC when an enemy NPC refreshes its
            // detection. They come from the required character profile
            // above; missing profile data is an initialization error.
            let detection_speed_in_forest = character.detection_speed_in_forest;
            let detection_speed_in_city = character.detection_speed_in_city;
            // Currently-running animation — front order of the PC's
            // current in-progress sequence element.  `Invalid` is the
            // enum default and behaves as "no animation running"
            // (silent) in the noise lookup table.
            let order_type = self
                .sequence_manager
                .current_order_for_actor(pc_id)
                .map(|(_, _, o)| o.order_type)
                .unwrap_or(crate::order::OrderType::Invalid);
            let weapon = assets
                .profile_manager
                .get_hth_weapon(hth_weapon_id)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {} requires missing HtH weapon profile {}",
                        pc_id.index(),
                        hth_weapon_id
                    )
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                weapon.distance[crate::weapons::WeaponDistance::Default as usize],
                weapon.distance[crate::weapons::WeaponDistance::Maximal as usize],
                weapon.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            // Honour check: a man of honour does not strike an
            // opponent in any of these animations.
            let in_recovery = !alive
                || matches!(
                    pc.actor.old_action,
                    crate::order::OrderType::BeingHitSword
                        | crate::order::OrderType::ExtractingArrowSword
                        | crate::order::OrderType::DyingSword
                        | crate::order::OrderType::BeingDeadSword
                        | crate::order::OrderType::FallingBackSword
                        | crate::order::OrderType::BeingUnconsciousSword
                        | crate::order::OrderType::BeingDeadFallenBackSword
                        | crate::order::OrderType::StandingUpSword
                );
            // Compute eye-point Z for the 3D blip-sight check (PC as
            // viewer) and detection-point Z for `VisibilityQuery`
            // (PC as target).  PCs are never riders in the current
            // data model, so pass `false`.
            let pc_ground_z = pc.element.position().z;
            let emergency_lying = pc
                .element
                .sprite
                .position_iface
                .is_using_emergency_lying_box();
            let eye_position = crate::stealth::eye_point_xy(
                pos,
                pc.element.posture,
                pc.element.direction(),
                emergency_lying,
            )
            .to_geo()
            .into();
            let eye_z = pc_ground_z + crate::stealth::eye_z_for_posture(pc.element.posture, false);
            let detection_z =
                pc_ground_z + crate::stealth::detection_z_for_posture(pc.element.posture, false);
            // Refresh produced-noise volume once per PC per frame.
            // We pass the previous frame's stored volume so the
            // shadow-stage carry-over branches work correctly.
            //
            // Volume is forced to 0 when the PC is not "active" —
            // i.e. mid-door-pass, unconscious, lying-tied, lying
            // stuck-under-net, dead, or suspended by the script-driven
            // freeze-all flag.  The Rust model tracks those states on
            // separate fields (the early filter at the top of this
            // loop already drops PCs whose raw `element.active` is
            // false), so derive the active analogue from the per-state
            // flags here.
            let posture_inactive = matches!(
                pc.element.posture,
                Posture::Dead | Posture::DeadBack | Posture::StuckUnderNet | Posture::Tied
            );
            let active = element_active
                && !is_unconscious
                && !is_passing_door
                && !posture_inactive
                && !self.actors_frozen();
            let noise_volume = Self::pc_noise_volume(
                order_type,
                material,
                building_sector.is_some(),
                active,
                pc.actor.last_noise_volume,
            );
            pc_snapshots.push(PcSnapshot {
                id: pc_id,
                active: element_active,
                playable: pc.pc.playable,
                position: pos,
                eye_position,
                layer,
                posture: pc.element.posture,
                action_state: pc.actor.action_state,
                order_type,
                building_sector,
                detection_speed_in_forest,
                detection_speed_in_city,
                direction: pc.element.direction() as u16,
                // PC `is_able_to_fight`: life > 0 (filtered above),
                // active, not unconscious, and not
                // in a disguised posture (Tree/Spy).
                able_to_fight: element_active
                    && alive
                    && !matches!(pc.element.posture, Posture::Tree | Posture::Spy),
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                in_recovery,
                hth_weapon_id,
                is_vip: character.vip,
                is_robin: pc.pc.robin,
                eye_z,
                detection_z,
                melee_target: pc.pc.melee_target,
                principal_opponent: pc.human.opponents.first().map(|id| id.index()).unwrap_or(0),
                opponent_handles: pc.human.opponents.iter().map(|id| id.index()).collect(),
                unconscious: is_unconscious,
                carried: is_carried,
                passing_door: is_passing_door,
                noise_volume,
                sector_num: pc.element.sector().map(u16::from).unwrap_or(0),
                ground_elevation: pc.element.sprite.position_iface.get_elevation() as u16,
                is_swordfighting: !pc.human.opponents.is_empty(),
                guarded: pc.pc.guard.is_some(),
                obstacle_idx: pc.element.obstacle_index(),
            });
        }

        // Persist the freshly computed noise volume back onto each PC
        // actor so that the next frame's shadow-stage walk/run branch
        // can pick it up.  Stored on the human element so it carries
        // across frames.
        for snap in &pc_snapshots {
            if let Some(Entity::Pc(pc)) = self.entities.get_mut(snap.id) {
                pc.actor.last_noise_volume = snap.noise_volume;
            }
        }

        pc_snapshots
    }

    /// P1b — pre-compute destination forecasts for every PC.  Used by
    /// `forecast_destination_for_ia` when an NPC loses sight of its
    /// target; built before the NPC loop so we don't need to borrow
    /// the target entity while mutably borrowing the NPC entity.
    pub(super) fn tick_enemy_ai_build_pc_forecasts(
        &self,
    ) -> std::collections::HashMap<u32, crate::ai::ForecastedDestination> {
        let doors = self
            .mission_script
            .as_ref()
            .and_then(|s| s.game_host())
            .map(|h| h.doors.as_slice())
            .unwrap_or(&[]);
        let mut forecasts = std::collections::HashMap::with_capacity(self.pc_ids.len());
        forecasts.extend(self.pc_ids.iter().filter_map(|&pc_id| {
            let entity = self.entities.get(pc_id)?;
            let input = extract_forecast_input(entity)?;
            let forecast = crate::ai::forecast_destination_for_ia(
                &input,
                doors,
                &self.fast_grid.level.sectors,
                &self.fast_grid.level.sector_number_map,
            );
            Some((pc_id.index(), forecast))
        }));
        forecasts
    }

    /// P2b — count, per PC, how many enemy soldiers in a swordfight
    /// substate already have it as `primary_target`.  Used later by
    /// primary-target selection to penalize "ganging up" — soldiers
    /// prefer un-occupied targets (`PRIMARY_TARGET_UNOCCUPIED_PREFERED`).
    ///
    /// Only soldiers actively engaged in a swordfight count toward
    /// another target's occupancy — gated by the any-swordfight-substate
    /// check.  Counting every alerted soldier (e.g. ones still seeking,
    /// running to the alert, or shooting bows) inflates the penalty
    /// and makes `get_new_primary_target` consumers without a local
    /// override pick the wrong targets.
    ///
    /// `BTreeMap` (not `HashMap`) so the iteration order in the per-NPC
    /// loop is deterministic for replay hashing.
    pub(super) fn tick_enemy_ai_build_primary_target_multiplicity(
        &self,
    ) -> std::collections::BTreeMap<EntityId, u32> {
        let mut primary_target_multiplicity: std::collections::BTreeMap<EntityId, u32> =
            std::collections::BTreeMap::new();
        for (_, s) in self.entities.soldiers() {
            if let Some(ai) = s.npc.ai_brain.base()
                && ai.primary_target != 0
                && ai.current_substate.is_any_swordfight()
            {
                *primary_target_multiplicity
                    .entry(EntityId::Pc(crate::entity_id::PcId(ai.primary_target)))
                    .or_insert(0) += 1;
            }
        }
        primary_target_multiplicity
    }

    /// P2c-pre — for each NPC with a primary target, precompute whether
    /// a table swordfight (cross-sector via jump-line pair) is needed.
    /// Stored so the AI can build `Move + EnterSwordfight` sequences.
    pub(super) fn tick_enemy_ai_build_jump_lines(
        &self,
        assets: &LevelAssets,
    ) -> std::collections::HashMap<EntityId, Option<u32>> {
        let mut npc_jump_lines: std::collections::HashMap<EntityId, Option<u32>> =
            std::collections::HashMap::with_capacity(self.entities.soldiers().count());
        for (npc_id, s) in self.entities.soldiers() {
            if let Some(ai) = s.npc.ai_brain.enemy()
                && ai.base.primary_target != 0
            {
                let jl = crate::engine::melee::is_table_swordfight_needed(
                    &self.entities,
                    &self.fast_grid,
                    &assets.profile_manager,
                    npc_id,
                    EntityId::Pc(crate::entity_id::PcId(ai.base.primary_target)),
                );
                npc_jump_lines.insert(npc_id.into(), jl);
            }
        }
        npc_jump_lines
    }

    /// P2c — snapshot every alive soldier's combat-relevant state.
    ///
    /// `battle_decisions` iterates all fighters to build the us-list;
    /// the snapshot lets each per-NPC inner loop walk this immutable
    /// Vec instead of re-borrowing `self.entities`.  Also derives
    /// `archer_behind_me` from the reverse of `shield_bearer_before_me`
    /// links and writes it back onto each soldier's stored `EnemyAi`
    /// so direct self-reads stay consistent with the snapshot view.
    pub(super) fn tick_enemy_ai_build_soldier_snapshots(
        &mut self,
        assets: &LevelAssets,
    ) -> Vec<SoldierSnapshot> {
        let mut soldier_snapshots: Vec<SoldierSnapshot> =
            Vec::with_capacity(self.entities.soldiers().count());
        for (npc_id, s) in self.entities.soldiers() {
            if !s.element.active || s.human.unconscious {
                continue;
            }
            let able_to_fight = !s.human.unconscious && s.element.active && s.npc.life_points > 0;
            // Original: every `RHElementActorSoldier` owns
            // `RHArtificialMalignity`; its Hourglass calls soldier-only AI
            // methods unconditionally. A live soldier without EnemyAi is an
            // invalid partially-initialized entity.
            let enemy_ai = s.npc.ai_brain.enemy().unwrap_or_else(|| {
                panic!(
                    "active soldier {} has no EnemyAi brain",
                    EntityId::from(npc_id).index()
                )
            });
            let rank = enemy_ai.soldier_profile_rank;
            let company_number = enemy_ai.company_number;
            let pride = enemy_ai.soldier_profile_pride;
            let primary_target = enemy_ai.base.primary_target;
            let hth_weapon_id = enemy_ai.hth_weapon_id;
            let alert_status = enemy_ai.base.current_music_alert_status;
            let seek_flag_look_for_help = enemy_ai
                .seek_flags
                .contains(crate::ai_enemy::SeekFlags::LOOK_FOR_HELP_AFTER);
            let left_combat_neighbour = enemy_ai.left_combat_neighbour;
            let right_combat_neighbour = enemy_ai.right_combat_neighbour;
            let shield_bearer_before_me = enemy_ai.shield_bearer_before_me;
            let shield_bearer_direction = enemy_ai.shield_bearer_direction;
            let script_locked = enemy_ai.base.script_locked;
            let report_type = enemy_ai.base.my_reconnaissance_report.report_type;
            let report_seek_position = enemy_ai.base.my_reconnaissance_report.seek_position;
            let report_seen_bodies = enemy_ai.base.my_reconnaissance_report.seen_bodies.clone();
            let report_charly = enemy_ai.base.my_reconnaissance_report.charly;
            let alert_soldiers_point = enemy_ai.base.alert_soldiers_point;
            let is_tower_guard = enemy_ai.tower_guard;
            let patrol_chief = enemy_ai.base.patrol_chief;
            let antagonist = enemy_ai.base.antagonist;
            let ai_seek_position = enemy_ai.base.seek_position;
            let current_task_priority = enemy_ai.current_task_priority;
            let minimal_task_priority = enemy_ai.minimal_task_priority;
            // Snapshot the body-detectable list — corpses this
            // soldier has not yet reacted to.  See
            // `near_officer_who_is_informed_about_this_body`.
            let detectable_bodies = {
                let idx = crate::element::DetectableType::Body as usize;
                s.npc
                    .detectable_lists
                    .get(idx)
                    .map(|list| {
                        let mut bodies = Vec::with_capacity(list.len());
                        bodies.extend(list.iter().filter_map(|d| d.element.map(|e| e.index())));
                        bodies
                    })
                    .unwrap_or_default()
            };
            // Soldier weapon profile lookup for combat ranges and
            // formation flag.
            // Original: `RHProfileManager.h::GetSoldierProfile` and
            // `GetHandToHandProfile` assert bounds. Preserve that invariant
            // instead of substituting generic rank/fighting/ranges.
            let soldier_profile = assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "soldier {} requires missing soldier profile {}",
                        EntityId::from(npc_id).index(),
                        u32::from(s.soldier.soldier_profile_index)
                    )
                });
            let has_formation = soldier_profile.formation;
            let fighting_ability = {
                let base = soldier_profile.fighting;
                if s.soldier.cached_camp == Camp::Lacklandists {
                    let diff = crate::player_profile::DifficultyLevel::current();
                    diff.modify_capacity(
                        base,
                        crate::player_profile::difficulty_params::EASY_ENEMY_FIGHTING,
                        crate::player_profile::difficulty_params::HARD_ENEMY_FIGHTING,
                        100,
                    )
                } else {
                    base
                }
            };
            // `RHArtificialMalignity::IsArcher` is exactly
            // `mpMe->GetBow() != NULL`. `InitializeWeapons` creates the
            // bow whenever the one-based shooting-weapon id is non-zero;
            // the bow profile's ranges do not participate in identity.
            let bow_profile = if soldier_profile.shooting_weapon_id == 0 {
                None
            } else {
                Some(
                    assets
                        .profile_manager
                        .get_bow(soldier_profile.shooting_weapon_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "soldier {} requires missing bow profile {}",
                                EntityId::from(npc_id).index(),
                                soldier_profile.shooting_weapon_id
                            )
                        }),
                )
            };
            let is_archer_unit = is_archer_from_bow(bow_profile);
            let bow_max_range = bow_profile
                .map(|bow| {
                    if bow.has_long_shoot {
                        bow.long_shoot.range
                    } else {
                        bow.normal_shoot.range
                    }
                })
                .unwrap_or(0);
            let hth_profile = assets
                .profile_manager
                .get_hth_weapon(hth_weapon_id)
                .unwrap_or_else(|| {
                    panic!(
                        "soldier {} requires missing HtH weapon profile {}",
                        EntityId::from(npc_id).index(),
                        hth_weapon_id
                    )
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                hth_profile.distance[crate::weapons::WeaponDistance::Default as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Maximal as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            // Shield-bearer check: both the HtH weapon must be a
            // shield AND the sprite must have the WAITING_SHIELD
            // animation row.  We exercise the sprite's `has_animation`
            // (parsing the same conversion table the engine uses) so
            // soldiers whose sprite lacks the WAITING_SHIELD row no
            // longer falsely qualify just because their HtH weapon
            // flag is set.
            let weapon_is_shield = hth_profile.shield;
            let has_shield_anim = s
                .element
                .sprite
                .has_animation(crate::order::OrderType::WaitingShield);
            let is_shield_bearer = weapon_is_shield && has_shield_anim;
            let seek_position =
                MapPoint::new(enemy_ai.base.seek_position.x, enemy_ai.base.seek_position.y);
            // Honour check.
            let in_recovery = !able_to_fight
                || matches!(
                    s.actor.old_action,
                    crate::order::OrderType::BeingHitSword
                        | crate::order::OrderType::ExtractingArrowSword
                        | crate::order::OrderType::DyingSword
                        | crate::order::OrderType::BeingDeadSword
                        | crate::order::OrderType::FallingBackSword
                        | crate::order::OrderType::BeingUnconsciousSword
                        | crate::order::OrderType::BeingDeadFallenBackSword
                        | crate::order::OrderType::StandingUpSword
                );
            // Whether the soldier is inside a building, for the
            // `alert_officer` layer-penalty gate.  Includes the
            // door-transit branch — see `entity_data_inside_building`.
            let in_building = self.entity_data_inside_building(&s.element);
            // Forecast the officer's destination — used by
            // `alert_officer` to home on where the officer will be,
            // not where they are now.
            let forecast_destination = {
                let doors = self
                    .mission_script
                    .as_ref()
                    .and_then(|ms| ms.game_host())
                    .map(|h| h.doors.as_slice())
                    .unwrap_or(&[]);
                let pos_now = s.element.position_map();
                let door_pass = s
                    .actor
                    .active_door_pass
                    .as_ref()
                    .map(|dp| (dp.door_index, dp.direct));
                let input = crate::ai::ForecastInput {
                    position_map_x: pos_now.x,
                    position_map_y: pos_now.y,
                    sector: s.element.sector().map(u16::from).unwrap_or(0),
                    layer: s.element.layer(),
                    direction: s.element.direction() as u16,
                    forecasted_movement_z: s
                        .element
                        .sprite
                        .position_iface
                        .get_forecasted_movement()
                        .z,
                    door_pass,
                };
                crate::ai::forecast_destination_for_ia(
                    &input,
                    doors,
                    &self.fast_grid.level.sectors,
                    &self.fast_grid.level.sector_number_map,
                )
                .position
            };
            let able_to_help = crate::ai_enemy::soldier_is_able_to_help_state(
                able_to_fight,
                s.npc.ai_state(),
                s.npc.ai_substate(),
            );

            soldier_snapshots.push(SoldierSnapshot {
                id: npc_id.into(),
                position: s.element.position_map(),
                layer: s.element.layer(),
                camp: s.soldier.cached_camp,
                ai_state: s.npc.ai_state(),
                ai_substate: s.npc.ai_substate(),
                posture: s.element.posture,
                rank,
                company_number,
                pride,
                primary_target,
                principal_opponent: s.human.opponents.first().map(|id| id.index()).unwrap_or(0),
                opponent_handles: s.human.opponents.iter().map(|id| id.index()).collect(),
                able_to_fight,
                able_to_help,
                alert_status,
                seek_flag_look_for_help,
                direction: s.element.direction() as u16,
                action_state: s.actor.action_state,
                is_swordfighting: !s.human.opponents.is_empty(),
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                has_formation,
                is_shield_bearer,
                is_archer_unit,
                left_combat_neighbour,
                right_combat_neighbour,
                in_recovery,
                hth_weapon_id,
                is_vip: soldier_profile.vip,
                is_tower_guard,
                seek_position,
                shield_bearer_before_me,
                // Derived below from reverse scan.
                archer_behind_me: 0,
                shield_bearer_direction,
                bow_max_range,
                script_locked,
                report_type,
                report_seek_position,
                report_seen_bodies,
                report_charly,
                alert_soldiers_point,
                elevation: s.element.position().z as u16,
                ground_z: s.element.position().z,
                patrol_chief,
                antagonist,
                duty_flag: soldier_profile.duty,
                in_building,
                forecast_destination,
                detectable_bodies,
                ai_seek_position,
                current_task_priority,
                minimal_task_priority,
                view_direction: s.npc.view_direction,
                view_radius: s.npc.view_radius,
                real_half_aperture: s.npc.real_half_aperture,
                eye_blind: s.npc.eye_status.is_blind(),
                building_sector: self.entity_building_sector(s.element.sector()),
                is_rider: s.soldier.rider,
                passing_door: s.actor.active_door_pass.is_some(),
                obstacle_idx: s.element.obstacle_index(),
            });
        }

        // Derive `archer_behind_me` from the reverse of
        // `shield_bearer_before_me` links so the filter in
        // `get_nearest_free_shield_bearer` prevents double-claiming.
        // Also propagate back to the stored EnemyAi field for
        // consistency with direct self-reads.
        {
            // Collect (archer_handle, shield_bearer_handle) pairs.
            let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(soldier_snapshots.len());
            pairs.extend(
                soldier_snapshots
                    .iter()
                    .filter(|s| s.shield_bearer_before_me != 0)
                    .map(|s| (s.id.index(), s.shield_bearer_before_me)),
            );
            for (archer_handle, sb_handle) in &pairs {
                if let Some(sb) = soldier_snapshots
                    .iter_mut()
                    .find(|s| s.id.index() == *sb_handle)
                {
                    sb.archer_behind_me = *archer_handle;
                }
            }
            // Write back to stored EnemyAi fields so direct self-reads
            // (outside snapshots) stay fresh.
            for snap in &soldier_snapshots {
                let npc_id = snap.id;
                if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
                    && let Some(enemy_ai) = s.npc.ai_brain.enemy_mut()
                {
                    enemy_ai.archer_behind_me = snap.archer_behind_me;
                }
            }
        }

        soldier_snapshots
    }

    /// P2d — collect unconscious same-camp soldiers KO'd in a money
    /// fight.  `SoldierSnapshot` filters out unconscious soldiers,
    /// but `wants_to_continue_money_fight` needs to count them, so
    /// we keep this side-list separate.
    pub(super) fn tick_enemy_ai_build_ko_money_fight_soldiers(&self) -> Vec<(EntityId, Camp)> {
        let mut ko_money_fight_soldiers: Vec<(EntityId, Camp)> =
            Vec::with_capacity(self.entities.soldiers().count());
        for (npc_id, s) in self.entities.soldiers() {
            if !s.element.active {
                continue;
            }
            if s.npc.life_points <= 0 {
                continue;
            }
            if !s.human.unconscious {
                continue;
            }
            if !s
                .npc
                .ai_brain
                .base()
                .map(|ai| ai.knocked_out_in_money_fight)
                .unwrap_or(false)
            {
                continue;
            }
            ko_money_fight_soldiers.push((npc_id.into(), s.soldier.cached_camp));
        }
        ko_money_fight_soldiers
    }

    /// Snapshot every potential human + object target referenced by one
    /// NPC's per-type detectable lists at that NPC's creation-order boundary.
    /// The resulting maps let the body / friend / missed-friend / beggar /
    /// object passes run without re-borrowing `self.entities` for each lookup.
    ///
    /// Captures the targets the per-type detection-refresh loop
    /// dereferences from each detectable list.  Hashing by
    /// `EntityId` means a non-trivial detectable list size doesn't
    /// blow up to a linear scan per lookup.
    ///
    /// Body / Friend / MissedFriend / Beggar share the `HumanTarget`
    /// shape — all four feed `compute_visibility` against a human
    /// target, so the per-target metadata is identical (position /
    /// posture / unconscious / etc) plus a couple of per-kind
    /// predicate fields (`able_to_help` for Friend,
    /// `is_true_or_false_beggar` for the Beggar cleanup-detectables
    /// predicate).  Object stays in its own snapshot map because it
    /// uses `compute_object_visibility`, which has a different query
    /// shape.
    pub(super) fn tick_enemy_ai_build_human_object_targets_for_npc(
        &self,
        npc_id: EntityId,
    ) -> (
        std::collections::HashMap<EntityId, HumanTarget>,
        std::collections::HashMap<EntityId, ObjectTarget>,
    ) {
        use crate::element::DetectableType;

        let entity = self.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "NPC {} disappeared before its live detection target snapshot",
                npc_id.index()
            )
        });
        let npc = entity.npc_data().unwrap_or_else(|| {
            panic!(
                "creation-ordered NPC {} has no NPC data for detection",
                npc_id.index()
            )
        });
        let mut human_ids: std::collections::HashSet<EntityId> = std::collections::HashSet::new();
        let mut object_ids: std::collections::HashSet<EntityId> = std::collections::HashSet::new();
        for kind in [
            DetectableType::Body,
            DetectableType::Friend,
            DetectableType::MissedFriend,
            DetectableType::Beggar,
        ] {
            for detectable in &npc.detectable_lists[kind as usize] {
                if let Some(id) = detectable.element {
                    human_ids.insert(id);
                }
            }
        }
        for detectable in &npc.detectable_lists[DetectableType::Object as usize] {
            if let Some(id) = detectable.element {
                object_ids.insert(id);
            }
        }

        let mut human_targets: std::collections::HashMap<EntityId, HumanTarget> =
            std::collections::HashMap::with_capacity(human_ids.len());
        for id in human_ids {
            let Some(entity) = self.entities.get(id) else {
                continue;
            };
            let position = entity.element_data().position_map();
            let layer = entity.element_data().layer();
            let posture = entity.element_data().posture;
            // These IDs came from a human-typed detectable list. Original
            // `RefreshDetection` statically casts each entry to
            // `RHElementActorHuman*`; a non-human entry is corrupt data, not
            // a waiting/conscious human.
            let actor = entity.actor_data().unwrap_or_else(|| {
                panic!("human detectable target {} has no actor data", id.index())
            });
            let human = entity.human_data().unwrap_or_else(|| {
                panic!("human detectable target {} has no human data", id.index())
            });
            let action_state = actor.action_state;
            let unconscious = human.unconscious;
            let active = entity.element_data().active;
            let passing_door = actor.active_door_pass.is_some();
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            // HumanTarget's `eye_z` is consumed as the *detection*
            // point Z by `VisibilityQuery::target_eye_z` (the target
            // side of `compute_detection_point`), so use
            // `detection_z_for_posture` not `eye_z_for_posture` —
            // they differ for Lying (+2 vs +5) and Carried (+25 vs
            // default).
            let is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);
            let eye_z = entity.element_data().position().z
                + crate::stealth::detection_z_for_posture(posture, is_rider);
            let direction = entity.element_data().direction();
            let is_pc = matches!(entity, Entity::Pc(_));
            // Only PCs carry a guard; everything else is unguarded
            // by definition.
            let guarded = if let Entity::Pc(p) = entity {
                p.pc.guard.is_some()
            } else {
                false
            };

            // Soldier `is_able_to_help`.
            let able_to_help = if let Entity::Soldier(s) = entity {
                let able_to_fight = active && !s.human.unconscious && s.npc.life_points > 0;
                crate::ai_enemy::soldier_is_able_to_help_state(
                    able_to_fight,
                    s.npc.ai_state(),
                    s.npc.ai_substate(),
                )
            } else {
                // Civilians and PCs are never able to help — the
                // predicate is soldier-only.
                false
            };

            // True for a civilian whose profile is a beggar, or any
            // human in `Posture::SimulatingBeggar`.
            let is_true_or_false_beggar = if let Entity::Civilian(c) = entity {
                c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
            } else {
                posture == crate::element::Posture::SimulatingBeggar
            };

            let obstacle_idx = entity.element_data().obstacle_index();
            human_targets.insert(
                id,
                HumanTarget {
                    position,
                    layer,
                    eye_z,
                    posture,
                    direction,
                    action_state,
                    building_sector,
                    unconscious,
                    active,
                    is_pc,
                    able_to_help,
                    is_true_or_false_beggar,
                    passing_door,
                    guarded,
                    obstacle_idx,
                },
            );
        }

        let mut object_targets: std::collections::HashMap<EntityId, ObjectTarget> =
            std::collections::HashMap::with_capacity(object_ids.len());
        for id in object_ids {
            let Some(entity) = self.entities.get(id) else {
                continue;
            };
            let position = entity.element_data().position_map();
            let layer = entity.element_data().layer();
            let active = entity.element_data().active;
            // Original `RefreshDetection` casts DETECTABLE_OBJECT entries to
            // `RHElementObject*` before reading `BelongsToBeggar`.
            let belongs_to_beggar = entity
                .object_data()
                .unwrap_or_else(|| {
                    panic!("object detectable target {} has no object data", id.index())
                })
                .belongs_to_beggar;
            object_targets.insert(
                id,
                ObjectTarget {
                    position,
                    layer,
                    belongs_to_beggar,
                    active,
                },
            );
        }

        (human_targets, object_targets)
    }
}

#[cfg(test)]
mod tests {
    use super::is_archer_from_bow;

    #[test]
    fn bow_presence_defines_archer_even_with_zero_normal_range() {
        let bow = crate::profiles::BowProfile::default();
        assert_eq!(bow.normal_shoot.range, 0);
        assert!(is_archer_from_bow(Some(&bow)));
        assert!(!is_archer_from_bow(None));
    }
}
