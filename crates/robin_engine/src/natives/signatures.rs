//! Declarative registry for native IDs, signatures, provenance, and Lua exposure.
//!
//! The original game's script API defines the fixed 0..=264 native ID range
//! and the corresponding script-visible signatures.

use super::NativeFn;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum NativeYieldPolicy {
    Never,
    Always,
    Conditional,
}

/// Engine word representation shared by native adapters. Historical type
/// spellings remain in the registry for documentation and ABI identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NativeAbiType {
    Int,
    Float,
    Bool,
    Handle,
    Void,
}

impl NativeAbiType {
    /// Evaluated while constructing the static registry: unknown types cannot
    /// become a runtime-only bridge failure.
    pub const fn from_declared_type(name: &str) -> Self {
        match name.as_bytes() {
            b"int" => Self::Int,
            b"float" => Self::Float,
            b"bool" => Self::Bool,
            b"void" => Self::Void,
            b"Actor" | b"Door" | b"Patch" | b"Location" | b"SoundSource" | b"Building"
            | b"Scroll" | b"Way" => Self::Handle,
            _ => panic!("unsupported native ABI type"),
        }
    }

    pub const fn parameter_type(name: &str) -> Self {
        let kind = Self::from_declared_type(name);
        if matches!(kind, Self::Void) {
            panic!("void native parameter");
        }
        kind
    }

