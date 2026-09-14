//! AI system — core types, state machine, stimulus processing.
//!
//! Defines the enums, flags, data structures, and base AI controller
//! that drive all NPC behavior. The actual behavior implementations live in
//! [`ai_enemy`](super::ai_enemy) (villain/soldier AI) and
//! [`ai_friendly`](super::ai_friendly) (civilian AI).

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::EntityId;

pub(crate) mod parity_gate;
pub(crate) mod parity_trace;
mod types;
pub(crate) use types::optional_ai_handle;
pub use types::{
    AiEntityHandle, AiLockFlags, AiStateChangeSource, AlertFlags, DoorHandle, DutyFlags,
    ElementHandle, GotoFlags, HALF_MAX_ATT_VALUE, HumanHandle, IntoOptionalAiHandle, MAX_ATT_VALUE,
    NpcHandle, ObjectHandle, QUARTER_MAX_ATT_VALUE, RemarkTargetFlags, SectorHandle, SpeechFlags,
    THREE_QUARTERS_MAX_ATT_VALUE,
};

mod macro_patrol;
pub use macro_patrol::{
    DetachedPatrolPathStatus, ForecastInput, ForecastedDestination, MacroOpcode, PATROL_SPEED_BASE,
    PATROL_SPEED_DIVISOR, PathHistoryEntry, PathId, PatrolPath, Position,
    PreparedForecastDestination, forecast_destination_for_ia, prepare_forecast_destination_for_ia,
};

mod model;
pub(crate) use model::cache_npc_villain_authorized_direct;
pub use model::{
    AMBUSH_BOX_HALF_SIZE, AiState, AlertLevel, AlertSoldiersFailureContinuation, AmbushPoint,
    Attitude, CombatInfo, Curiosity, Decision, Detection, DoorCombatInfo, DoorSeekInfo,
    EmoticonType, ForbiddenRemark, Hint, LogLine, LogLineType, LookDirection, Noise, NoiseOrigin,
    NoiseType, OriginalEnumWord, PanicRequest, PatrolAssignment, PointArchery,
    ProbabilityDistribution, Question, ReconnaissanceReport, Remark, ReportType, RepulsivePoint,
    ScreenRemark, ScriptSeekAreaRequest, SectorArchery, SeekPoint, SeekPointDirection, Stimulus,
    StimulusCategory, StimulusInfo, StimulusType, StolenObject, StoredEnumWord, Substate,
    TargetType, ViewCone, stimulus_to_ai_event_code,
};

mod contexts;
pub use contexts::{
    AI_DOOR_RALLY_POINT_DISTANCE, AiGlobalState, AntagonistInfo, DoorRallyPoint, House,
    ReinforcementDoorInfo,
};
pub(crate) use contexts::{ai_position_to_point_3d, enemy_lift_approach_for_position};

mod duty;
mod effects;
pub(crate) use duty::{
    BodyReaction, EnemyObservation, EnemyRecovery, MoneyFightOperation, OfficerAlertCaller,
};
pub use effects::AiSpeechAttempt;

mod controller;
pub mod persisted;
mod role;
pub use controller::AiController;
pub(crate) use controller::WillStopCaller;
pub(crate) use role::AiRole;

#[cfg(test)]
mod tests;
