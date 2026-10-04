//! Reviewed mission exceptions for preserved-life accounting.

use super::{EngineInner, EntityId, LevelAssets};

/// Script-element slots are the exact actor references used by these missions.
/// These slots resolve through the entity store after loading or restoring a save;
/// they are not soldier IDs, and need no additional per-entity snapshot state.
fn required_story_kill_slot(mission: &str) -> Option<u32> {
    if mission.eq_ignore_ascii_case("S05_Yrk_EC") {
        // Guisbourne: the rescue requires defeating him in Robin's city duel.
        // Although the script tests incapacitation, town sword strikes are
        // lethal and VIP rules exclude other characters' nonlethal attacks.
        Some(143)
    } else if mission.eq_ignore_ascii_case("H10_Yor_VL") {
        // Longchamps: Hourglass advances the escape phase only after his death.
        Some(106)
    } else if mission.eq_ignore_ascii_case("H12_Not_MP") {
        // Sheriff: his death starts the sequence that sets the victory flag.
        Some(97)
    } else {
        // Scathlock and his replacement in Derby only award an optional blazon.
        None
    }
}

impl EngineInner {
    fn required_story_kill(&self, assets: &LevelAssets) -> Option<EntityId> {
        if !self
            .control
            .sim_config
            .exclude_required_kills_from_preserved_lives
        {
            return None;
        }
        let mission = assets.scripts.mission_name.as_deref()?;
        let slot = required_story_kill_slot(mission)?;
        let entity = self.world.entities.id_at_legacy_slot(slot);
        match entity {
            Some(id @ EntityId::Soldier(_)) => Some(id),
            _ => {
                tracing::warn!(
                    mission,
                    slot,
                    ?entity,
                    "required story-kill slot is not a soldier; keeping normal preserved-life accounting"
                );
                None
            }
        }
    }

    /// Count eligible living and dead hostiles for campaign totals and recruitment.
    /// Mission-authored required deaths are omitted from both sides of the ratio.
    pub(crate) fn count_soldiers_at_quit(&mut self, assets: &LevelAssets) -> (u32, u32) {
        use crate::element::{Camp, Human as _};

        let required_kill = self.required_story_kill(assets);
        let mut living = 0u32;
        let mut dead = 0u32;
        let mut living_by_camp = std::collections::BTreeMap::<Camp, u32>::new();
        for (id, s) in self.world.entities.soldiers() {
            if (self
                .control
                .sim_config
                .exclude_starting_dead_soldiers_from_preserved_lives
                && self.is_baseline_dead_npc(EntityId::Soldier(id)))
                || required_kill == Some(EntityId::Soldier(id))
            {
                continue;
            }
            if s.life_points() > 0 {
                *living_by_camp.entry(s.camp()).or_default() += 1;
            }
            if self.is_hostile_to_player_camp(s.camp()) {
                if s.life_points() > 0 {
                    living += 1;
                } else {
                    dead += 1;
                }
            }
        }
        self.mission_domain
            .mission_stat
            .reset_faction_living_counts();
        for (camp, count) in living_by_camp {
            self.mission_domain
                .mission_stat
                .set_faction_living_soldiers_at_end(camp, count);
        }
        if self
            .control
            .sim_config
            .exclude_starting_dead_soldiers_from_preserved_lives
            || self
                .control
                .sim_config
                .exclude_required_kills_from_preserved_lives
        {
            // Debriefing and history use the same eligible population as the
            // campaign percentage and recruitment calculation.
            self.mission_domain.mission_stat.living_soldier_count = living;
            self.mission_domain.mission_stat.total_soldier_count = living + dead;
        } else {
            // Preserve the legacy cumulative stat and authored load-time total
            // when both accounting extensions are disabled.
            self.mission_domain.mission_stat.living_soldier_count = self
                .mission_domain
                .mission_stat
                .living_soldier_count
                .saturating_add(living);
        }
        (living, dead)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorSoldier, Camp, ElementData, ElementKind, Entity, NpcData, SoldierData,
    };

    fn soldier(life_points: i16) -> Entity {
        let mut element = ElementData::default();
        element.kind = ElementKind::ActorSoldier;
        Entity::Soldier(ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: NpcData {
                life_points,
                ..Default::default()
            },
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..Default::default()
            },
        })
    }

    fn mission(mission: &str, required_slot: usize) -> (EngineInner, LevelAssets) {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        assets.scripts.mission_name = Some(mission.to_owned());
        let mut slots = vec![None; required_slot + 1];
        slots[0] = Some(soldier(100));
        slots[1] = Some(soldier(0)); // An avoidable casualty.
        slots[2] = Some(soldier(0)); // Already dead at startup.
        slots[required_slot] = Some(soldier(0));
        engine.world.entities = crate::entities::Entities::from_legacy_slots(slots);
        let baseline_corpse = engine.world.entities.id_at_legacy_slot(2).unwrap();
        engine
            .mission_domain
            .achievements
            .initialize_mission_baseline(0, [(baseline_corpse, true)]);
        (engine, assets)
    }

    #[test]
    fn mandatory_kill_and_starting_corpse_options_are_independent() {
        for (name, slot) in [("S05_Yrk_EC", 143), ("H10_Yor_VL", 106), ("h12_not_mp", 97)] {
            for (exclude_starting, exclude_required, expected_dead) in [
                (true, true, 1),
                (true, false, 2),
                (false, true, 2),
                (false, false, 3),
            ] {
                let (mut engine, assets) = mission(name, slot);
                engine
                    .control
                    .sim_config
                    .exclude_starting_dead_soldiers_from_preserved_lives = exclude_starting;
                engine
                    .control
                    .sim_config
                    .exclude_required_kills_from_preserved_lives = exclude_required;
                assert_eq!(
                    engine.count_soldiers_at_quit(&assets),
                    (1, expected_dead),
                    "{name}: starting={exclude_starting}, required={exclude_required}"
                );
            }
        }
    }

    #[test]
    fn optional_and_unreviewed_missions_keep_all_new_casualties() {
        for name in ["Str02_Der_MP", "H07_Not_MK", "custom_mission"] {
            let (mut engine, assets) = mission(name, 97);
            assert_eq!(engine.count_soldiers_at_quit(&assets), (1, 2), "{name}");
        }
    }

    #[test]
    fn all_avoidable_soldiers_spared_can_reach_one_hundred_percent() {
        let (mut engine, assets) = mission("H10_Yor_VL", 106);
        engine
            .world
            .entities
            .get_legacy_slot_mut(1)
            .unwrap()
            .1
            .as_soldier_mut()
            .unwrap()
            .npc
            .life_points = 100;
        assert_eq!(engine.count_soldiers_at_quit(&assets), (2, 0));
        assert_eq!(engine.mission_domain.mission_stat.living_soldier_count, 2);
        assert_eq!(engine.mission_domain.mission_stat.total_soldier_count, 2);
    }
}
