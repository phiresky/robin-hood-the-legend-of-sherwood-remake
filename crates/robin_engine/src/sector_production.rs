use crate::element::ObjectType;
use crate::entities::Entities;
use crate::profiles::Action;
use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Type {
    MakeArrow,
    MakePurse,
    MakeStone,
    MakeApple,
    MakeAle,
    MakeLamblegg,
    MakePlant,
    MakeNet,
    MakeWaspNest,
    TrainBow,
    TrainHandToHand,
    Heal,
    Relic,
    #[default]
    Unknown,
}

impl Type {
    /// Map the production type to the raw `BonusType` ordinal used by
    /// mission-file bonus entries (and by `bonus_type_to_sprite_asset`).
    /// Returns `None` for non-MAKE_* types.
    pub fn bonus_raw_type(self) -> Option<u16> {
        match self {
            Type::MakeArrow => Some(0),
            Type::MakeStone => Some(1),
            Type::MakeApple => Some(2),
            Type::MakeAle => Some(3),
            Type::MakeLamblegg => Some(4),
            Type::MakePlant => Some(5),
            Type::MakeNet => Some(6),
            Type::MakeWaspNest => Some(7),
            Type::MakePurse => Some(8),
            _ => None,
        }
    }

    pub fn from_script_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Type::MakeArrow),
            1 => Some(Type::MakePurse),
            2 => Some(Type::MakeStone),
            3 => Some(Type::MakeApple),
            4 => Some(Type::MakeAle),
            5 => Some(Type::MakeLamblegg),
            6 => Some(Type::MakePlant),
            7 => Some(Type::MakeNet),
            8 => Some(Type::MakeWaspNest),
            9 => Some(Type::TrainBow),
            10 => Some(Type::TrainHandToHand),
            11 => Some(Type::Heal),
            12 => Some(Type::Relic),
            _ => None,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub layer: u16,
    pub sector: u16,
    pub obstacle: u16,
}

/// A PC captured in a production sector when the player exits Sherwood,
/// restored to the same position on the next Sherwood visit.  A
/// `(pc_description, position, obstacle)` triple.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Occupant {
    /// Index into `Campaign::characters`.
    pub pc_description_idx: usize,
    pub x: f32,
    pub y: f32,
    /// `0xFFFF` means "no obstacle recorded".
    pub obstacle: u16,
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SectorProduction {
    pub prod_type: Type,
    /// Canonical script-zone attachment. The original stores a non-null
    /// `RHSectorScript*` and asserts both single attachment and attachment
    /// before accepting production points.
    pub script_zone: Option<usize>,
    pub speed: u16,
    pub production_points: Vec<Point>,
    /// Captured Sherwood-side occupants.
    pub occupants: Vec<Occupant>,
    pub amount: u16,
    pub produced_amount: u16,
    pub max_amount_reached: bool,
}

/// Read-only projection of one MAKE_* sector after a production cycle.
///
/// This is deliberately produced by [`SectorProduction::forecast`] and the
/// same private calculation used by [`SectorProduction::update_amount`].  UI
/// code must not reproduce the production formula: doing so would eventually
/// make the forecast disagree with campaign state at truncation and saturation
/// boundaries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductionForecast {
    pub prod_type: Type,
    /// Authored duration of the projected mission/cycle, in seconds.
    pub cycle_seconds: u16,
    /// PCs currently assigned to this sector, including its specialist.
    pub worker_count: usize,
    /// Whether the matching specialist supplies the 1.5x multiplier.
    pub has_specialist: bool,
    /// Script-authored production speed (the unscaled `RegisterProductionSector`
    /// value). Exposed so presentation and automation can explain the input.
    pub speed: u16,
    /// Consumable raw-material units required by the shipped production rule.
    ///
    /// The original has no raw-material inventory or recipe input: all MAKE_*
    /// sectors therefore report zero. Keeping the constraint explicit avoids
    /// presenting workers as though they consume an undocumented resource and
    /// leaves a typed seam for data-driven recipes later.
    pub raw_materials_required: u16,
    pub current_stock: u16,
    /// Number of item slots represented by production points (five per point).
    pub storage_capacity: u32,
    /// Exact amount credited by the production formula for this cycle.
    pub produced: u16,
    /// Stored amount after the campaign's `u16::saturating_add` rule.
    pub projected_total: u16,
    /// Portion of `projected_total` that cannot be placed at production points.
    pub overflow: u32,
}

impl SectorProduction {
    pub fn new(prod_type: Type) -> Self {
        Self {
            prod_type,
            ..Default::default()
        }
    }

    pub fn get_occupant_count(&self) -> usize {
        self.occupants.len()
    }

    pub fn get_produced_amount(&self) -> u16 {
        self.produced_amount
    }

    pub fn is_max_reached(&self) -> bool {
        self.max_amount_reached
    }

