//! Coordinator for atomic Original v48 engine adoption.
//!
//! Every child plan is constructed against the same initialized mission
//! before any mutation occurs. Applying the coordinator to a detached clone is
//! then infallible. The public replay entry point remains disconnected until
//! the remaining decoded sections have plans and are included here.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::engine::{Engine, EngineInner, LevelAssets};

use super::{
    adopt::{LegacyEntityFixups, derive_position_topology},
    adopt_actor_ownership::LegacyActorOwnershipAdoptionPlan,
    adopt_camera::{LegacyCameraAdoptionPlan, LegacyCameraHostState},
    adopt_campaign::LegacyCampaignAdoptionPlan,
    adopt_dynamic_elements::LegacyDynamicElementAdoptionPlan,
    adopt_elements::LegacyStaticElementAdoption,
    adopt_grid::{LegacyFastFindGridAdoptionPlan, LegacyGridHostState},
    adopt_hiking_tail::{LegacyHikingTailAdoptionPlan, LegacyTrajectoryHostOutput},
    adopt_mobile::LegacyMobileAdoptionPlan,
    adopt_object_leaves::LegacyObjectLeafAdoptionPlan,
    adopt_paths::{LegacyPathAdoptionPlan, preflight_v48_paths},
    adopt_pc_human::LegacyPcHumanAdoptionPlan,
    adopt_post_load::{
        LegacyPostLoadAdoptionPlan, LegacyPostLoadHostOutput, LegacyRngRestorePolicy,
    },
    adopt_preamble::LegacyLinuxPreambleState,
    adopt_preamble_services::{
        LegacyPreambleHostState, LegacyPreambleServicesPlan, preflight_v48_preamble_services,
    },
    adopt_sequences::{
        LegacySequenceAdoptionPlan, LegacySequenceTopology, preflight_v48_sequence_manager,
    },
    adopt_simple::{LegacySimpleAdoptionPlan, LegacySimpleHostState},
    adopt_tail_basic::LegacyTailBasicAdoptionPlan,
    adopt_tail_runtime::LegacyTailRuntimeAdoptionPlan,
    adopt_vm_arena::LegacyVmArenaPlan,
    body::LegacySaveBody,
    topology_adapter::derive_static_element_topology,
};

#[derive(Debug, Error)]
#[error("cannot adopt Original v48 save section {stage}: {detail}")]
pub struct LegacyKnownAdoptionError {
    pub stage: &'static str,
    pub detail: String,
}

/// All currently implemented engine-owned sections, validated as one unit.
///
/// TODO(save-import): include the outstanding sound/messenger/game,
/// Soldier leaf, waypoint/projectile, dead-PC/shield, and host-state plans
/// before exposing installation to replay or normal save loading.
pub(crate) struct LegacyKnownAdoptionPlan {
    campaign: LegacyCampaignAdoptionPlan,
    preamble: LegacyLinuxPreambleState,
    preamble_services: LegacyPreambleServicesPlan,
    camera: LegacyCameraAdoptionPlan,
    vm_arena: LegacyVmArenaPlan,
    elements: LegacyStaticElementAdoption,
    mobiles: LegacyMobileAdoptionPlan,
    object_leaves: LegacyObjectLeafAdoptionPlan,
    grid: LegacyFastFindGridAdoptionPlan,
    sequences: LegacySequenceAdoptionPlan,
    actor_ownership: LegacyActorOwnershipAdoptionPlan,
    pc_human: LegacyPcHumanAdoptionPlan,
    hiking_tail: LegacyHikingTailAdoptionPlan,
    tail_runtime: LegacyTailRuntimeAdoptionPlan,
    paths: LegacyPathAdoptionPlan,
    simple: LegacySimpleAdoptionPlan,
    tail_basic: LegacyTailBasicAdoptionPlan,
    post_load: LegacyPostLoadAdoptionPlan,
    post_dynamic_creation_counter: u32,
}