    pub const fn lua_name(self) -> &'static str {
        match self {
            Self::Int => "integer",
            Self::Float => "number",
            Self::Bool => "boolean",
            Self::Handle => "handle",
            Self::Void => "no value",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeParamSig {
    pub ty: &'static str,
    pub abi_type: NativeAbiType,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSignature {
    pub name: &'static str,
    pub return_type: &'static str,
    pub return_abi_type: NativeAbiType,
    pub params: &'static [NativeParamSig],
}

/// Namespace owning a native ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeNamespace {
    /// Fixed namespace used by shipped SCB bytecode.
    Original,
    /// Operations supplied by the Rust/Lua port, outside the original interface.
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

/// Spellforge-facing aliases for native ABI entries. Keeping this beside the
/// declarative registry gives every Lua implementation one canonical mapping.
pub const SPELLFORGE_NATIVE_ALIASES: &[(&str, NativeFn)] = &[
    ("StartSequence", NativeFn::Start),
    ("EndSequence", NativeFn::Thanx),
    ("SequenceScrollCameraTo", NativeFn::RecordScrollCameraTo),
    ("SequenceJumpCameraTo", NativeFn::RecordJumpCameraTo),
    ("SequenceSetZoomLevel", NativeFn::RecordSetZoom),
    ("SequenceMoveCameraTo", NativeFn::RecordMoveCameraTo),
    ("SequenceDisplayMap", NativeFn::RecordDisplayMap),
    ("SequenceMove", NativeFn::RecordMove),
    ("SequenceMoveIntoBuilding", NativeFn::RecordMoveIntoBuilding),
    ("SequenceMoveNear", NativeFn::RecordMoveNear),
    ("SequenceEnterLevel", NativeFn::RecordEnterGame),
    ("SequenceLeaveLevel", NativeFn::RecordLeaveGame),
    ("SequenceTurnTo", NativeFn::RecordTurnTo),
    ("SequencePlayAnim", NativeFn::RecordPlayAnim),
    ("SequencePlayAnimLoop", NativeFn::RecordPlayAnimLoop),
    ("SequencePlayAnimFreeze", NativeFn::RecordPlayAnimFreeze),
    ("SequencePlayDialog", NativeFn::RecordPlayDialog),
    ("SequenceReplaceAnim", NativeFn::RecordReplaceAnim),
    ("SequenceRestoreAnim", NativeFn::RecordRestoreAnim),
    ("SequenceLockAI", NativeFn::RecordLockAI),
    ("SequenceUnlockAI", NativeFn::RecordUnlockAI),
    ("SequenceLockUser", NativeFn::RecordLockUser),
    ("SequenceUnLockUser", NativeFn::RecordUnLockUser),
    ("SequenceLockCameraOn", NativeFn::RecordLockCameraOn),
    ("SequenceClearCameraLock", NativeFn::RecordClearCameraLock),
    ("SequenceTimer", NativeFn::RecordTimer),
    ("SequenceSpeak", NativeFn::RecordSpeak),
    ("SequenceSpeakPC", NativeFn::RecordSpeakPC),
    ("SequenceFreezeAll", NativeFn::RecordFreezeAll),
    ("SequenceDisplayPopupText", NativeFn::RecordDisplayPopupText),
    ("SequenceSendMessage", NativeFn::RecordSendMessage),
    (
        "SequenceSendMessageWithArguments",
        NativeFn::RecordSendMessageWithArguments,
    ),
    ("SequenceSeekActor", NativeFn::RecordSeekActor),
    ("SequenceSeekActorMessage", NativeFn::RecordSeekActorMessage),
    (
        "SequenceSeekActorMessageWithArguments",
        NativeFn::RecordSeekActorMessageWithArguments,
    ),
    (
        "SequenceActivateMobileElement",
        NativeFn::RecordActivateMobileElement,
    ),
    (
        "SequenceDeactivateMobileElement",
        NativeFn::RecordDeactivateMobileElement,
    ),
    (
        "SequenceStartMobileElement",
        NativeFn::RecordStartMobileElement,
    ),
    (
        "SequenceStopMobileElement",
        NativeFn::RecordStopMobileElement,
    ),
    ("SequenceTakeCorpse", NativeFn::RecordTakeCorpse),
    ("SequenceLeaveCorpse", NativeFn::RecordLeaveCorpse),
    ("SequenceAction", NativeFn::RecordAction),
    ("SequenceActionAvailable", NativeFn::RecordActionAvailable),
    (
        "SequenceCharacterAvailable",
        NativeFn::RecordCharacterAvailable,
    ),
    ("SequenceUnBlip", NativeFn::RecordUnBlip),
    ("AssignPatrol", NativeFn::AssignPath),
    ("AddAsSquadMember", NativeFn::AddAsSubordinate),
    ("RemoveAllSquadMembers", NativeFn::RemoveAllSubordinates),
    ("GetScrollState", NativeFn::GetScrollStatus),
    ("SetScrollState", NativeFn::SetScrollStatus),
    (
        "AreAllEnemiesInsideOutOfAction",
        NativeFn::AreAllEnemiesInsideHS,
    ),
];

/// The single declaration of every native. Consumers expand this to generate
/// the ID enum and metadata tables, so order, signatures, and Lua enumeration
/// cannot drift independently.
macro_rules! native_registry {
    ($consumer:ident) => {
        $consumer! {
            original {
            InitGlobal => ("void", [("int", "iID"), ("int", "iValue")], lua, ScriptCore, Never);
            SetGlobal => ("void", [("int", "iID"), ("int", "iValue")], lua, ScriptCore, Never);
            GetGlobal => ("int", [("int", "iID")], lua, ScriptCore, Never);
            GetActorScript => ("Actor", [("int", "iPosition")], lua, ScriptCore, Never);
            GetDoorScript => ("Door", [("int", "iPosition")], lua, ScriptCore, Never);
            GetPatchScript => ("Patch", [("int", "iPosition")], lua, ScriptCore, Never);
            GetLocationScript => ("Location", [("int", "iPosition")], lua, ScriptCore, Never);
            GetSoundSourceScript => ("SoundSource", [("int", "iPosition")], lua, ScriptCore, Never);
            GetBuildingScript => ("Building", [("int", "iPosition")], lua, ScriptCore, Never);
            GetWayScript => ("Way", [("int", "iPosition")], lua, ScriptCore, Never);
            GetActorIndex => ("int", [("Actor", "actor")], lua, ScriptCore, Never);
            GetDoorIndex => ("int", [("Door", "door")], lua, ScriptCore, Never);
            GetPatchIndex => ("int", [("Patch", "patch")], lua, ScriptCore, Never);
            GetLocationIndex => ("int", [("Location", "location")], lua, ScriptCore, Never);
            GetSoundSourceIndex => ("int", [("SoundSource", "soundsource")], lua, ScriptCore, Never);
            GetBuildingIndex => ("int", [("Building", "building")], lua, ScriptCore, Never);
            GetWayIndex => ("int", [("Way", "way")], lua, ScriptCore, Never);
            StartDialog => ("void", [("int", "iDialogue")], lua, Sequences, Never);
            ScrollCameraTo => ("bool", [("Location", "location")], lua, Sequences, Never);
            ScrollCameraSlowlyTo => ("bool", [("Location", "location"), ("float", "fSpeed")], lua, Sequences, Never);
            JumpCameraTo => ("bool", [("Location", "location")], lua, Sequences, Never);
            SetZoomLevel => ("bool", [("float", "fZoom")], lua, Sequences, Never);
            DisplayMap => ("bool", [("bool", "bDisplay")], lua, Sequences, Never);
            DisplayConsole => ("void", [], lua, Sequences, Never);
            CustomizeMinimapDisplay => ("void", [("Actor", "actor"), ("int", "iKindOfDot")], lua, Sequences, Never);
            DefineFlatTrajectoryZone => ("void", [("Location", "pLocation"), ("int", "iApex")], lua, Sequences, Never);
            AddShortBriefing => ("void", [("int", "iID"), ("bool", "bPrimary")], lua, Sequences, Never);
            DoneShortBriefing => ("void", [("int", "iID")], lua, Sequences, Never);
            ChooseVictoryDefeatText => ("void", [("int", "iID")], lua, Sequences, Never);
            ForceCheckVictory => ("void", [], lua, ScriptCore, Never);
            Start => ("bool", [], lua, ScriptCore, Never);
            Thanx => ("bool", [], lua, ScriptCore, Always);
            Then => ("int", [], lua, ScriptCore, Never);
            RecordScrollCameraTo => ("bool", [("Location", "location")], lua, Sequences, Never);
            RecordJumpCameraTo => ("bool", [("Location", "location")], lua, Sequences, Never);
            RecordSetZoom => ("bool", [("float", "fZoomLevel")], lua, Sequences, Never);
            RecordDisplayMap => ("bool", [("bool", "bDisplay")], lua, Sequences, Never);
            RecordActionAvailable => ("bool", [("Actor", "actor"), ("int", "iAction"), ("bool", "bAvailable")], lua, Sequences, Never);
            RecordCharacterAvailable => ("bool", [("Actor", "actor"), ("bool", "bAvailable")], lua, Sequences, Never);
            RecordLockCameraOn => ("bool", [("Actor", "actor")], lua, Sequences, Never);
            RecordClearCameraLock => ("bool", [], lua, Sequences, Never);
            RecordPlayDialog => ("bool", [("int", "iDialogID")], lua, Sequences, Never);
            RecordMoveCameraTo => ("bool", [("Location", "destination"), ("int", "iSpeed")], lua, Sequences, Never);
            RecordSendMessage => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode")], lua, Sequences, Never);
            RecordSendMessageWithArguments => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode"), ("int", "iArgument1"), ("int", "iArgument2")], lua, Sequences, Never);
            RecordMove => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iStyle")], lua, Sequences, Never);
            RecordEnterGame => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iDirection"), ("int", "iStyle")], lua, Sequences, Never);
            RecordLeaveGame => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iDirection"), ("int", "iStyle")], lua, Sequences, Never);
            RecordTurnTo => ("bool", [("Actor", "actor"), ("Location", "location")], lua, Sequences, Never);
            RecordPlayAnim => ("bool", [("Actor", "actor"), ("int", "iId")], lua, Sequences, Never);
            RecordPlayAnimLoop => ("bool", [("Actor", "actor"), ("int", "iId")], lua, Sequences, Never);
            RecordPlayAnimFreeze => ("bool", [("Actor", "actor"), ("int", "iId")], lua, Sequences, Never);
            RecordLockAI => ("bool", [("Actor", "actor")], lua, Sequences, Never);
            RecordUnlockAI => ("bool", [("Actor", "actor")], lua, Sequences, Never);
            RecordLockUser => ("bool", [], lua, Sequences, Never);
            RecordUnLockUser => ("bool", [], lua, Sequences, Never);
            RecordTimer => ("bool", [("int", "iFrames")], lua, Sequences, Never);
            RecordSeekActor => ("bool", [("Actor", "actor"), ("Actor", "target"), ("int", "iStyle"), ("float", "fTolerance")], lua, Sequences, Never);
            RecordStopSeek => ("bool", [("Actor", "actor")], lua, Sequences, Never);
            RecordAction => ("bool", [("Actor", "actor"), ("int", "iID"), ("int", "iValue")], lua, Sequences, Never);
            RecordReplaceAnim => ("bool", [("Actor", "actor"), ("int", "iOriginalAnim"), ("int", "iNewAnim")], lua, Sequences, Never);
            RecordRestoreAnim => ("bool", [("Actor", "actor"), ("int", "iOriginalAnim")], lua, Sequences, Never);
            RecordSpeakPC => ("bool", [("Actor", "actor"), ("int", "iRemarkID"), ("int", "iRemarkVariant")], lua, Sequences, Never);
            RecordTakeCorpse => ("int", [("Actor", "taker"), ("Actor", "corpse"), ("int", "iStyle")], lua, Sequences, Never);
            RecordMoveIntoBuilding => ("bool", [("Actor", "actor"), ("Location", "pointBeforeDoor"), ("int", "iStyle")], lua, Sequences, Never);
            RecordLeaveCorpse => ("bool", [("Actor", "actor")], lua, Sequences, Never);
            ResetAnim => ("bool", [("Actor", "actor")], lua, Sequences, Never);
            RecordStartMobileElement => ("void", [("int", "iIndex")], lua, Sequences, Never);
            RecordStopMobileElement => ("void", [("int", "iIndex")], lua, Sequences, Never);
            RecordSpeak => ("bool", [("Actor", "actor"), ("int", "iRemarkID")], lua, Sequences, Never);
            RecordSeekActorMessage => ("bool", [("Actor", "pActor"), ("Actor", "pTarget"), ("int", "iStyle"), ("float", "fDistance"), ("Actor", "pActorEvent"), ("int", "iID")], lua, Sequences, Never);
            RecordSeekActorMessageWithArguments => ("bool", [("Actor", "pActor"), ("Actor", "pTarget"), ("int", "iStyle"), ("float", "fDistance"), ("Actor", "pActorEvent"), ("int", "iID"), ("int", "iArg1"), ("int", "iArg2")], lua, Sequences, Never);
            RecordActivateMobileElement => ("void", [("int", "iIndex")], lua, Sequences, Never);
            RecordDeactivateMobileElement => ("void", [("int", "iIndex")], lua, Sequences, Never);
            ThisActor => ("Actor", [], lua, Actors, Never);
            GetNumberOfActorsInEngine => ("int", [], lua, Actors, Never);
            IsActorAnimation => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorObject => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorCharacter => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorPC => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorNPC => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorSoldier => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorCivilian => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorAnimal => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsActorCart => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsNull => ("bool", [("Actor", "actor")], lua, ScriptCore, Never);
            IsActorEqual => ("bool", [("Actor", "one"), ("Actor", "two")], lua, ScriptCore, Never);
            IsActorDead => ("bool", [("Actor", "actor")], lua, ScriptCore, Never);
            IsActorKO => ("bool", [("Actor", "actor")], lua, ScriptCore, Never);
            IsActorTied => ("bool", [("Actor", "actor")], lua, ScriptCore, Never);
            IsActorHS => ("bool", [("Actor", "actor")], lua, ScriptCore, Never);
            GetActorPosture => ("int", [("Actor", "actor")], lua, Actors, Never);
            SetActorPosture => ("void", [("Actor", "actor"), ("int", "iPosture")], lua, Actors, Always);
            GetActorDirection => ("int", [("Actor", "actor")], lua, Actors, Never);
            SetActorDirection => ("bool", [("Actor", "actor"), ("int", "iDirection")], lua, Actors, Never);
            GetActorLocation => ("Location", [("Actor", "actor")], lua, Actors, Never);
            SetActorLocation => ("bool", [("Actor", "actor"), ("Location", "location")], lua, Actors, Always);
            IsInside => ("bool", [("Actor", "actor"), ("Location", "location")], lua, Actors, Never);
            IsInsideBuilding => ("bool", [("Actor", "actor"), ("Building", "building")], lua, Actors, Never);
            UnBlip => ("bool", [("Actor", "actor")], lua, Actors, Never);
            GetMovementStyle => ("int", [("Actor", "actor")], lua, Actors, Never);
            GetCurrentAction => ("int", [("Actor", "actor")], lua, Actors, Never);
            InflictPain => ("void", [("Actor", "actor"), ("int", "iDamage"), ("bool", "bStun")], lua, Actors, Conditional);
            StopActor => ("bool", [("Actor", "actor")], lua, ScriptCore, Always);
            Sees => ("bool", [("Actor", "actorNPC"), ("Actor", "actorTarget")], lua, Actors, Never);
            EnableViewCone => ("void", [("Actor", "actor")], lua, Actors, Conditional);
            GetOutlineDisplay => ("bool", [], lua, Sequences, Never);
            SetOutlineDisplay => ("void", [("bool", "bDisplay")], lua, Sequences, Never);
            PrototypeFilterEvent => ("bool", [("Actor", "prototype"), ("Actor", "actorSource"), ("int", "iEvent")], lua, Actors, Always);
            SendMessage => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode")], lua, Actors, Always);
            SendMessageWithArguments => ("void", [("Actor", "actReceiver"), ("int", "iMessageCode"), ("int", "iArgument1"), ("int", "iArgument2")], lua, Actors, Always);
            God => ("Actor", [], lua, ScriptCore, Never);
            Select => ("bool", [("int", "selectCode")], lua, ScriptCore, Never);
            Deactivate => ("bool", [("Actor", "actor")], lua, ScriptCore, Never);
            Activate => ("bool", [("Actor", "actor")], lua, ScriptCore, Never);
            SetActionAvailable => ("bool", [("Actor", "actor"), ("int", "iAction"), ("bool", "bAvailable")], lua, Actors, Never);
            IsActionAvailable => ("bool", [("Actor", "actor"), ("int", "iAction")], lua, Actors, Never);
            SetPersistentProperty => ("bool", [("Actor", "actor"), ("int", "iProperty"), ("int", "iAmount")], lua, Actors, Conditional);
            GetPersistentProperty => ("int", [("Actor", "actor"), ("int", "iProperty")], lua, Actors, Never);
            IsAnyCivilianDead => ("bool", [], lua, Actors, Never);
            IsAnyEnemyDead => ("bool", [], lua, Actors, Never);
            GetOverallEnemyAlert => ("int", [], lua, Actors, Never);
            GetOverallCivilianAlert => ("int", [], lua, Actors, Never);
            SetAIAlertStatus => ("bool", [("Actor", "actor"), ("int", "iStatus")], lua, Ai, Never);
            GetAIAlertStatus => ("int", [("Actor", "actor")], lua, Ai, Never);
            SetAIState => ("bool", [("Actor", "actor"), ("int", "iState")], lua, Ai, Conditional);
            GetAIState => ("int", [("Actor", "actor")], lua, Ai, Never);
            SetAIAttitude => ("bool", [("Actor", "actor"), ("int", "iAttitude")], lua, Ai, Never);
            GetAIAttitude => ("int", [("Actor", "actor")], lua, Ai, Never);
            SetAILevel => ("bool", [("Actor", "actor"), ("int", "iProperty"), ("int", "iLevel")], lua, Ai, Never);
            StareActor => ("void", [("Actor", "actor"), ("Actor", "actorTarget"), ("bool", "bTurnSprite")], lua, Ai, Always);
            StareLocation => ("void", [("Actor", "actor"), ("Location", "locPoint"), ("bool", "bTurnSprite")], lua, Ai, Always);
            AssignPath => ("void", [("Actor", "actor"), ("Way", "myWay")], lua, Ai, Always);
            AssignPost => ("void", [("Actor", "actor"), ("Location", "location"), ("int", "iDirection")], lua, Ai, Always);
            LockAI => ("void", [("Actor", "actor"), ("bool", "bRememberEvents")], lua, ScriptCore, Always);
            UnlockAI => ("void", [("Actor", "actor")], lua, ScriptCore, Always);
            ForceBattleDecision => ("void", [("Actor", "actor"), ("int", "iDecision")], lua, Ai, Never);
            MakeNoise => ("void", [("Location", "location"), ("int", "iTypeID")], lua, Ai, Never);
            Freeze => ("void", [("Actor", "actor"), ("bool", "bFrozen")], lua, ScriptCore, Never);
            FreezeAll => ("void", [("bool", "bFrozen")], lua, ScriptCore, Never);
            SetPathWalkingStyle => ("void", [("Actor", "NPC"), ("int", "i0Walking1Running2Backward")], lua, Ai, Always);
            GetSoldierRank => ("int", [("Actor", "actor")], lua, Ai, Never);
            IsAnimationActive => ("bool", [("Actor", "actor")], lua, World, Never);
            SetAnimationState => ("bool", [("Actor", "actor"), ("bool", "bState")], lua, World, Never);
            IsPatchApplied => ("bool", [("Patch", "patch")], lua, World, Never);
            ApplyPatch => ("bool", [("Patch", "patch")], lua, World, Never);
            ResetPatch => ("bool", [("Patch", "patch")], lua, World, Never);
            SuspendAllSoundSources => ("bool", [], lua, World, Never);
            ResumeAllSoundSources => ("bool", [], lua, World, Never);
            ActivateSoundSource => ("bool", [("SoundSource", "source")], lua, World, Never);
            DeactivateSoundSource => ("bool", [("SoundSource", "source")], lua, World, Never);
            DestroySoundSource => ("bool", [("SoundSource", "source")], lua, World, Never);
            CleanFromHisBuildingBeforeTeleport => ("bool", [("Actor", "actor")], no_lua, World, Never);
            CleanFromScriptZoneBeforeTeleport => ("bool", [("Actor", "actor"), ("Location", "cestLaZone")], no_lua, World, Never);
            AddToScriptZoneAfterTeleport => ("bool", [("Actor", "actor"), ("Location", "cestLaZone")], no_lua, World, Never);
            SetCorpseExistsInBuilding => ("void", [("Actor", "pActor")], no_lua, World, Never);
            // TODO(original parity): available script API declarations
            // spell ID 156 as `PutActorInBulding`. Rust retains its established
            // corrected public spelling.
            PutActorInBuilding => ("void", [("Actor", "actor"), ("Building", "building")], no_lua, World, Never);
            SetBuildingActive => ("void", [("Building", "building"), ("bool", "bActive")], lua, World, Never);
            GetAnyActorInsideBuilding => ("Actor", [("Building", "building")], lua, World, Never);
            NoWhere => ("Location", [], lua, ScriptCore, Never);
            GetDistance => ("int", [("Location", "here"), ("Location", "there")], lua, ScriptCore, Never);
            Rand => ("int", [("int", "iMaximum")], lua, ScriptCore, Never);
            PrintConsole => ("void", [("int", "iValue")], lua, ScriptCore, Never);
            GetSizeOfMissionTeam => ("int", [], lua, ScriptCore, Never);
            GetPCFromMissionTeam => ("Actor", [("int", "ulPC")], lua, Campaign, Never);
            AddPCToMissionTeam => ("void", [("Actor", "actor")], lua, Campaign, Never);
            RemovePCFromMissionTeam => ("void", [("Actor", "actor")], lua, Campaign, Never);
            GetNumberOfObligatoryPCsInMissionTeam => ("int", [], lua, Campaign, Never);
            GetObligatoryPCFromMissionTeam => ("Actor", [("int", "ulPC")], lua, Campaign, Never);
            IsPCObligatoryInMissionTeam => ("bool", [("Actor", "actor")], lua, Campaign, Never);
            IsMissionTeamValid => ("bool", [], lua, ScriptCore, Never);
            GetLastPlayedMission => ("int", [], lua, ScriptCore, Never);
            GetNextPlayedMission => ("int", [], lua, ScriptCore, Never);
            IsMenToBlazonConversionMode => ("bool", [], no_lua, Campaign, Never);
            GetNumberOfBeamMes => ("int", [], no_lua, Campaign, Never);
            MoveBeamMe => ("void", [("int", "iIndex"), ("Location", "pLocation")], no_lua, Campaign, Never);
            SetCompanyNumber => ("void", [("Actor", "pActor"), ("int", "iNumber")], lua, Actors, Never);
            SetAlwaysAttentive => ("void", [("Actor", "actor"), ("bool", "bYes")], lua, Actors, Conditional);
            WinBlazon => ("void", [("Actor", "blazon")], lua, Campaign, Never);
            LoseBlazon => ("void", [("Actor", "blazon")], lua, Campaign, Never);
            SetInvisible => ("void", [("Actor", "actor"), ("bool", "bHollow")], lua, Actors, Never);
            IsInvisible => ("bool", [("Actor", "actor")], lua, Actors, Never);
            IsDoorLockedPC => ("bool", [("Door", "door")], lua, World, Never);
            IsDoorUnlockable => ("bool", [("Door", "door")], lua, World, Never);
            IsDoorLockedNPCCivilian => ("bool", [("Door", "door")], lua, World, Never);
            IsDoorLockedNPCVillain => ("bool", [("Door", "door")], lua, World, Never);
            SetDoorLockedPC => ("void", [("Door", "door"), ("bool", "bState")], lua, World, Never);
            SetDoorUnlockable => ("void", [("Door", "door"), ("bool", "bState")], lua, World, Never);
            SetDoorLockedNPCCivilian => ("void", [("Door", "door"), ("bool", "bState")], lua, World, Never);
            SetDoorLockedNPCVillain => ("void", [("Door", "door"), ("bool", "bState")], lua, World, Never);
            SetDoorSpecialAutorisation => ("void", [("Door", "door"), ("Actor", "actor"), ("bool", "bDirect")], lua, World, Never);
            ActivateDoorMouseSector => ("void", [("bool", "bActive"), ("Door", "door")], lua, World, Never);
            ThisScroll => ("Actor", [], lua, World, Never);
            GetScrollStatus => ("int", [("Actor", "scroll")], lua, World, Never);
            SetScrollStatus => ("void", [("Actor", "scroll"), ("int", "iStatus")], lua, World, Never);
            GetCustomCampaignValue => ("int", [("int", "iIndex")], lua, ScriptCore, Never);
            SetCustomCampaignValue => ("void", [("int", "iIndex"), ("int", "iValue")], lua, ScriptCore, Never);
            GetCustomNPCValue => ("int", [("Actor", "actor"), ("int", "iIndex")], lua, ScriptCore, Never);
            SetCustomNPCValue => ("void", [("Actor", "actor"), ("int", "iIndex"), ("int", "iValue")], lua, ScriptCore, Never);
            RegisterAsProductionSector => ("void", [("int", "iType"), ("Location", "sector"), ("int", "iProductionSpeed")], lua, Campaign, Never);
            AddProductionPoint => ("void", [("int", "iType"), ("Location", "point")], lua, Campaign, Never);
            GetActorForBeamMe => ("Actor", [("int", "iIndex")], lua, Campaign, Never);
            DisplayPopupText => ("void", [("int", "iPopupTextID")], lua, Sequences, Never);
            RecordDisplayPopupText => ("void", [("int", "iPopupTextID")], lua, Sequences, Never);
            GetNumberOfActorsInSector => ("int", [("Location", "loc")], lua, Campaign, Never);
            GetActorInSector => ("Actor", [("Location", "loc"), ("int", "iIndex")], lua, Campaign, Never);
            BitwiseAnd => ("int", [("int", "i"), ("int", "j")], lua, ScriptCore, Never);
            BitwiseOr => ("int", [("int", "i"), ("int", "j")], lua, ScriptCore, Never);
            BitwiseXor => ("int", [("int", "i"), ("int", "j")], lua, ScriptCore, Never);
            HasPCAction => ("bool", [("Actor", "actPC"), ("int", "iActionCode")], lua, Actors, Never);
            HasAnyPCAction => ("bool", [("int", "iActionCode")], lua, Actors, Never);
            GetRobin => ("Actor", [], lua, Campaign, Never);
            RecordMoveNear => ("bool", [("Actor", "actor"), ("Location", "location"), ("int", "iStyle"), ("int", "iTolerance")], lua, Sequences, Never);
            ComputeLocationBetween => ("Location", [("Location", "locA"), ("Location", "locB"), ("float", "fLambdaBetweenZeroAndOne")], lua, Campaign, Never);
            DeclareAsCombatTrainer => ("void", [("Actor", "actor")], lua, Ai, Never);
            GetRelic => ("Actor", [("int", "iID")], lua, Campaign, Never);
            GetNumberOfPCs => ("int", [], lua, ScriptCore, Never);
            GetPC => ("Actor", [("int", "i")], lua, ScriptCore, Never);
            AddAsSubordinate => ("void", [("Actor", "actChief"), ("Actor", "actSubordinate")], lua, Ai, Never);
            RemoveAllSubordinates => ("void", [("Actor", "actChief")], lua, Ai, Always);
            SwitchToAlertPath => ("void", [("Actor", "actSoldier")], lua, Ai, Always);
            IsActorRider => ("bool", [("Actor", "actWhoever")], lua, Actors, Never);
            IsUnblipped => ("bool", [("Actor", "actWhoever")], lua, Actors, Never);
            IsBlazonWon => ("bool", [("Actor", "blazon")], lua, Campaign, Never);
            AddRepulsivePoint => ("int", [("Location", "location"), ("float", "fRadius"), ("float", "fActionRadius"), ("int", "iFlags")], lua, Ai, Never);
            SetViewRadius => ("void", [("int", "iRadius")], lua, Sequences, Never);
            RecordFreezeAll => ("void", [("bool", "bFreeze")], lua, Sequences, Never);
            DeleteRepulsivePoint => ("void", [("int", "iID")], lua, Ai, Never);
            SetNPCEmoticon => ("void", [("Actor", "actNPC"), ("int", "iEmoticonType"), ("int", "iTime")], lua, Ai, Never);
            ConfiscateMoney => ("void", [("Actor", "actCapitalist")], lua, Campaign, Never);
            AreAllPCsInside => ("bool", [("Location", "location")], lua, World, Never);
            AreAllEnemiesInsideHS => ("bool", [("Location", "locZone")], lua, World, Never);
            AddPCToGang => ("void", [("Actor", "actor")], lua, Campaign, Never);
            AttachScrollToNPC => ("void", [("Actor", "actNPC"), ("Actor", "scroll")], lua, World, Never);
            AreAllBlazonsWon => ("bool", [], lua, ScriptCore, Never);
            IsBonusItemPickedUp => ("bool", [("Actor", "actItem")], lua, Campaign, Never);
            GetRansomMoney => ("int", [], lua, ScriptCore, Never);
            SetRansomMoney => ("void", [("int", "iRansomMoneyAmount")], lua, ScriptCore, Never);
            GetDifficultyLevel => ("int", [], lua, ScriptCore, Never);
            DisplaySherwoodReport => ("void", [], lua, Sequences, Never);
            IsActorActive => ("bool", [("Actor", "actor")], lua, Actors, Never);
            AddFarmerToGang => ("void", [("int", "iType"), ("int", "iExperienceSword"), ("int", "iExperienceBow")], lua, Campaign, Never);
            SetExperiences => ("void", [("Actor", "actor"), ("int", "iExperienceSword"), ("int", "iExperienceBow")], lua, Campaign, Never);
            RecordUnBlip => ("bool", [("Actor", "pActor")], lua, Sequences, Never);
            SetPatchAnimationActive => ("void", [("Patch", "patch"), ("bool", "bActive")], lua, World, Never);
            GetNumberOfPCsAlive => ("int", [], lua, ScriptCore, Never);
            AreAllPCsAliveInside => ("bool", [("Location", "location")], lua, World, Never);
            TransformHandleTargetToTakeTarget => ("void", [("Actor", "actTarget")], lua, Campaign, Never);
            IsPCSelected => ("bool", [("Actor", "actPC")], lua, Campaign, Never);
            GetNumberOfSelectedPCs => ("int", [], lua, Campaign, Never);
            GetSelectedPC => ("Actor", [("int", "iIndex")], lua, Campaign, Never);
            PlayTrapJingle => ("void", [], lua, Sequences, Never);
            MakePCCrouched => ("void", [("Actor", "actPC")], lua, Actors, Never);
            HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood => ("bool", [("int", "iActionCode")], lua, ScriptCore, Never);
            LockPatch => ("void", [("Patch", "patch"), ("bool", "bLocked")], lua, World, Never);
            HasAnyActivePCAction => ("bool", [("int", "iActionCode")], lua, Actors, Never);
            GetPCType => ("int", [("Actor", "actPC")], lua, Campaign, Never);
            SelectActorPC => ("void", [("Actor", "actPCOrGodForAllPCs"), ("bool", "bSelectOrUnselect")], lua, Campaign, Never);
            HasAnyActionSelected => ("bool", [("Actor", "actPC")], lua, Actors, Never);
            GetActorActionState => ("int", [("Actor", "actor")], lua, Actors, Never);
            SetActorActionState => ("void", [("Actor", "actor"), ("int", "iActionState")], lua, Actors, Always);
            SecretAgentsAreBackInSherwood => ("bool", [], lua, ScriptCore, Never);
            FadeToBlack => ("void", [("int", "iSpeed")], lua, Sequences, Never);
            LinkTargetToFX => ("void", [("Actor", "actTarget"), ("Actor", "actFX")], lua, World, Never);
            ForbidNPCRemark => ("void", [("Actor", "actNPC"), ("int", "iRemark"), ("bool", "bTrueMeansForbidFalseMeansAllow")], lua, Ai, Never);
            }
            rust_extensions {
            Reveal => ("int", [("Actor", "actActor")], lua, Campaign, Never);
            AddObjective => ("int", [("int", "iObjectiveID"), ("bool", "bIsMainObjective")], lua, Campaign, Never);
            CompleteObjective => ("int", [("int", "iObjectiveID")], lua, Campaign, Never);
            IsActorOutOfAction => ("bool", [("Actor", "actActor")], lua, Campaign, Never);
            SetPatrolShouldRun => ("void", [("Actor", "actPatrolLeader"), ("bool", "bShouldRun")], lua, Campaign, Never);
            SequenceReveal => ("int", [("Actor", "actActor")], lua, Campaign, Never);
            GetActorAllegiance => ("int", [("Actor", "actActor")], lua, ScriptCore, Never);
            GetDiplomacyRelationship => ("int", [("int", "iFirst"), ("int", "iSecond")], lua, ScriptCore, Never);
            SetDiplomacyRelationship => ("void", [("int", "iFirst"), ("int", "iSecond"), ("int", "iRelationship")], lua, ScriptCore, Never);
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
            return_abi_type: NativeAbiType::from_declared_type($return_type),
            params: &[
                $(NativeParamSig { ty: $param_type, abi_type: NativeAbiType::parameter_type($param_type), name: $param_name }),*
            ],
        }
    };
}

macro_rules! define_native_metadata {
    (
        original {
            $( $original:ident => ($original_return:literal, $original_params:tt, $original_lua:ident, $original_domain:ident, $original_yield:ident); )*
        }
        rust_extensions {
            $( $extension:ident => ($extension_return:literal, $extension_params:tt, $extension_lua:ident, $extension_domain:ident, $extension_yield:ident); )*
        }
    ) => {
        impl NativeFn {
            pub(super) fn domain(self) -> super::dispatch::NativeDomain {
                match self {
                    $(Self::$original => super::dispatch::NativeDomain::$original_domain,)*
                    $(Self::$extension => super::dispatch::NativeDomain::$extension_domain,)*
                }
            }

            pub(super) fn yield_policy(self) -> NativeYieldPolicy {
                match self {
                    $(Self::$original => NativeYieldPolicy::$original_yield,)*
                    $(Self::$extension => NativeYieldPolicy::$extension_yield,)*
                }
            }

            /// Whether any valid call can suspend into the engine driver.
            pub fn may_yield(self) -> bool {
                !matches!(self.yield_policy(), NativeYieldPolicy::Never)
            }
        }

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
    let position = if index < super::ORIGINAL_NATIVE_COUNT {
        index
    } else {
        index
            .checked_sub(super::RUST_EXTENSION_NATIVE_START)?
            .checked_add(super::ORIGINAL_NATIVE_COUNT)?
    };
    NATIVE_REGISTRY
        .get(usize::try_from(position).ok()?)
        .filter(|definition| definition.native as u32 == index)
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
    fn synchronous_ai_natives_are_rejected_before_direct_host_mutation() {
        for native in [
            NativeFn::StareActor,
            NativeFn::StareLocation,
            NativeFn::AssignPath,
            NativeFn::AssignPost,
            NativeFn::SetPathWalkingStyle,
            NativeFn::SwitchToAlertPath,
            NativeFn::RemoveAllSubordinates,
            NativeFn::StopActor,
            NativeFn::LockAI,
            NativeFn::UnlockAI,
        ] {
            assert_eq!(native.yield_policy(), NativeYieldPolicy::Always, "{native}");
            assert!(native.may_yield(), "{native}");
        }
        assert!(!NativeFn::GetGlobal.may_yield());
        assert_eq!(
            NativeFn::SetAIState.yield_policy(),
            NativeYieldPolicy::Conditional
        );
    }

    #[test]
    fn native_lookup_rejects_out_of_range_ids() {
        assert!(native_definition_by_index(NATIVE_REGISTRY.len() as u32).is_none());
        assert!(native_definition_by_index(u32::MAX).is_none());
        for definition in NATIVE_REGISTRY {
            assert_eq!(
                native_definition_by_index(definition.native as u32),
                Some(definition)
            );
        }
    }

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
            let signature = &definition.signature;
            assert_eq!(
                signature.return_abi_type,
                NativeAbiType::from_declared_type(signature.return_type)
            );
            for parameter in signature.params {
                assert_eq!(
                    parameter.abi_type,
                    NativeAbiType::from_declared_type(parameter.ty)
                );
                assert_ne!(
                    parameter.abi_type,
                    NativeAbiType::Void,
                    "void parameter in {}",
                    signature.name
                );
            }
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