    /// Maximum stock that can be materialized in Sherwood (five items for
    /// each script-registered production point).
    pub fn storage_capacity(&self) -> u32 {
        u32::try_from(self.production_points.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(5)
    }

    /// Project this sector through one won mission without changing campaign
    /// state. `mission_length` uses the same authored seconds value consumed by
    /// production when the player returns to Sherwood.
    pub fn forecast(&self, mission_length: u16, has_specialist: bool) -> ProductionForecast {
        let produced = self.calculate_produced_amount(mission_length, has_specialist);
        let projected_total = self.amount.saturating_add(produced);
        let storage_capacity = self.storage_capacity();
        ProductionForecast {
            prod_type: self.prod_type,
            cycle_seconds: mission_length,
            worker_count: self.occupants.len(),
            has_specialist,
            speed: self.speed,
            raw_materials_required: 0,
            current_stock: self.amount,
            storage_capacity,
            produced,
            projected_total,
            overflow: u32::from(projected_total).saturating_sub(storage_capacity),
        }
    }

    /// Original `RHSectorProduction::UpdateAmount` arithmetic, including its
    /// `f32` operations and truncation to an integer. Keep this as the single
    /// source used by both mutation and forecasting.
    fn calculate_produced_amount(&self, mission_length: u16, has_specialist: bool) -> u16 {
        let production_speed = (self.speed as f32) / (100.0 * 10.0);
        let super_production = if has_specialist { 1.5 } else { 1.0 };
        let produced = super_production
            * production_speed
            * self.occupants.len() as f32
            * mission_length as f32;
        (produced as u32).min(u16::MAX as u32) as u16
    }

    /// Map a production type to the player `Action` it feeds.  Returns
    /// `None` for non-bonus types (training, heal, relic, unknown).
    pub fn associated_action(&self) -> Option<Action> {
        match self.prod_type {
            Type::MakeArrow => Some(Action::Bow),
            Type::MakePurse => Some(Action::Purse),
            Type::MakeStone => Some(Action::Stone),
            Type::MakeApple => Some(Action::Apple),
            Type::MakeAle => Some(Action::Ale),
            Type::MakeLamblegg => Some(Action::Eat),
            Type::MakePlant => Some(Action::Heal),
            Type::MakeNet => Some(Action::Net),
            Type::MakeWaspNest => Some(Action::WaspNest),
            _ => None,
        }
    }

    /// Harvest the remaining bonus count for this sector's action from the
    /// live engine entities: sums quantities for active bonuses of matching
    /// action, plus one per active in-flight arrow projectile when the
    /// sector feeds `Action::Bow`.
    pub fn get_amount_from_current_mission(&mut self, entities: &Entities) {
        let Some(action) = self.associated_action() else {
            return;
        };
        let mut total: u32 = 0;
        for (_, bonus) in entities.bonuses() {
            if bonus.element.active && bonus.object.associated_action == action {
                total += bonus.object.quantity as u32;
            }
        }
        if action == Action::Bow {
            for (_, projectile) in entities.projectiles() {
                if projectile.element.active && projectile.object.object_type == ObjectType::Arrow {
                    total += 1;
                }
            }
        }
        self.amount = total.min(u16::MAX as u32) as u16;
    }

    /// Add production from the last-played mission (when won) onto the stored
    /// amount.
    ///
    /// `has_specialist` toggles the 1.5× "superproduction" bonus — the caller
    /// resolves it by scanning occupants for the expected profile name
    /// (see `sherwood_stat::find_specialist`).
    pub fn update_amount(&mut self, mission_length: u16, has_specialist: bool) {
        self.produced_amount = self.calculate_produced_amount(mission_length, has_specialist);
        self.amount = self.amount.saturating_add(self.produced_amount);
    }

    /// Compute the per-point bonus drops that `SetAmountToCurrentMission`
    /// would spawn, capped at 5/point.  Returns `(point_index, quantity)`
    /// pairs and updates `max_amount_reached`.  Consumers:
    /// `EngineInner::spawn_sherwood_production_bonuses`.
    pub fn plan_bonus_spawns(&mut self) -> Vec<(usize, u16)> {
        let mut plan = Vec::new();
        let mut left = self.amount;
        for (i, _pt) in self.production_points.iter().enumerate() {
            if left == 0 {
                break;
            }
            let q = left.min(5);
            plan.push((i, q));
            left -= q;
        }
        self.max_amount_reached = self.amount as usize >= self.production_points.len() * 5;
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementBonus, ElementData, ElementKind, Entity, ObjectData, ObjectType};

    fn bonus_arrow(quantity: u16) -> Entity {
        Entity::Bonus(ElementBonus {
            element: ElementData {
                kind: ElementKind::ObjectBonus,
                active: true,
                ..Default::default()
            },
            object: ObjectData {
                quantity,
                object_type: ObjectType::BonusArrow,
                associated_action: Action::Bow,
                ..Default::default()
            },
        })
    }

    fn point(x: f32, y: f32) -> Point {
        Point {
            x,
            y,
            layer: 0,
            sector: 0,
            obstacle: 0xFFFF,
        }
    }

    /// End-to-end campaign round-trip: harvest leftover arrows in mission A,
    /// round-trip through JSON (simulating campaign save/load), then verify
    /// the respawn plan drops them at the next Sherwood visit.
    #[test]
    fn harvest_then_respawn_round_trips_through_serde() {
        let mut sector = SectorProduction::new(Type::MakeArrow);
        sector.speed = 200;
        sector.production_points = vec![point(0.0, 0.0), point(10.0, 0.0), point(20.0, 0.0)];

        // Mission A leaves 7 arrows in the world (5 in one stack, 2 in another).
        let mut entities = Entities::new();
        entities.push(Some(bonus_arrow(5)));
        entities.push(Some(bonus_arrow(2)));
        sector.get_amount_from_current_mission(&entities);
        assert_eq!(sector.amount, 7);

        // Round-trip through JSON (serde) — simulates campaign save/load.
        let json = serde_json::to_string(&sector).unwrap();
        let mut loaded: SectorProduction = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.amount, 7);
        assert_eq!(loaded.production_points.len(), 3);
        assert_eq!(loaded.speed, 200);

        // Sherwood respawn: no prior mission won, so just drop the harvested
        // amount back at the production points — 5 cap per point.
        let plan = loaded.plan_bonus_spawns();
        assert_eq!(plan, vec![(0, 5), (1, 2)]);
        assert!(!loaded.max_amount_reached);
    }

