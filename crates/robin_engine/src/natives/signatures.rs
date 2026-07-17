//! Declarative registry for native IDs, signatures, provenance, and Lua exposure.
//!
//! Original provenance: `original-code/GVMCoreCustom.cpp`,
//! `VMCoreCustom::InitializeStaticExtensions`, is the authoritative 0..=264
//! registration order. `original-code/RHScriptAPI.scs` is the authoritative
//! source for the corresponding script-visible signatures.

use super::NativeFn;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeParamSig {
    pub ty: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSignature {
    pub name: &'static str,
    pub return_type: &'static str,
    pub params: &'static [NativeParamSig],
}

/// Namespace owning a native ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeNamespace {
    /// Fixed namespace used by shipped SCB bytecode.
    Original,
    /// Functions supplied by the Rust/Lua port, outside the original ABI.
    RustExtension,
}

/// Complete metadata for one native function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeDefinition {
    pub native: NativeFn,
    pub namespace: NativeNamespace,
    pub signature: NativeSignature,
    pub expose_to_lua: bool,
}

/// The single declaration of every native. Consumers expand this to generate
/// the ID enum and metadata tables, so order, signatures, and Lua enumeration
/// cannot drift independently.
macro_rules! native_registry {
    ($consumer:ident) => {
        $consumer! {
            original {
            InitGlobal => ("void", [("int", "iID"), ("int", "iValue")], lua);
            SetGlobal => ("void", [("int", "iID"), ("int", "iValue")], lua);
            GetGlobal => ("int", [("int", "iID")], lua);
            GetActorScript => ("Actor", [("int", "iPosition")], lua);
            GetDoorScript => ("Door", [("int", "iPosition")], lua);
            GetPatchScript => ("Patch", [("int", "iPosition")], lua);
            GetLocationScript => ("Location", [("int", "iPosition")], lua);
            GetSoundSourceScript => ("SoundSource", [("int", "iPosition")], lua);
            GetBuildingScript => ("Building", [("int", "iPosition")], lua);
            GetWayScript => ("Way", [("int", "iPosition")], lua);
            GetActorIndex => ("int", [("Actor", "actor")], lua);
            GetDoorIndex => ("int", [("Door", "door")], lua);
            GetPatchIndex => ("int", [("Patch", "patch")], lua);
            GetLocationIndex => ("int", [("Location", "location")], lua);
            GetSoundSourceIndex => ("int", [("SoundSource", "soundsource")], lua);
            GetBuildingIndex => ("int", [("Building", "building")], lua);
            GetWayIndex => ("int", [("Way", "way")], lua);
            StartDialog => ("void", [("int", "iDialogue")], lua);
            ScrollCameraTo => ("bool", [("Location", "location")], lua);
            ScrollCameraSlowlyTo => ("bool", [("Location", "location"), ("float", "fSpeed")], lua);
            JumpCameraTo => ("bool", [("Location", "location")], lua);
            SetZoomLevel => ("bool", [("float", "fZoom")], lua);
            DisplayMap => ("bool", [("bool", "bDisplay")], lua);
            DisplayConsole => ("void", [], lua);
            CustomizeMinimapDisplay => ("void", [("Actor", "actor"), ("int", "iKindOfDot")], lua);
            DefineFlatTrajectoryZone => ("void", [("Location", "pLocation"), ("int", "iApex")], lua);
            AddShortBriefing => ("void", [("int", "iID"), ("bool", "bPrimary")], lua);
            DoneShortBriefing => ("void", [("int", "iID")], lua);
            ChooseVictoryDefeatText => ("void", [("int", "iID")], lua);
            ForceCheckVictory => ("void", [], lua);
            Start => ("bool", [], lua);
            Thanx => ("bool", [], lua);
            Then => ("int", [], lua);
            RecordScrollCameraTo => ("bool", [("Location", "location")], lua);
            RecordJumpCameraTo => ("bool", [("Location", "location")], lua);
            RecordSetZoom => ("bool", [("float", "fZoomLevel")], lua);
            RecordDisplayMap => ("bool", [("bool", "bDisplay")], lua);
            RecordActionAvailable => ("bool", [("Actor", "actor"), ("int", "iAction"), ("bool", "bAvailable")], lua);
            RecordCharacterAvailable => ("bool", [("Actor", "actor"), ("bool", "bAvailable")], lua);
            RecordLockCameraOn => ("bool", [("Actor", "actor")], lua);
            RecordClearCameraLock => ("bool", [], lua);
            RecordPlayDialog => ("bool", [("int", "iDialogID")], lua);
            RecordMoveCameraTo => ("bool", [("Location", "destination"), ("int", "iSpeed")], lua);
            RecordSendMessage => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode")], lua);
            RecordSendMessageWithArguments => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode"), ("int", "iArgument1"), ("int", "iArgument2")], lua);
            RecordMove => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iStyle")], lua);
            RecordEnterGame => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iDirection"), ("int", "iStyle")], lua);
            RecordLeaveGame => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iDirection"), ("int", "iStyle")], lua);
            RecordTurnTo => ("bool", [("Actor", "actor"), ("Location", "location")], lua);
            RecordPlayAnim => ("bool", [("Actor", "actor"), ("int", "iId")], lua);
            RecordPlayAnimLoop => ("bool", [("Actor", "actor"), ("int", "iId")], lua);
            RecordPlayAnimFreeze => ("bool", [("Actor", "actor"), ("int", "iId")], lua);
            RecordLockAI => ("bool", [("Actor", "actor")], lua);
            RecordUnlockAI => ("bool", [("Actor", "actor")], lua);
            RecordLockUser => ("bool", [], lua);
            RecordUnLockUser => ("bool", [], lua);
            RecordTimer => ("bool", [("int", "iFrames")], lua);
            RecordSeekActor => ("bool", [("Actor", "actor"), ("Actor", "target"), ("int", "iStyle"), ("float", "fTolerance")], lua);
            RecordStopSeek => ("bool", [("Actor", "actor")], lua);
            RecordAction => ("bool", [("Actor", "actor"), ("int", "iID"), ("int", "iValue")], lua);
            RecordReplaceAnim => ("bool", [("Actor", "actor"), ("int", "iOriginalAnim"), ("int", "iNewAnim")], lua);
            RecordRestoreAnim => ("bool", [("Actor", "actor"), ("int", "iOriginalAnim")], lua);
            RecordSpeakPC => ("bool", [("Actor", "actor"), ("int", "iRemarkID"), ("int", "iRemarkVariant")], lua);
            RecordTakeCorpse => ("int", [("Actor", "taker"), ("Actor", "corpse"), ("int", "iStyle")], lua);
            RecordMoveIntoBuilding => ("bool", [("Actor", "actor"), ("Location", "pointBeforeDoor"), ("int", "iStyle")], lua);
            RecordLeaveCorpse => ("bool", [("Actor", "actor")], lua);
            ResetAnim => ("bool", [("Actor", "actor")], lua);
            RecordStartMobileElement => ("void", [("int", "iIndex")], lua);
            RecordStopMobileElement => ("void", [("int", "iIndex")], lua);
            RecordSpeak => ("bool", [("Actor", "actor"), ("int", "iRemarkID")], lua);
            RecordSeekActorMessage => ("bool", [("Actor", "pActor"), ("Actor", "pTarget"), ("int", "iStyle"), ("float", "fDistance"), ("Actor", "pActorEvent"), ("int", "iID")], lua);
            RecordSeekActorMessageWithArguments => ("bool", [("Actor", "pActor"), ("Actor", "pTarget"), ("int", "iStyle"), ("float", "fDistance"), ("Actor", "pActorEvent"), ("int", "iID"), ("int", "iArg1"), ("int", "iArg2")], lua);
            RecordActivateMobileElement => ("void", [("int", "iIndex")], lua);
            RecordDeactivateMobileElement => ("void", [("int", "iIndex")], lua);
            ThisActor => ("Actor", [], lua);
            GetNumberOfActorsInEngine => ("int", [], lua);
            IsActorAnimation => ("bool", [("Actor", "actor")], lua);
            IsActorObject => ("bool", [("Actor", "actor")], lua);
            IsActorCharacter => ("bool", [("Actor", "actor")], lua);
            IsActorPC => ("bool", [("Actor", "actor")], lua);
            IsActorNPC => ("bool", [("Actor", "actor")], lua);
            IsActorSoldier => ("bool", [("Actor", "actor")], lua);
            IsActorCivilian => ("bool", [("Actor", "actor")], lua);
            IsActorAnimal => ("bool", [("Actor", "actor")], lua);
            IsActorCart => ("bool", [("Actor", "actor")], lua);
            IsNull => ("bool", [("Actor", "actor")], lua);
            IsActorEqual => ("bool", [("Actor", "one"), ("Actor", "two")], lua);
            IsActorDead => ("bool", [("Actor", "actor")], lua);
            IsActorKO => ("bool", [("Actor", "actor")], lua);
            IsActorTied => ("bool", [("Actor", "actor")], lua);
            IsActorHS => ("bool", [("Actor", "actor")], lua);
            GetActorPosture => ("int", [("Actor", "actor")], lua);
            SetActorPosture => ("void", [("Actor", "actor"), ("int", "iPosture")], lua);
            GetActorDirection => ("int", [("Actor", "actor")], lua);
            SetActorDirection => ("bool", [("Actor", "actor"), ("int", "iDirection")], lua);
            GetActorLocation => ("Location", [("Actor", "actor")], lua);
            SetActorLocation => ("bool", [("Actor", "actor"), ("Location", "location")], lua);
            IsInside => ("bool", [("Actor", "actor"), ("Location", "location")], lua);
            IsInsideBuilding => ("bool", [("Actor", "actor"), ("Building", "building")], lua);
            UnBlip => ("bool", [("Actor", "actor")], lua);
            GetMovementStyle => ("int", [("Actor", "actor")], lua);
            GetCurrentAction => ("int", [("Actor", "actor")], lua);
            InflictPain => ("void", [("Actor", "actor"), ("int", "iDamage"), ("bool", "bStun")], lua);
            StopActor => ("bool", [("Actor", "actor")], lua);
            Sees => ("bool", [("Actor", "actorNPC"), ("Actor", "actorTarget")], lua);
            EnableViewCone => ("void", [("Actor", "actor")], lua);
            GetOutlineDisplay => ("bool", [], lua);
            SetOutlineDisplay => ("void", [("bool", "bDisplay")], lua);
            PrototypeFilterEvent => ("bool", [("Actor", "prototype"), ("Actor", "actorSource"), ("int", "iEvent")], lua);
            SendMessage => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode")], lua);
            SendMessageWithArguments => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode"), ("int", "iArgument1"), ("int", "iArgument2")], lua);
            God => ("Actor", [], lua);
            Select => ("bool", [("int", "selectCode")], lua);
            Deactivate => ("bool", [("Actor", "actor")], lua);
            Activate => ("bool", [("Actor", "actor")], lua);
            SetActionAvailable => ("bool", [("Actor", "actor"), ("int", "iAction"), ("bool", "bAvailable")], lua);
            IsActionAvailable => ("bool", [("Actor", "actor"), ("int", "iAction")], lua);
            SetPersistentProperty => ("bool", [("Actor", "actor"), ("int", "iProperty"), ("int", "iAmount")], lua);
            GetPersistentProperty => ("int", [("Actor", "actor"), ("int", "iProperty")], lua);
            IsAnyCivilianDead => ("bool", [], lua);
            IsAnyEnemyDead => ("bool", [], lua);
            GetOverallEnemyAlert => ("int", [], lua);
            GetOverallCivilianAlert => ("int", [], lua);
            SetAIAlertStatus => ("bool", [("Actor", "actor"), ("int", "iStatus")], lua);
            GetAIAlertStatus => ("int", [("Actor", "actor")], lua);
            SetAIState => ("bool", [("Actor", "actor"), ("int", "iState")], lua);
            GetAIState => ("int", [("Actor", "actor")], lua);
            SetAIAttitude => ("bool", [("Actor", "actor"), ("int", "iAttitude")], lua);
            GetAIAttitude => ("int", [("Actor", "actor")], lua);
            SetAILevel => ("bool", [("Actor", "actor"), ("int", "iProperty"), ("int", "iLevel")], lua);
            StareActor => ("void", [("Actor", "actor"), ("Actor", "actorTarget"), ("bool", "bTurnSprite")], lua);
            StareLocation => ("void", [("Actor", "actor"), ("Location", "locPoint"), ("bool", "bTurnSprite")], lua);
            AssignPath => ("void", [("Actor", "actor"), ("Way", "myWay")], lua);
            AssignPost => ("void", [("Actor", "actor"), ("Location", "location"), ("int", "iDirection")], lua);
            LockAI => ("void", [("Actor", "actor"), ("bool", "bRememberEvents")], lua);
            UnlockAI => ("void", [("Actor", "actor")], lua);
            ForceBattleDecision => ("void", [("Actor", "actor"), ("int", "iDecision")], lua);
            MakeNoise => ("void", [("Location", "location"), ("int", "iTypeID")], lua);
            Freeze => ("void", [("Actor", "actor"), ("bool", "bFrozen")], lua);
            FreezeAll => ("void", [("bool", "bFrozen")], lua);
            SetPathWalkingStyle => ("void", [("Actor", "NPC"), ("int", "i0Walking1Running2Backward")], lua);
            GetSoldierRank => ("int", [("Actor", "actor")], lua);
            IsAnimationActive => ("bool", [("Actor", "actor")], lua);
            SetAnimationState => ("bool", [("Actor", "actor"), ("bool", "bState")], lua);
            IsPatchApplied => ("bool", [("Patch", "patch")], lua);
            ApplyPatch => ("bool", [("Patch", "patch")], lua);
            ResetPatch => ("bool", [("Patch", "patch")], lua);
            SuspendAllSoundSources => ("bool", [], lua);
            ResumeAllSoundSources => ("bool", [], lua);
            ActivateSoundSource => ("bool", [("SoundSource", "source")], lua);
            DeactivateSoundSource => ("bool", [("SoundSource", "source")], lua);
            DestroySoundSource => ("bool", [("SoundSource", "source")], lua);
            CleanFromHisBuildingBeforeTeleport => ("bool", [("Actor", "actor")], no_lua);
            CleanFromScriptZoneBeforeTeleport => ("bool", [("Actor", "actor"), ("Location", "cestLaZone")], no_lua);
            AddToScriptZoneAfterTeleport => ("bool", [("Actor", "actor"), ("Location", "cestLaZone")], no_lua);
            SetCorpseExistsInBuilding => ("void", [("Actor", "pActor")], no_lua);
            // TODO(original parity): `RHScriptAPI.scs` and `GVMCoreCustom.cpp`
            // spell ID 156 as `PutActorInBulding`. Rust retains its established
            // corrected public spelling.
            PutActorInBuilding => ("void", [("Actor", "actor"), ("Building", "building")], no_lua);
            SetBuildingActive => ("void", [("Building", "building"), ("bool", "bActive")], lua);
            GetAnyActorInsideBuilding => ("Actor", [("Building", "building")], lua);
            NoWhere => ("Location", [], lua);
            GetDistance => ("int", [("Location", "here"), ("Location", "there")], lua);
            Rand => ("int", [("int", "iMaximum")], lua);
            PrintConsole => ("void", [("int", "iValue")], lua);
            GetSizeOfMissionTeam => ("int", [], lua);
            GetPCFromMissionTeam => ("Actor", [("int", "ulPC")], lua);
            AddPCToMissionTeam => ("void", [("Actor", "actor")], lua);
            RemovePCFromMissionTeam => ("void", [("Actor", "actor")], lua);
            GetNumberOfObligatoryPCsInMissionTeam => ("int", [], lua);
            GetObligatoryPCFromMissionTeam => ("Actor", [("int", "ulPC")], lua);
            IsPCObligatoryInMissionTeam => ("bool", [("Actor", "actor")], lua);
            IsMissionTeamValid => ("bool", [], lua);
            GetLastPlayedMission => ("int", [], lua);
            GetNextPlayedMission => ("int", [], lua);
            IsMenToBlazonConversionMode => ("bool", [], no_lua);
            GetNumberOfBeamMes => ("int", [], no_lua);
            MoveBeamMe => ("void", [("int", "iIndex"), ("Location", "pLocation")], no_lua);
            SetCompanyNumber => ("void", [("Actor", "pActor"), ("int", "iNumber")], lua);
            SetAlwaysAttentive => ("void", [("Actor", "actor"), ("bool", "bYes")], lua);
            WinBlazon => ("void", [("Actor", "blazon")], lua);
            LoseBlazon => ("void", [("Actor", "blazon")], lua);
            SetInvisible => ("void", [("Actor", "actor"), ("bool", "bHollow")], lua);
            IsInvisible => ("bool", [("Actor", "actor")], lua);
            IsDoorLockedPC => ("bool", [("Door", "door")], lua);
            IsDoorUnlockable => ("bool", [("Door", "door")], lua);
            IsDoorLockedNPCCivilian => ("bool", [("Door", "door")], lua);
            IsDoorLockedNPCVillain => ("bool", [("Door", "door")], lua);
            SetDoorLockedPC => ("void", [("Door", "door"), ("bool", "bState")], lua);
            SetDoorUnlockable => ("void", [("Door", "door"), ("bool", "bState")], lua);
            SetDoorLockedNPCCivilian => ("void", [("Door", "door"), ("bool", "bState")], lua);
            SetDoorLockedNPCVillain => ("void", [("Door", "door"), ("bool", "bState")], lua);
            SetDoorSpecialAutorisation => ("void", [("Door", "door"), ("Actor", "actor"), ("bool", "bDirect")], lua);
            ActivateDoorMouseSector => ("void", [("bool", "bActive"), ("Door", "door")], lua);
            ThisScroll => ("Actor", [], lua);
            GetScrollStatus => ("int", [("Actor", "scroll")], lua);
            SetScrollStatus => ("void", [("Actor", "scroll"), ("int", "iStatus")], lua);
            GetCustomCampaignValue => ("int", [("int", "iIndex")], lua);
            SetCustomCampaignValue => ("void", [("int", "iIndex"), ("int", "iValue")], lua);
            GetCustomNPCValue => ("int", [("Actor", "actor"), ("int", "iIndex")], lua);
            SetCustomNPCValue => ("void", [("Actor", "actor"), ("int", "iIndex"), ("int", "iValue")], lua);
            RegisterAsProductionSector => ("void", [("int", "iType"), ("Location", "sector"), ("int", "iProductionSpeed")], lua);
            AddProductionPoint => ("void", [("int", "iType"), ("Location", "point")], lua);
            GetActorForBeamMe => ("Actor", [("int", "iIndex")], lua);
            DisplayPopupText => ("void", [("int", "iPopupTextID")], lua);
            RecordDisplayPopupText => ("void", [("int", "iPopupTextID")], lua);
            GetNumberOfActorsInSector => ("int", [("Location", "loc")], lua);
            GetActorInSector => ("Actor", [("Location", "loc"), ("int", "iIndex")], lua);
            BitwiseAnd => ("int", [("int", "i"), ("int", "j")], lua);
            BitwiseOr => ("int", [("int", "i"), ("int", "j")], lua);
            BitwiseXor => ("int", [("int", "i"), ("int", "j")], lua);
            HasPCAction => ("bool", [("Actor", "actPC"), ("int", "iActionCode")], lua);
            HasAnyPCAction => ("bool", [("int", "iActionCode")], lua);
            GetRobin => ("Actor", [], lua);
            RecordMoveNear => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iStyle"), ("int", "iTolerance")], lua);
            ComputeLocationBetween => ("Location", [("Location", "locA"), ("Location", "locB"), ("float", "fLambdaBetweenZeroAndOne")], lua);
            DeclareAsCombatTrainer => ("void", [("Actor", "actor")], lua);
            GetRelic => ("Actor", [("int", "iID")], lua);
            GetNumberOfPCs => ("int", [], lua);
            GetPC => ("Actor", [("int", "i")], lua);
            AddAsSubordinate => ("void", [("Actor", "actChief"), ("Actor", "actSubordinate")], lua);
            RemoveAllSubordinates => ("void", [("Actor", "actChief")], lua);
            SwitchToAlertPath => ("void", [("Actor", "actSoldier")], lua);
            IsActorRider => ("bool", [("Actor", "actWhoever")], lua);
            IsUnblipped => ("bool", [("Actor", "actWhoever")], lua);
            IsBlazonWon => ("bool", [("Actor", "blazon")], lua);
            AddRepulsivePoint => ("int", [("Location", "location"), ("float", "fRadius"), ("float", "fActionRadius"), ("int", "iFlags")], lua);
            SetViewRadius => ("void", [("int", "iRadius")], lua);
            RecordFreezeAll => ("void", [("bool", "bFreeze")], lua);
            DeleteRepulsivePoint => ("void", [("int", "iID")], lua);
            SetNPCEmoticon => ("void", [("Actor", "actNPC"), ("int", "iEmoticonType"), ("int", "iTime")], lua);
            ConfiscateMoney => ("void", [("Actor", "actCapitalist")], lua);
            AreAllPCsInside => ("bool", [("Location", "location")], lua);
            AreAllEnemiesInsideHS => ("bool", [("Location", "locZone")], lua);
            AddPCToGang => ("void", [("Actor", "actor")], lua);
            AttachScrollToNPC => ("void", [("Actor", "actNPC"), ("Actor", "scroll")], lua);
            AreAllBlazonsWon => ("bool", [], lua);
            IsBonusItemPickedUp => ("bool", [("Actor", "actItem")], lua);
            GetRansomMoney => ("int", [], lua);
            SetRansomMoney => ("void", [("int", "iRansomMoneyAmount")], lua);
            GetDifficultyLevel => ("int", [], lua);
            DisplaySherwoodReport => ("void", [], lua);
            IsActorActive => ("bool", [("Actor", "actor")], lua);
            AddFarmerToGang => ("void", [("int", "iType"), ("int", "iExperienceSword"), ("int", "iExperienceBow")], lua);
            SetExperiences => ("void", [("Actor", "actor"), ("int", "iExperienceSword"), ("int", "iExperienceBow")], lua);
            RecordUnBlip => ("bool", [("Actor", "pActor")], lua);
            SetPatchAnimationActive => ("void", [("Patch", "patch"), ("bool", "bActive")], lua);
            GetNumberOfPCsAlive => ("int", [], lua);
            AreAllPCsAliveInside => ("bool", [("Location", "location")], lua);
            TransformHandleTargetToTakeTarget => ("void", [("Actor", "actTarget")], lua);
            IsPCSelected => ("bool", [("Actor", "actPC")], lua);
            GetNumberOfSelectedPCs => ("int", [], lua);
            GetSelectedPC => ("Actor", [("int", "iIndex")], lua);
            PlayTrapJingle => ("void", [], lua);
            MakePCCrouched => ("void", [("Actor", "actPC")], lua);
            HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood => ("bool", [("int", "iActionCode")], lua);
            LockPatch => ("void", [("Patch", "patch"), ("bool", "bLocked")], lua);
            HasAnyActivePCAction => ("bool", [("int", "iActionCode")], lua);
            GetPCType => ("int", [("Actor", "actPC")], lua);
            SelectActorPC => ("void", [("Actor", "actPCOrGodForAllPCs"), ("bool", "bSelectOrUnselect")], lua);
            HasAnyActionSelected => ("bool", [("Actor", "actPC")], lua);
            GetActorActionState => ("int", [("Actor", "actor")], lua);
            SetActorActionState => ("void", [("Actor", "actor"), ("int", "iActionState")], lua);
            SecretAgentsAreBackInSherwood => ("bool", [], lua);
            FadeToBlack => ("void", [("int", "iSpeed")], lua);
            LinkTargetToFX => ("void", [("Actor", "actTarget"), ("Actor", "actFX")], lua);
            ForbidNPCRemark => ("void", [("Actor", "actNPC"), ("int", "iRemark"), ("bool", "bTrueMeansForbidFalseMeansAllow")], lua);
            }
            rust_extensions {
            Reveal => ("int", [("Actor", "actActor")], lua);
            AddObjective => ("int", [("int", "iObjectiveID"), ("bool", "bIsMainObjective")], lua);
            CompleteObjective => ("int", [("int", "iObjectiveID")], lua);
            IsActorOutOfAction => ("bool", [("Actor", "actActor")], lua);
            SetPatrolShouldRun => ("void", [("Actor", "actPatrolLeader"), ("bool", "bShouldRun")], lua);
            SequenceReveal => ("int", [("Actor", "actActor")], lua);
            }
        }
    };
}

