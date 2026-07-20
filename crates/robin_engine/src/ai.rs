//! AI system — core types, state machine, stimulus processing.
//!
//! Defines the enums, flags, data structures, and base AI controller
//! that drive all NPC behavior. The actual behavior implementations live in
//! [`ai_enemy`](super::ai_enemy) (villain/soldier AI) and
//! [`ai_friendly`](super::ai_friendly) (civilian AI).

use std::sync::Arc;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::EntityId;
use crate::order::AiOrderIntent;

mod types;
pub use types::{
    AiLockFlags, AiStateChangeSource, AlertFlags, CharlySeekerTarget, DoorHandle, DutyFlags,
    ElementHandle, EnterSwordfightRequest, GotoFlags, HALF_MAX_ATT_VALUE, HumanHandle,
    MAX_ATT_VALUE, NpcHandle, ObjectHandle, QUARTER_MAX_ATT_VALUE, RemarkTargetFlags, SectorHandle,
    SpeechFlags, THREE_QUARTERS_MAX_ATT_VALUE,
};

mod macro_patrol;
pub use macro_patrol::{
    ForecastInput, ForecastedDestination, MacroOpcode, PATROL_SPEED_BASE, PATROL_SPEED_DIVISOR,
    PathHistoryEntry, PathId, PatrolPath, Position, forecast_destination_for_ia,
};

mod model;
pub(crate) use model::cache_npc_villain_authorized_direct;
pub use model::{
    AMBUSH_BOX_HALF_SIZE, AiState, AlertLevel, AmbushPoint, Attitude, CombatInfo, CrossNpcAction,
    Curiosity, Decision, Detection, DoorCombatInfo, DoorSeekInfo, EmoticonType, ForbiddenRemark,
    Hint, LogLine, LogLineType, LookDirection, Noise, NoiseType, PanicRequest, PatrolAssignment,
    PointArchery, ProbabilityDistribution, Question, ReconnaissanceReport, Remark, ReportType,
    RepulsivePoint, ScreenRemark, ScriptSeekAreaRequest, SectorArchery, SeekPoint,
    SeekPointDirection, Stimulus, StimulusCategory, StimulusInfo, StimulusType, StolenObject,
    Substate, TargetType, ViewCone, stimulus_to_ai_event_code,
};

mod contexts;
pub use contexts::{
    AI_DOOR_RALLY_POINT_DISTANCE, AiContext, AiGlobalState, AiPerTickData, AntagonistInfo,
    DoorRallyPoint, FriendSwapCandidate, House, MyExitDoorInfo, PhalanxEnemySnapshot,
    PhalanxMemberThemList, ReinforcementDoorInfo, SleepingEnemyInfo,
};

mod effects;
#[allow(unused_imports)]
pub(crate) use effects::{AiActorCoreEffects, AiActorPreemptionEffects};
pub use effects::{
    AiActorOutbox, AiDetectionOutbox, AiMusicOutbox, AiOutbox, AiOwnerWork, AiPatrolOutbox,
    AiRecoveryOutbox, AiReentrantOutbox, AiSpeechAttempt, AiStateChangeNotification,
    ArcheryReservationRelease, AttentiveModeEffect, GuardedPcEffect, InitStateSideEffects,
    ReservedShootingPoint,
};

mod controller;
pub use controller::{AiController, ConsiderationAccumulator};

#[cfg(test)]
mod tests;