impl LegacyKnownAdoptionPlan {
    pub(crate) fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        body: &LegacySaveBody,
        rng_policy: LegacyRngRestorePolicy,
        entities: LegacyEntityFixups,
        post_dynamic_creation_counter: u32,
    ) -> Result<Self, LegacyKnownAdoptionError> {
        let campaign = stage(
            "campaign",
            LegacyCampaignAdoptionPlan::preflight(
                &body.campaigns,
                &assets.profile_manager,
                body.header.mission_id,
            ),
        )?;
        let position_topology = stage(
            "position topology",
            derive_position_topology(engine, assets),
        )?;
        let sequence_topology = stage(
            "sequence topology",
            LegacySequenceTopology::derive(engine, assets),
        )?;
        let preamble = stage(
            "engine preamble",
            LegacyLinuxPreambleState::try_from_v48(body.header.abi_profile, &body.engine),
        )?;
        let preamble_services = stage(
            "sound/messenger/game preamble",
            preflight_v48_preamble_services(body.header.abi_profile, &body.engine),
        )?;
        let camera = stage(
            "camera/locker preamble",
            LegacyCameraAdoptionPlan::preflight(engine, body.header.abi_profile, &body.engine),
        )?;
        let vm_arena = stage(
            "computed Location arena",
            LegacyVmArenaPlan::preflight(
                engine,
                assets,
                &body.element_payloads,
                &body.grid,
                &body.hiking_guide,
                body.tail.global_script_members.as_ref(),
            ),
        )?;
        let elements = stage(
            "elements",
            LegacyStaticElementAdoption::preflight(
                engine,
                assets,
                &body.element_payloads,
                &entities,
                &position_topology,
            ),
        )?;
        let mobiles = stage(
            "mobile master state",
            LegacyMobileAdoptionPlan::preflight(
                engine,
                &body.element_payloads,
                &entities,
                &position_topology,
            ),
        )?;
        let object_leaves = stage(
            "object/bonus/scroll/target/FX leaf state",
            LegacyObjectLeafAdoptionPlan::preflight(
                engine,
                assets,
                &body.element_payloads,
                &entities,
                &vm_arena,
            ),
        )?;
        let grid = stage(
            "FastFindGrid",
            LegacyFastFindGridAdoptionPlan::preflight(
                engine,
                assets,
                &body.grid,
                &entities,
                &position_topology,
                &vm_arena,
            ),
        )?;
        let sequences = stage(
            "SequenceManager",
            preflight_v48_sequence_manager(&body.sequence_manager, &entities, &sequence_topology),
        )?;
        let pc_human = stage(
            "Human/PC leaf state",
            LegacyPcHumanAdoptionPlan::preflight(
                engine,
                &body.element_payloads,
                body.header.abi_profile,
                &entities,
                &position_topology,
                &sequence_topology,
                &sequences,
                &body.campaigns.live.campaign,
                assets,
            ),
        )?;
        let actor_ownership = stage(
            "actor ownership/script state",
            LegacyActorOwnershipAdoptionPlan::preflight(
                engine,
                assets,
                &body.element_payloads,
                &entities,
                &sequence_topology,
                &sequences,
                &vm_arena,
            ),
        )?;
        let hiking_tail = stage(
            "HikingGuide/dead PC/shield/trajectory",
            LegacyHikingTailAdoptionPlan::preflight(
                engine,
                assets,
                &body.hiking_guide,
                &body.projectile_trajectory,
                &body.tail,
                body.engine.shield_protected,
                &entities,
                &vm_arena,
            ),
        )?;
        let tail_runtime = stage(
            "global VM/timers/camera",
            LegacyTailRuntimeAdoptionPlan::preflight(
                engine,
                assets,
                body.tail.global_script_members.as_ref(),
                &body.tail.script_globals,
                &body.tail.timers,
                &entities,
                &sequences,
                &vm_arena,
            ),
        )?;
        let paths = stage(
            "path queues/graph",
            preflight_v48_paths(
                engine,
                assets,
                &body.failed_path_requests,
                &body.tail.pathfinder,
                &sequences,
                &entities,
            ),
        )?;
        let simple = stage(
            "selection/feedback",
            LegacySimpleAdoptionPlan::preflight(
                engine,
                &entities,
                &body.user_lock,
                &body.selected_elements,
                &body.selected_before_lock,
                &body.follow_view,
                &body.ground_mark,
                &body.titbits,
                &body.minimap,
            ),
        )?;
        let tail_basic = stage(
            "global AI/mission statistics",
            LegacyTailBasicAdoptionPlan::preflight(
                engine,
                &entities,
                &body.tail.global_ai,
                &body.tail.mission_statistics,
            ),
        )?;
        let post_load = stage(
            "post-load consequences",
            LegacyPostLoadAdoptionPlan::preflight(
                engine,
                &body.element_payloads,
                &body.tail.global_ai,
                &entities,
                rng_policy,
            ),
        )?;

        Ok(Self {
            campaign,
            preamble,
            preamble_services,
            camera,
            vm_arena,
            elements,
            mobiles,
            object_leaves,
            grid,
            sequences,
            actor_ownership,
            pc_human,
            hiking_tail,
            tail_runtime,
            paths,
            simple,
            tail_basic,
            post_load,
            post_dynamic_creation_counter,
        })
    }

    /// Apply to a detached initialized mission after all preflight succeeds.
    pub(crate) fn apply(
        self,
        engine: &mut EngineInner,
        assets: &LevelAssets,
    ) -> LegacyKnownHostState {
        self.campaign.apply(engine);
        engine.apply_legacy_linux_preamble_state(self.preamble);
        engine.world.next_original_creation_order = self.post_dynamic_creation_counter;
        let preamble = self.preamble_services.apply(engine);
        let camera = self.camera.apply(engine);
        self.elements.apply(engine);
        self.mobiles.apply(engine);
        self.object_leaves.apply(engine);
        let grid = self.grid.apply(engine);
        self.sequences.apply(engine);
        self.actor_ownership.apply(engine);
        self.pc_human.apply(engine);
        let trajectory = self.hiking_tail.apply_engine(engine);
        self.tail_runtime.apply(engine);
        self.vm_arena.apply(engine);
        self.paths.apply(engine);
        let host = self.simple.apply(engine);
        self.tail_basic.apply(engine);
        let post_load = self.post_load.apply(engine, assets);
        LegacyKnownHostState {
            preamble,
            camera,
            grid,
            simple: host,
            trajectory,
            post_load,
        }
    }
}

