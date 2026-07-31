//! Adoption of Original `RHElementMobile` masters kept outside Rust's entity arena.

use thiserror::Error;

use crate::{
    coordinates::{MapPoint, MapVec},
    element::EntityId,
    engine::EngineInner,
};

use super::{
    adopt::{LegacyEntityFixups, LegacyPositionTopology},
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    payload_objects::LegacyObjectItemPayload,
};

#[derive(Debug, Error)]
pub enum LegacyMobileAdoptError {
    #[error("saved mobile creation order {creation_order} has no initialized mobile master")]
    MissingMaster { creation_order: u32 },
    #[error(
        "saved mobile {mobile_index} has {saved} masked children, initialized master has {runtime}"
    )]
    ChildCount {
        mobile_index: usize,
        saved: usize,
        runtime: usize,
    },
    #[error("saved mobile {mobile_index} field {field} contains non-finite value {value}")]
    NonFinite {
        mobile_index: usize,
        field: &'static str,
        value: f32,
    },
    #[error("saved mobile {mobile_index} references absent Original sector slot {sector}")]
    MissingSector { mobile_index: usize, sector: u16 },
}

struct SavedMobile {
    mobile_index: usize,
    child_ids: Vec<EntityId>,
    child_positions: Vec<MapPoint>,
    child_active: Vec<bool>,
    child_animation_speeds: Vec<f32>,
    active: bool,
    stopped: bool,
    position: MapPoint,
    old_position: MapPoint,
    goal: MapPoint,
    increment: MapVec,
    path_index: u16,
    current_waypoint: u16,
    forward: bool,
    layer: u16,
    sector: u16,
    speed: f32,
    speed_goal: f32,
    acceleration: f32,
}

pub struct LegacyMobileAdoptionPlan {
    records: Vec<SavedMobile>,
}

impl LegacyMobileAdoptionPlan {
    pub fn preflight(
        engine: &EngineInner,
        payloads: &LegacyElementPayloadStream,
        entities: &LegacyEntityFixups,
        positions: &LegacyPositionTopology,
    ) -> Result<Self, LegacyMobileAdoptError> {
        let mut records = Vec::new();
        for record in &payloads.records {
            let LegacyElementPayload::ObjectItem(LegacyObjectItemPayload::Mobile(saved)) =
                &record.payload
            else {
                continue;
            };
            let creation_order = record.header.creation_order;
            let mobile_index = entities
                .mobile_by_creation_order
                .get(&creation_order)
                .copied()
                .ok_or(LegacyMobileAdoptError::MissingMaster { creation_order })?;
            let runtime = &engine.world.mobile_elements[mobile_index];
            if saved.sprites.len() != runtime.sprite_ids.len() {
                return Err(LegacyMobileAdoptError::ChildCount {
                    mobile_index,
                    saved: saved.sprites.len(),
                    runtime: runtime.sprite_ids.len(),
                });
            }
            for (field, value) in [
                ("position.x", saved.element.sprite.position.map.x),
                ("position.y", saved.element.sprite.position.map.y),
                ("old_position.x", saved.element.sprite.position.old_map.x),
                ("old_position.y", saved.element.sprite.position.old_map.y),
                ("goal.x", saved.element.sprite.position.goal_map.x),
                ("goal.y", saved.element.sprite.position.goal_map.y),
                ("increment.x", saved.element.sprite.position.increment_map.x),
                ("increment.y", saved.element.sprite.position.increment_map.y),
                ("speed", saved.speed),
                ("speed_goal", saved.speed_goal),
                ("acceleration", saved.acceleration),
            ] {
                if !value.is_finite() {
                    return Err(LegacyMobileAdoptError::NonFinite {
                        mobile_index,
                        field,
                        value,
                    });
                }
            }
            let sector_slot = saved.element.sprite.position.sector.0;
            let sector = match sector_slot {
                Some(slot) => positions
                    .sectors
                    .get(usize::from(slot))
                    .and_then(|sector| *sector)
                    .ok_or(LegacyMobileAdoptError::MissingSector {
                        mobile_index,
                        sector: slot,
                    })?
                    .get(),
                None => u16::MAX,
            };
            records.push(SavedMobile {
                mobile_index,
                child_ids: runtime.sprite_ids.clone(),
                child_positions: saved
                    .sprites
                    .iter()
                    .map(|sprite| {
                        MapPoint::new(
                            sprite.element.sprite.position.map.x,
                            sprite.element.sprite.position.map.y,
                        )
                    })
                    .collect(),
                child_active: saved
                    .sprites
                    .iter()
                    .map(|sprite| sprite.element.active)
                    .collect(),
                child_animation_speeds: saved
                    .sprites
                    .iter()
                    .map(|sprite| sprite.animation_speed)
                    .collect(),
                active: saved.element.active,
                stopped: saved.stopped,
                position: MapPoint::new(
                    saved.element.sprite.position.map.x,
                    saved.element.sprite.position.map.y,
                ),
                old_position: MapPoint::new(
                    saved.element.sprite.position.old_map.x,
                    saved.element.sprite.position.old_map.y,
                ),
                goal: MapPoint::new(
                    saved.element.sprite.position.goal_map.x,
                    saved.element.sprite.position.goal_map.y,
                ),
                increment: MapVec::new(
                    saved.element.sprite.position.increment_map.x,
                    saved.element.sprite.position.increment_map.y,
                ),
                path_index: saved.path.hiking_path_index.unwrap_or(runtime.path_index),
                current_waypoint: u16::from(saved.path.current_waypoint_index),
                forward: saved.path.forward_movement,
                layer: saved.element.sprite.position.layer,
                sector,
                speed: saved.speed,
                speed_goal: saved.speed_goal,
                acceleration: saved.acceleration,
            });
        }
        Ok(Self { records })
    }

    pub fn apply(self, engine: &mut EngineInner) {
        for saved in self.records {
            let mobile = &mut engine.world.mobile_elements[saved.mobile_index];
            let translation = saved.position - mobile.position;
            for point in &mut mobile.motion_polygon {
                *point = *point + translation;
            }
            mobile.active = saved.active;
            mobile.stopped = saved.stopped;
            mobile.position = saved.position;
            mobile.old_position = saved.old_position;
            mobile.goal = saved.goal;
            mobile.increment = saved.increment;
            mobile.path_index = saved.path_index;
            mobile.current_waypoint = saved.current_waypoint;
            mobile.forward = saved.forward;
            mobile.layer = saved.layer;
            mobile.sector = saved.sector;
            mobile.speed = saved.speed;
            mobile.speed_goal = saved.speed_goal;
            mobile.acceleration = saved.acceleration;

            for (((child_id, position), active), animation_speed) in saved
                .child_ids
                .into_iter()
                .zip(saved.child_positions)
                .zip(saved.child_active)
                .zip(saved.child_animation_speeds)
            {
                let child = engine
                    .world
                    .entities
                    .get_mut(child_id)
                    .and_then(crate::element::Entity::as_fx_mut)
                    .expect("mobile topology preflight retained every masked child");
                child.element.set_position_map(position);
                child.element.active = active;
                child.fx.animation_speed = animation_speed;
            }
        }
    }
}
