//! Atomic adoption of directly represented v48 engine-tail state.
//!
//! Global AI topology is mission-created, so saved mutable fields are applied
//! onto the initialized seek points and archery sectors rather than replacing
//! their geometry. Mission statistics are fully serialized and replace their
//! initialized counterpart.

use thiserror::Error;

use crate::{
    ai::AlertLevel,
    element::EntityId,
    engine::EngineInner,
    mission_stat::{MissionStat, PcStatName},
};

use super::{
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    post_tail::{LegacyGlobalAiState, LegacyMissionStatistics},
};

#[derive(Debug, Error)]
pub enum LegacyTailBasicAdoptError {
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
    #[error("saved global AI {field} count is {saved}, initialized mission count is {runtime}")]
    CountMismatch {
        field: &'static str,
        saved: usize,
        runtime: usize,
    },
    #[error("saved global AI field {field} has unknown alert value {value}")]
    UnknownAlert { field: &'static str, value: i32 },
}

#[derive(Clone, Debug)]
pub struct LegacyTailBasicAdoptionPlan {
    global_ai: ConvertedGlobalAi,
    mission_stat: MissionStat,
}

#[derive(Clone, Debug)]
struct ConvertedGlobalAi {
    stupid_soldiers_cheat: bool,
    seek_points: Vec<(u32, u8, bool)>,
    archery_sectors: Vec<(u16, Vec<Option<EntityId>>)>,
    green_alert_soldiers: u16,
    yellow_alert_soldiers: u16,
    red_alert_soldiers: u16,
    overall_alert_status: AlertLevel,
    overall_villain_alert_status: AlertLevel,
    saved_random_seed: i64,
}

impl LegacyTailBasicAdoptionPlan {
    pub fn preflight(
        engine: &EngineInner,
        entities: &LegacyEntityFixups,
        global_ai: &LegacyGlobalAiState,
        statistics: &LegacyMissionStatistics,
    ) -> Result<Self, LegacyTailBasicAdoptError> {
        require_count(
            "seek_points",
            global_ai.seek_points.len(),
            engine.ai.global.seek_points.len(),
        )?;
        require_count(
            "archery_sectors",
            global_ai.archery_sectors.len(),
            engine.ai.global.archery_sectors.len(),
        )?;

        let mut archery_sectors = Vec::with_capacity(global_ai.archery_sectors.len());
        for (saved, runtime) in global_ai
            .archery_sectors
            .iter()
            .zip(&engine.ai.global.archery_sectors)
        {
            require_count(
                "archery_sector.points",
                saved.point_owners.len(),
                runtime.points.len(),
            )?;
            let owners = saved
                .point_owners
                .iter()
                .map(|&owner| entities.resolve_element(owner))
                .collect::<Result<Vec<_>, _>>()?;
            archery_sectors.push((saved.number_of_owners, owners));
        }

        let global_ai = ConvertedGlobalAi {
            stupid_soldiers_cheat: global_ai.stupid_soldiers_cheat,
            seek_points: global_ai
                .seek_points
                .iter()
                .map(|point| {
                    (
                        point.frame_when_fully_interesting,
                        point.last_calculated_interest,
                        point.locked,
                    )
                })
                .collect(),
            archery_sectors,
            green_alert_soldiers: global_ai.green_alert_soldiers,
            yellow_alert_soldiers: global_ai.yellow_alert_soldiers,
            red_alert_soldiers: global_ai.red_alert_soldiers,
            overall_alert_status: alert("overall_alert_status", global_ai.overall_alert_status)?,
            overall_villain_alert_status: alert(
                "overall_villain_alert_status",
                global_ai.overall_villain_alert_status,
            )?,
            saved_random_seed: i64::from(global_ai.saved_random_seed.0),
        };
        let mission_stat = MissionStat {
            collected_money: statistics.collected_money,
            bonus_money: statistics.bonus_money,
            soldier_money: statistics.soldier_money,
            living_soldier_count: statistics.living_soldier_count,
            total_soldier_count: statistics.total_soldier_count,
            new_peasant_count: statistics.new_peasant_count,
            killed_peasant_count: statistics.killed_peasant_count,
            killed_allied_count: statistics.killed_allied_count,
            added_score: statistics.added_score,
            pc_names: statistics
                .pc_names
                .iter()
                .cloned()
                .map(|name| PcStatName::new(name, None))
                .collect(),
        };
        Ok(Self {
            global_ai,
            mission_stat,
        })
    }

    pub fn apply(self, engine: &mut EngineInner) {
        let runtime = &mut engine.ai.global;
        runtime.stupid_soldiers_cheat = self.global_ai.stupid_soldiers_cheat;
        for (point, (frame, interest, locked)) in runtime
            .seek_points
            .iter_mut()
            .zip(self.global_ai.seek_points)
        {
            point.frame_when_full_interest = frame;
            point.last_calculated_interest = interest;
            point.locked = locked;
        }
        for (sector, (number_of_owners, owners)) in runtime
            .archery_sectors
            .iter_mut()
            .zip(self.global_ai.archery_sectors)
        {
            sector.num_owners = number_of_owners;
            for (point, owner) in sector.points.iter_mut().zip(owners) {
                point.owner = owner;
            }
        }
        runtime.green_alert_soldiers = self.global_ai.green_alert_soldiers;
        runtime.yellow_alert_soldiers = self.global_ai.yellow_alert_soldiers;
        runtime.red_alert_soldiers = self.global_ai.red_alert_soldiers;
        runtime.overall_alert_status = self.global_ai.overall_alert_status;
        runtime.overall_villain_alert_status = self.global_ai.overall_villain_alert_status;
        runtime.saved_random_seed = self.global_ai.saved_random_seed;
        engine.mission_domain.mission_stat = self.mission_stat;
    }
}

fn require_count(
    field: &'static str,
    saved: usize,
    runtime: usize,
) -> Result<(), LegacyTailBasicAdoptError> {
    if saved != runtime {
        return Err(LegacyTailBasicAdoptError::CountMismatch {
            field,
            saved,
            runtime,
        });
    }
    Ok(())
}

fn alert(field: &'static str, value: i32) -> Result<AlertLevel, LegacyTailBasicAdoptError> {
    let value_u32 = u32::try_from(value)
        .map_err(|_| LegacyTailBasicAdoptError::UnknownAlert { field, value })?;
    AlertLevel::try_from(value_u32)
        .map_err(|_| LegacyTailBasicAdoptError::UnknownAlert { field, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_conversion_is_exact() {
        assert_eq!(alert("test", 0).unwrap(), AlertLevel::Green);
        assert_eq!(alert("test", 2).unwrap(), AlertLevel::Red);
        assert!(alert("test", -1).is_err());
        assert!(alert("test", 3).is_err());
    }
}