pub struct LegacyKnownHostState {
    preamble: LegacyPreambleHostState,
    camera: LegacyCameraHostState,
    grid: LegacyGridHostState,
    simple: LegacySimpleHostState,
    trajectory: LegacyTrajectoryHostOutput,
    post_load: LegacyPostLoadHostOutput,
}

impl LegacyKnownHostState {
    /// Restore the host-owned display state serialized by the Original.
    ///
    /// Replay callers have two display-state holders (the logical replay
    /// driver and, in visual mode, the renderer host), so this deliberately
    /// borrows and may be applied to both.
    pub fn apply_display_to(&self, display: &mut crate::engine::HostDisplayState) {
        self.camera.clone().apply_to(display);
        self.simple.clone().apply_minimap_to(display);
    }

    pub fn selected_view_element(&self) -> Option<crate::element::EntityId> {
        self.simple.selected_view_element
    }

    pub fn trajectory_output(&self) -> LegacyTrajectoryHostOutput {
        self.trajectory
    }

    pub fn post_load_output(&self) -> LegacyPostLoadHostOutput {
        self.post_load
    }
}

/// Validate every adoption slice currently assembled into the coordinator
/// without changing the live engine.
///
/// This is exposed for corpus auditing while final installation remains
/// crate-internal and deliberately disconnected.
pub fn preflight_known_linux_v48_adoption(
    engine: &Engine,
    assets: &LevelAssets,
    body: &LegacySaveBody,
) -> Result<(), LegacyKnownAdoptionError> {
    let (candidate, entities, post_dynamic_creation_counter) =
        prepare_dynamic_candidate(engine, assets, body)?;
    LegacyKnownAdoptionPlan::preflight(
        &candidate,
        assets,
        body,
        LegacyRngRestorePolicy::PreserveRecordedGlobalDrawStream,
        entities,
        post_dynamic_creation_counter,
    )
    .map(drop)
}