pub(crate) use native_registry;

macro_rules! lua_exposure {
    (lua) => {
        true
    };
    (no_lua) => {
        false
    };
}

macro_rules! signature {
    ($name:ident, $return_type:literal, [$(($param_type:literal, $param_name:literal)),* $(,)?]) => {
        NativeSignature {
            name: stringify!($name),
            return_type: $return_type,
            params: &[
                $(NativeParamSig { ty: $param_type, name: $param_name }),*
            ],
        }
    };
}

macro_rules! define_native_metadata {
    (
        original {
            $( $original:ident => ($original_return:literal, $original_params:tt, $original_lua:ident); )*
        }
        rust_extensions {
            $( $extension:ident => ($extension_return:literal, $extension_params:tt, $extension_lua:ident); )*
        }
    ) => {
        /// Complete registry in numeric ID order.
        pub const NATIVE_REGISTRY: &[NativeDefinition] = &[
            $(
                NativeDefinition {
                    native: NativeFn::$original,
                    namespace: NativeNamespace::Original,
                    signature: signature!($original, $original_return, $original_params),
                    expose_to_lua: lua_exposure!($original_lua),
                },
            )*
            $(
                NativeDefinition {
                    native: NativeFn::$extension,
                    namespace: NativeNamespace::RustExtension,
                    signature: signature!($extension, $extension_return, $extension_params),
                    expose_to_lua: lua_exposure!($extension_lua),
                },
            )*
        ];

        /// Compatibility view of all signatures in numeric ID order.
        pub const NATIVE_SIGNATURES: &[NativeSignature] = &[
            $(signature!($original, $original_return, $original_params),)*
            $(signature!($extension, $extension_return, $extension_params),)*
        ];
    };
}

