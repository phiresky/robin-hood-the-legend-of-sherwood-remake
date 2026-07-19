//! Domain routing for synchronous native calls.
//!
//! The declarative native registry remains the sole owner of IDs and
//! signatures. This module only routes an already-decoded native to a
//! cohesive implementation module.

use super::*;

mod actors;
mod ai;
mod campaign;
mod script_core;
mod sequences;
mod world;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeDomain {
    ScriptCore,
    Actors,
    Ai,
    Sequences,
    World,
    Campaign,
}

impl NativeFn {
    /// Compiler-exhaustive classification of every registered native.
    /// Adding a registry entry now fails this match at compile time instead
    /// of falling through six wildcard dispatchers to a runtime panic.
    fn domain(self) -> NativeDomain {
        use NativeDomain::*;
        use NativeFn::*;
        match self {
            ForceCheckVictory
            | InitGlobal
            | SetGlobal
            | GetGlobal
            | Start
            | Thanx
            | Then
            | IsNull
            | IsActorEqual
            | IsActorDead
            | IsActorKO
            | IsActorTied
            | IsActorHS
            | StopActor
            | God
            | Select
            | Deactivate
            | Activate
            | LockAI
            | UnlockAI
            | Freeze
            | FreezeAll
            | NoWhere
            | GetDistance
            | Rand
            | PrintConsole
            | GetCustomCampaignValue
            | SetCustomCampaignValue
            | GetCustomNPCValue
            | SetCustomNPCValue
            | BitwiseAnd
            | BitwiseOr
            | BitwiseXor
            | HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood
            | GetNumberOfPCs
            | GetPC
            | GetRansomMoney
            | SetRansomMoney
            | GetDifficultyLevel
            | GetSizeOfMissionTeam
            | IsMissionTeamValid
            | GetNumberOfPCsAlive
            | AreAllBlazonsWon
            | SecretAgentsAreBackInSherwood
            | GetLastPlayedMission
            | GetNextPlayedMission
            | GetActorScript
            | GetDoorScript
            | GetPatchScript
            | GetLocationScript
            | GetSoundSourceScript
            | GetBuildingScript
            | GetWayScript
            | GetActorIndex
            | GetDoorIndex
            | GetPatchIndex
            | GetLocationIndex
            | GetSoundSourceIndex
            | GetBuildingIndex
            | GetWayIndex => ScriptCore,

            ThisActor
            | GetNumberOfActorsInEngine
            | IsActorAnimation
            | IsActorObject
            | IsActorCharacter
            | IsActorPC
            | IsActorNPC
            | IsActorSoldier
            | IsActorCivilian
            | IsActorAnimal
            | IsActorCart
            | IsActorActive
            | IsActorRider
            | IsUnblipped
            | GetActorPosture
            | SetActorPosture
            | GetActorDirection
            | SetActorDirection
            | GetActorLocation
            | SetActorLocation
            | IsInside
            | IsInsideBuilding
            | UnBlip
            | GetMovementStyle
            | GetCurrentAction
            | InflictPain
            | SetCompanyNumber
            | SetAlwaysAttentive
            | SetInvisible
            | IsInvisible
            | MakePCCrouched
            | GetActorActionState
            | SetActorActionState
            | Sees
            | EnableViewCone
            | PrototypeFilterEvent
            | SendMessage
            | SendMessageWithArguments
            | SetActionAvailable
            | IsActionAvailable
            | SetPersistentProperty
            | GetPersistentProperty
            | IsAnyCivilianDead
            | IsAnyEnemyDead
            | GetOverallEnemyAlert
            | GetOverallCivilianAlert
            | HasPCAction
            | HasAnyPCAction
            | HasAnyActivePCAction
            | HasAnyActionSelected => Actors,

            SetAIAlertStatus
            | GetAIAlertStatus
            | SetAIState
            | GetAIState
            | SetAIAttitude
            | GetAIAttitude
            | SetAILevel
            | StareActor
            | StareLocation
            | AssignPath
            | AssignPost
            | ForceBattleDecision
            | MakeNoise
            | SetPathWalkingStyle
            | GetSoldierRank
            | SwitchToAlertPath
            | SetNPCEmoticon
            | ForbidNPCRemark
            | DeclareAsCombatTrainer
            | AddAsSubordinate
            | RemoveAllSubordinates
            | AddRepulsivePoint
            | DeleteRepulsivePoint => Ai,

            ScrollCameraTo
            | ScrollCameraSlowlyTo
            | JumpCameraTo
            | SetZoomLevel
            | StartDialog
            | DisplayMap
            | DisplayConsole
            | CustomizeMinimapDisplay
            | DefineFlatTrajectoryZone
            | AddShortBriefing
            | DoneShortBriefing
            | ChooseVictoryDefeatText
            | DisplayPopupText
            | DisplaySherwoodReport
            | FadeToBlack
            | SetOutlineDisplay
            | GetOutlineDisplay
            | SetViewRadius
            | PlayTrapJingle
            | RecordScrollCameraTo
            | RecordJumpCameraTo
            | RecordSetZoom
            | RecordDisplayMap
            | RecordMoveCameraTo
            | RecordLockCameraOn
            | RecordClearCameraLock
            | RecordPlayDialog
            | RecordDisplayPopupText
            | RecordActionAvailable
            | RecordCharacterAvailable
            | RecordSendMessage
            | RecordSendMessageWithArguments
            | RecordMove
            | RecordMoveNear
            | RecordMoveIntoBuilding
            | RecordEnterGame
            | RecordLeaveGame
            | RecordTurnTo
            | RecordPlayAnim
            | RecordPlayAnimLoop
            | RecordPlayAnimFreeze
            | RecordReplaceAnim
            | RecordRestoreAnim
            | ResetAnim
            | RecordSpeak
            | RecordSpeakPC
            | RecordLockAI
            | RecordUnlockAI
            | RecordLockUser
            | RecordUnLockUser
            | RecordFreezeAll
            | RecordTimer
            | RecordSeekActor
            | RecordSeekActorMessage
            | RecordSeekActorMessageWithArguments
            | RecordStopSeek
            | RecordAction
            | RecordTakeCorpse
            | RecordLeaveCorpse
            | RecordUnBlip
            | RecordStartMobileElement
            | RecordStopMobileElement
            | RecordActivateMobileElement
            | RecordDeactivateMobileElement => Sequences,

            IsAnimationActive
            | SetAnimationState
            | IsPatchApplied
            | ApplyPatch
            | ResetPatch
            | LockPatch
            | SetPatchAnimationActive
            | LinkTargetToFX
            | SuspendAllSoundSources
            | ResumeAllSoundSources
            | ActivateSoundSource
            | DeactivateSoundSource
            | DestroySoundSource
            | CleanFromHisBuildingBeforeTeleport
            | CleanFromScriptZoneBeforeTeleport
            | AddToScriptZoneAfterTeleport
            | SetCorpseExistsInBuilding
            | PutActorInBuilding
            | SetBuildingActive
            | GetAnyActorInsideBuilding
            | AreAllPCsInside
            | AreAllEnemiesInsideHS
            | AreAllPCsAliveInside
            | IsDoorLockedPC
            | IsDoorUnlockable
            | IsDoorLockedNPCCivilian
            | IsDoorLockedNPCVillain
            | SetDoorLockedPC
            | SetDoorUnlockable
            | SetDoorLockedNPCCivilian
            | SetDoorLockedNPCVillain
            | SetDoorSpecialAutorisation
            | ActivateDoorMouseSector
            | ThisScroll
            | GetScrollStatus
            | SetScrollStatus
            | AttachScrollToNPC => World,

            GetPCFromMissionTeam
            | AddPCToMissionTeam
            | RemovePCFromMissionTeam
            | GetNumberOfObligatoryPCsInMissionTeam
            | GetObligatoryPCFromMissionTeam
            | IsPCObligatoryInMissionTeam
            | IsMenToBlazonConversionMode
            | GetNumberOfBeamMes
            | MoveBeamMe
            | GetActorForBeamMe
            | RegisterAsProductionSector
            | AddProductionPoint
            | GetNumberOfActorsInSector
            | GetActorInSector
            | WinBlazon
            | LoseBlazon
            | IsBlazonWon
            | IsBonusItemPickedUp
            | ConfiscateMoney
            | AddPCToGang
            | AddFarmerToGang
            | SetExperiences
            | TransformHandleTargetToTakeTarget
            | GetRobin
            | GetRelic
            | GetPCType
            | SelectActorPC
            | IsPCSelected
            | GetNumberOfSelectedPCs
            | GetSelectedPC
            | Reveal
            | SequenceReveal
            | IsActorOutOfAction
            | AddObjective
            | CompleteObjective
            | SetPatrolShouldRun
            | ComputeLocationBetween => Campaign,
        }
    }
}

pub(super) fn call_immediate(
    context: &mut NativeContext<'_, '_>,
    index: u32,
    stack: &mut NativeStack,
) -> i32 {
    let Ok(native) = NativeFn::try_from(index) else {
        // We cannot drain the stack because an unknown ID has no signature.
        // A malformed SCB calling outside the declarative registry is already
        // invalid, but retaining the zero result matches the prior adapter.
        tracing::error!("Unknown native function index {index}");
        return 0;
    };

    match native.domain() {
        NativeDomain::ScriptCore => context.dispatch_script_core(native, stack),
        NativeDomain::Actors => context.dispatch_actors(native, stack),
        NativeDomain::Ai => context.dispatch_ai(native, stack),
        NativeDomain::Sequences => context.dispatch_sequences(native, stack),
        NativeDomain::World => context.dispatch_world(native, stack),
        NativeDomain::Campaign => context.dispatch_campaign(native, stack),
    }
}