    #[test]
    fn arrow_projectile_counts_toward_bow_sector() {
        use crate::element::{ElementProjectile, ProjectileData};
        let mut sector = SectorProduction::new(Type::MakeArrow);
        let mut entities = Entities::new();
        entities.push(Some(bonus_arrow(3)));
        entities.push(Some(Entity::Projectile(ElementProjectile {
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..Default::default()
            },
            object: ObjectData {
                object_type: ObjectType::Arrow,
                associated_action: Action::Bow,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
        })));
        sector.get_amount_from_current_mission(&entities);
        assert_eq!(sector.amount, 4);
    }

    #[test]
    fn update_amount_applies_specialist_multiplier() {
        let mut sector = SectorProduction::new(Type::MakeArrow);
        sector.speed = 100;
        sector.occupants = vec![
            Occupant {
                pc_description_idx: 0,
                x: 0.0,
                y: 0.0,
                obstacle: 0xFFFF,
            },
            Occupant {
                pc_description_idx: 1,
                x: 0.0,
                y: 0.0,
                obstacle: 0xFFFF,
            },
        ];
        sector.update_amount(60, false);
        assert_eq!(sector.produced_amount, 12); // 1.0 * 0.1 * 2 * 60

        let mut sector = SectorProduction::new(Type::MakeArrow);
        sector.speed = 100;
        sector.occupants = vec![
            Occupant {
                pc_description_idx: 0,
                x: 0.0,
                y: 0.0,
                obstacle: 0xFFFF,
            },
            Occupant {
                pc_description_idx: 1,
                x: 0.0,
                y: 0.0,
                obstacle: 0xFFFF,
            },
        ];
        sector.update_amount(60, true);
        assert_eq!(sector.produced_amount, 18); // 1.5 * 0.1 * 2 * 60
    }

    #[test]
    fn forecast_and_update_share_exact_boundary_arithmetic() {
        let cases = [
            (0, 0, 0, false, 0),
            (99, 1, 9, false, 7),
            (100, 2, 60, false, 7),
            (100, 2, 60, true, 7),
            (u16::MAX, 8, u16::MAX, true, u16::MAX - 3),
        ];

        for (speed, workers, seconds, specialist, stock) in cases {
            let mut sector = SectorProduction::new(Type::MakeArrow);
            sector.speed = speed;
            sector.amount = stock;
            sector.occupants = (0..workers)
                .map(|pc_description_idx| Occupant {
                    pc_description_idx,
                    ..Default::default()
                })
                .collect();

            let forecast = sector.forecast(seconds, specialist);
            sector.update_amount(seconds, specialist);

            assert_eq!(forecast.produced, sector.produced_amount);
            assert_eq!(forecast.projected_total, sector.amount);
        }
    }

    #[test]
    fn forecast_reports_capacity_and_existing_plus_new_overflow() {
        let mut sector = SectorProduction::new(Type::MakeNet);
        sector.speed = 100;
        sector.amount = 9;
        sector.production_points = vec![point(0.0, 0.0), point(1.0, 0.0)];
        sector.occupants = vec![Occupant::default(), Occupant::default()];

        let forecast = sector.forecast(60, false);
        assert_eq!(forecast.storage_capacity, 10);
        assert_eq!(forecast.produced, 12);
        assert_eq!(forecast.projected_total, 21);
        assert_eq!(forecast.overflow, 11);
        assert_eq!(forecast.raw_materials_required, 0);
    }
}
