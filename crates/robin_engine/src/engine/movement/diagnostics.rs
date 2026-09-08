//! Read-only movement boundary diagnostic selection. No simulation capability
//! is accepted here: filters cannot draw RNG or mutate the observed owner.

use crate::element::EntityId;
use serde::{Deserialize, Serialize};

#[inline]
pub(super) fn debug_post_seek_handoff_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_POST_SEEK_HANDOFF").is_some()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum MovementPopGoalOwnerKind {
    Pc,
    Soldier,
    Civilian,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct MovementPopGoalOwnerDebugConfig {
    frame: u32,
    kind: MovementPopGoalOwnerKind,
    index: u32,
}

fn movement_pop_goal_owner_debug_config() -> Option<&'static MovementPopGoalOwnerDebugConfig> {
    static CONFIG: std::sync::OnceLock<Option<MovementPopGoalOwnerDebugConfig>> =
        std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF")?;
            let frame = std::env::var("PARITY_DEBUG_GOAL_OWNER_FRAME").unwrap_or_else(|_| {
                panic!(
                    "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER_FRAME=FRAME"
                )
            });
            let frame = frame.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid PARITY_DEBUG_GOAL_OWNER_FRAME={frame:?}: {error}")
            });
            let owner = std::env::var("PARITY_DEBUG_GOAL_OWNER").unwrap_or_else(|_| {
                panic!(
                    "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER=pc|soldier|civilian:INDEX"
                )
            });
            let (kind, index) = owner.split_once(':').unwrap_or_else(|| {
                panic!("PARITY_DEBUG_GOAL_OWNER must look like pc|soldier|civilian:INDEX")
            });
            let kind = match kind {
                "pc" => MovementPopGoalOwnerKind::Pc,
                "soldier" => MovementPopGoalOwnerKind::Soldier,
                "civilian" => MovementPopGoalOwnerKind::Civilian,
                unsupported => {
                    panic!("PARITY_DEBUG_GOAL_OWNER has unsupported kind {unsupported:?}")
                }
            };
            let index = index.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid PARITY_DEBUG_GOAL_OWNER={owner:?}: {error}")
            });
            Some(MovementPopGoalOwnerDebugConfig { frame, kind, index })
        })
        .as_ref()
}

pub(super) fn movement_pop_goal_owner_debug_matches(frame: u32, owner: EntityId) -> bool {
    let Some(config) = movement_pop_goal_owner_debug_config() else {
        return false;
    };
    config.frame == frame
        && config.index == owner.index()
        && matches!(
            (config.kind, owner),
            (MovementPopGoalOwnerKind::Pc, EntityId::Pc(_))
                | (MovementPopGoalOwnerKind::Soldier, EntityId::Soldier(_))
                | (MovementPopGoalOwnerKind::Civilian, EntityId::Civilian(_))
        )
}