native_registry!(define_native_metadata);

pub fn native_definition_by_index(index: u32) -> Option<&'static NativeDefinition> {
    NATIVE_REGISTRY
        .iter()
        .find(|definition| definition.native as u32 == index)
}

pub fn native_definition_by_name(name: &str) -> Option<&'static NativeDefinition> {
    NATIVE_REGISTRY
        .iter()
        .find(|definition| definition.signature.name == name)
}

pub fn native_signature_by_index(index: u32) -> Option<&'static NativeSignature> {
    native_definition_by_index(index).map(|definition| &definition.signature)
}

pub fn native_signature_by_name(name: &str) -> Option<&'static NativeSignature> {
    native_definition_by_name(name).map(|definition| &definition.signature)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::natives::{ORIGINAL_NATIVE_COUNT, RUST_EXTENSION_NATIVE_START, native_name};

    #[test]
    fn registry_has_exhaustive_unique_ids_names_and_signatures() {
        assert_eq!(NATIVE_REGISTRY.len(), NATIVE_SIGNATURES.len());

        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for (position, definition) in NATIVE_REGISTRY.iter().enumerate() {
            let id = definition.native as u32;
            assert!(ids.insert(id), "duplicate native ID {id}");
            assert!(
                names.insert(definition.signature.name),
                "duplicate native name {}",
                definition.signature.name
            );
            assert_eq!(native_name(id), definition.signature.name);
            assert_eq!(
                native_signature_by_index(id),
                Some(&definition.signature),
                "missing signature for registry position {position}"
            );
            assert_eq!(
                native_signature_by_name(definition.signature.name),
                Some(&definition.signature)
            );
            assert_eq!(NATIVE_SIGNATURES[position], definition.signature);
        }
    }

    #[test]
    fn namespaces_are_contiguous_and_ordered() {
        assert_eq!(ORIGINAL_NATIVE_COUNT, 265);
        assert_eq!(RUST_EXTENSION_NATIVE_START, ORIGINAL_NATIVE_COUNT);

        for (expected_id, definition) in NATIVE_REGISTRY
            .iter()
            .take(ORIGINAL_NATIVE_COUNT as usize)
            .enumerate()
        {
            assert_eq!(definition.namespace, NativeNamespace::Original);
            assert_eq!(definition.native as usize, expected_id);
        }

        let extensions = &NATIVE_REGISTRY[ORIGINAL_NATIVE_COUNT as usize..];
        assert!(!extensions.is_empty());
        for (offset, definition) in extensions.iter().enumerate() {
            assert_eq!(definition.namespace, NativeNamespace::RustExtension);
            assert_eq!(
                definition.native as u32,
                RUST_EXTENSION_NATIVE_START + offset as u32
            );
        }
    }

    #[test]
    fn original_namespace_matches_original_registration_table_exactly() {
        // Original provenance: `VMCoreCustom::InitializeStaticExtensions` in
        // `original-code/GVMCoreCustom.cpp` assigns every ABI index explicitly.
        let original = include_str!("../../../../original-code/GVMCoreCustom.cpp");
        let mut expected = vec![None; ORIGINAL_NATIVE_COUNT as usize];
        for line in original.lines() {
            let Some(rest) = line.trim().strip_prefix("mparrayNativeFunctions[") else {
                continue;
            };
            let Some((index, rest)) = rest.split_once("] = I") else {
                continue;
            };
            let index: usize = index.parse().expect("original native index is numeric");
            if index >= expected.len() {
                continue;
            }
            let (name, _) = rest
                .split_once(';')
                .expect("original native assignment contains a semicolon");
            let name = match name {
                // TODO(original parity): preserve Rust's established corrected
                // spelling while keeping the original typo explicit here.
                "PutActorInBulding" => "PutActorInBuilding",
                name => name,
            };
            assert!(
                expected[index].replace(name).is_none(),
                "duplicate original ID {index}"
            );
        }

        for (index, definition) in NATIVE_REGISTRY
            .iter()
            .take(ORIGINAL_NATIVE_COUNT as usize)
            .enumerate()
        {
            assert_eq!(
                expected[index],
                Some(definition.signature.name),
                "original native registration mismatch at ID {index}"
            );
        }
        assert!(expected.into_iter().all(|name| name.is_some()));
    }

    #[test]
    fn lua_enumeration_is_unique_and_follows_registry_order() {
        let exposed: Vec<_> = NATIVE_REGISTRY
            .iter()
            .filter(|definition| definition.expose_to_lua)
            .collect();
        assert!(!exposed.is_empty());
        assert!(
            exposed
                .windows(2)
                .all(|pair| (pair[0].native as u32) < pair[1].native as u32)
        );

        let names: HashSet<_> = exposed
            .iter()
            .map(|definition| definition.signature.name)
            .collect();
        assert_eq!(names.len(), exposed.len());
    }
}