/// Atomically replace an initialized mission with a decoded Original
/// Linux-v48 save while retaining the replay's authoritative RNG draw stream.
///
/// Every fallible conversion is performed against a detached candidate. The
/// live engine changes only after the complete save has passed preflight.
pub fn adopt_known_linux_v48_replay(
    engine: &mut Engine,
    assets: &LevelAssets,
    body: &LegacySaveBody,
) -> Result<LegacyKnownHostState, LegacyKnownAdoptionError> {
    adopt_known_linux_v48_candidate(
        engine,
        assets,
        body,
        LegacyRngRestorePolicy::PreserveRecordedGlobalDrawStream,
    )
}

fn adopt_known_linux_v48_candidate(
    engine: &mut Engine,
    assets: &LevelAssets,
    body: &LegacySaveBody,
    rng_policy: LegacyRngRestorePolicy,
) -> Result<LegacyKnownHostState, LegacyKnownAdoptionError> {
    let (mut candidate, entities, post_dynamic_creation_counter) =
        prepare_dynamic_candidate(engine, assets, body)?;
    let plan = LegacyKnownAdoptionPlan::preflight(
        &candidate,
        assets,
        body,
        rng_policy,
        entities,
        post_dynamic_creation_counter,
    )?;
    let host = plan.apply(&mut candidate, assets);
    host.grid.clone().apply(&mut candidate);
    engine.install_legacy_adoption_inner(candidate);
    Ok(host)
}

fn prepare_dynamic_candidate(
    engine: &Engine,
    assets: &LevelAssets,
    body: &LegacySaveBody,
) -> Result<(EngineInner, LegacyEntityFixups, u32), LegacyKnownAdoptionError> {
    let mut candidate = engine.legacy_adoption_inner().clone();
    let static_topology = stage(
        "static element topology",
        derive_static_element_topology(&candidate, assets),
    )?;
    let dynamic = stage(
        "dynamic elements",
        LegacyDynamicElementAdoptionPlan::preflight(
            &candidate,
            assets,
            &body.element_envelope,
            &static_topology,
            &body.campaigns.live.campaign.characters,
            body.engine.creation_counter,
        ),
    )?;
    let post_dynamic_creation_counter = dynamic.post_load_creation_counter();
    let mut entities = dynamic.apply(&mut candidate);
    stage(
        "beam-me PC identity",
        remap_saved_beam_pc_identities(
            &mut candidate,
            body,
            &mut entities,
            post_dynamic_creation_counter,
        ),
    )?;
    Ok((candidate, entities, post_dynamic_creation_counter))
}

