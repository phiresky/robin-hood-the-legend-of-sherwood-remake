//! Coordinator for atomic Original Linux-v48 engine adoption.
//!
//! Every child plan is constructed against the same initialized mission
//! before any mutation occurs. Applying the coordinator to a detached clone is
//! then infallible. The public replay entry point remains disconnected until
//! the remaining decoded sections have plans and are included here.

use thiserror::Error;

use crate::engine::{Engine, EngineInner, LevelAssets};

use super::{
    LegacySaveAbiProfile,
    adopt::{LegacyEntityFixups, derive_position_topology, preflight_initialized_v48_adoption},
    adopt_actor_ownership::LegacyActorOwnershipAdoptionPlan,
    adopt_camera::{LegacyCameraAdoptionPlan, LegacyCameraHostState},
    adopt_campaign::LegacyCampaignAdoptionPlan,
    adopt_elements::LegacyStaticElementAdoption,
    adopt_grid::{LegacyFastFindGridAdoptionPlan, LegacyGridHostState},
    adopt_hiking_tail::{LegacyHikingTailAdoptionPlan, LegacyTrajectoryHostOutput},
    adopt_object_leaves::LegacyObjectLeafAdoptionPlan,
    adopt_paths::{LegacyPathAdoptionPlan, preflight_v48_paths},
    adopt_pc_human::LegacyPcHumanAdoptionPlan,
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
};

#[derive(Debug, Error)]
#[error("cannot adopt Original Linux-v48 save section {stage}: {detail}")]
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
}

impl LegacyKnownAdoptionPlan {
    pub(crate) fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        body: &LegacySaveBody,
    ) -> Result<Self, LegacyKnownAdoptionError> {
        if body.header.abi_profile != LegacySaveAbiProfile::PortLinuxI386V48 {
            return Err(LegacyKnownAdoptionError {
                stage: "header",
                detail: format!("expected Linux i386 v48, got {:?}", body.header.abi_profile),
            });
        }

        let campaign = stage(
            "campaign",
            LegacyCampaignAdoptionPlan::preflight(
                &body.campaigns,
                &assets.profile_manager,
                body.header.mission_id,
            ),
        )?;
        let entities: LegacyEntityFixups = stage(
            "element identities",
            preflight_initialized_v48_adoption(engine, assets, body),
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
                &entities,
                &position_topology,
                &sequence_topology,
                &sequences,
                &body.campaigns.live.campaign,
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

        Ok(Self {
            campaign,
            preamble,
            preamble_services,
            camera,
            vm_arena,
            elements,
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
        })
    }

    /// Apply to a detached initialized mission after all preflight succeeds.
    pub(crate) fn apply(self, engine: &mut EngineInner) -> LegacyKnownHostState {
        self.campaign.apply(engine);
        engine.apply_legacy_linux_preamble_state(self.preamble);
        let preamble = self.preamble_services.apply(engine);
        let camera = self.camera.apply(engine);
        self.elements.apply(engine);
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
        LegacyKnownHostState {
            preamble,
            camera,
            grid,
            simple: host,
            trajectory,
        }
    }
}

pub(crate) struct LegacyKnownHostState {
    pub preamble: LegacyPreambleHostState,
    pub camera: LegacyCameraHostState,
    pub grid: LegacyGridHostState,
    pub simple: LegacySimpleHostState,
    pub trajectory: LegacyTrajectoryHostOutput,
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
    LegacyKnownAdoptionPlan::preflight(engine.legacy_adoption_inner(), assets, body).map(drop)
}

pub(crate) fn adopt_known_linux_v48_candidate(
    engine: &mut Engine,
    assets: &LevelAssets,
    body: &LegacySaveBody,
) -> Result<LegacyKnownHostState, LegacyKnownAdoptionError> {
    let mut candidate = engine.legacy_adoption_inner().clone();
    let plan = LegacyKnownAdoptionPlan::preflight(&candidate, assets, body)?;
    let host = plan.apply(&mut candidate);
    engine.install_legacy_adoption_inner(candidate);
    Ok(host)
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