/// Pair serialized team PCs by exact campaign description, not incidental
/// constructor order.
///
/// `RHEngine::PopulateBeamMes` can assign the selected team to beam-me points
/// in a different construction order after a campaign reload. Both
/// construction order and `muwBeamMeIndex` can consequently differ, while
/// the character profile remains the exact sprite/behavior identity. Exact
/// `mpDescription` identity is preferred where the initialized campaign
/// retained it; duplicate campaign descriptions with the same profile are
/// otherwise isomorphic at this pre-adoption boundary. Mapping by raw
/// creation order can apply one character's sprite state to another profile.
fn remap_saved_beam_pc_identities(
    engine: &mut EngineInner,
    body: &LegacySaveBody,
    entities: &mut LegacyEntityFixups,
    next_original_creation_order: u32,
) -> Result<(), String> {
    let mut runtime_team = Vec::new();
    for (pc_id, pc) in engine.world.entities.pcs() {
        if pc.pc.beam_me_index < 0 {
            continue;
        }
        let entity_id = crate::element::EntityId::from(pc_id);
        let description = pc.pc.campaign_description_index.ok_or_else(|| {
            format!("initialized beam-me PC {entity_id} has no campaign description identity")
        })?;
        runtime_team.push((entity_id, description, pc.pc.profile_index.0));
    }

    let mut saved_team = Vec::new();
    let campaign_characters = &body.campaigns.live.campaign.characters;
    for record in &body.element_payloads.records {
        let super::payload_dispatch::LegacyElementPayload::ActorPc(saved) = &record.payload else {
            continue;
        };
        if matches!(
            body.element_envelope.records[record.slot].resolution,
            super::elements::LegacyElementResolution::ConstructDynamic { .. }
        ) {
            // Dynamic PCs were just constructed from their serialized
            // description and already have exact sprite/profile identity.
            continue;
        }
        let raw_beam = saved.pre_human.beam_me_index;
        if raw_beam == u16::MAX {
            continue;
        }
        i16::try_from(raw_beam).map_err(|_| {
            format!(
                "saved PC creation order {} has invalid beam-me index {raw_beam}",
                record.header.creation_order
            )
        })?;
        let description = saved.pre_human.description.0;
        let profile = campaign_characters
            .get(description as usize)
            .ok_or_else(|| {
                format!(
                    "saved beam-me PC creation order {} references absent campaign description {description}",
                    record.header.creation_order
                )
            })?
            .character_profile_index
            .ok_or_else(|| {
                format!(
                    "saved beam-me PC creation order {} campaign description {description} has no character profile",
                    record.header.creation_order
                )
            })?;
        saved_team.push((
            record.slot,
            record.header.creation_order,
            description,
            profile,
        ));
    }

    let mut assignments = Vec::with_capacity(saved_team.len());
    let mut assigned_entities = BTreeSet::new();
    for &(slot, creation_order, description, profile) in &saved_team {
        let exact =
            runtime_team
                .iter()
                .find(|&&(entity_id, runtime_description, runtime_profile)| {
                    !assigned_entities.contains(&entity_id)
                        && runtime_description == description
                        && runtime_profile == profile
                });
        let profile_match = runtime_team
            .iter()
            .find(|&&(entity_id, _, runtime_profile)| {
                !assigned_entities.contains(&entity_id) && runtime_profile == profile
            });
        let &(entity_id, _, _) = exact.or(profile_match).ok_or_else(|| {
            format!(
                "saved beam-me PC creation order {creation_order} campaign description {description} profile {profile} has no isomorphic initialized team PC"
            )
        })?;
        assigned_entities.insert(entity_id);
        assignments.push((slot, creation_order, entity_id));
    }

    for (slot, creation_order, entity_id) in assignments {
        let saved_slot = entities.by_saved_slot.get_mut(slot).ok_or_else(|| {
            format!("saved PC creation order {creation_order} has absent element slot {slot}")
        })?;
        entities.by_creation_order.insert(creation_order, entity_id);
        *saved_slot = Some(entity_id);
    }

    entities.creation_order_by_entity.clear();
    for (&creation_order, &entity_id) in &entities.by_creation_order {
        if let Some(first_creation_order) = entities
            .creation_order_by_entity
            .insert(entity_id, creation_order)
        {
            return Err(format!(
                "beam-me remap assigned {entity_id} to saved creation orders {first_creation_order} and {creation_order}"
            ));
        }
    }
    engine.world.install_original_creation_orders(
        entities.creation_order_by_entity.clone(),
        next_original_creation_order,
    );
    Ok(())
}

fn stage<T, E: std::fmt::Display>(
    stage: &'static str,
    result: Result<T, E>,
) -> Result<T, LegacyKnownAdoptionError> {
    result.map_err(|error| LegacyKnownAdoptionError {
        stage,
        detail: error.to_string(),
    })
}
