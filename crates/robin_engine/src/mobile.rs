//! Runtime for the cart/chariot subset of `RHElementMobile` used by shipped
//! Robin Hood missions.

use serde::{Deserialize, Serialize};

use crate::{
    coordinates::{MapBBox, MapPoint, MapVec},
    element::EntityId,
    fast_find_grid::GridLine,
    level_data::{RawHikingPath, RawMobileElement, WaypointCommand},
    repulsive::RepulsivePoint,
};

const COMMAND_MOBILE_SPEED: u8 = 129;
const COMMAND_MOBILE_ACCELERATION: u8 = 130;

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MobileElement {
    pub sprite_ids: Vec<EntityId>,
    pub motion_polygon: Vec<MapPoint>,
    pub position: MapPoint,
    pub old_position: MapPoint,
    pub path_index: u16,
    pub current_waypoint: u16,
    pub forward: bool,
    pub layer: u16,
    pub sector: u16,
    /// Current projection-area obstacle. Shipped records start at NULL and
    /// acquire one while crossing authored elevation bonds.
    pub obstacle: Option<u16>,
    pub active: bool,
    pub stopped: bool,
    pub speed: f32,
    pub speed_goal: f32,
    pub acceleration: f32,
    pub increment: MapVec,
    pub goal: MapPoint,
}

#[derive(Debug, Clone, Copy)]
pub struct MobileMotionPhase {
    pub movement: MapVec,
    pub reached_goal: bool,
}

impl MobileElement {
    pub fn from_raw(
        sim: &crate::sim_rng::SimulationContext,

        raw: &RawMobileElement,
        path: &RawHikingPath,
        sprite_ids: Vec<EntityId>,
    ) -> Result<Self, String> {
        let current_waypoint = u16::try_from(raw.start_waypoint)
            .map_err(|_| format!("start waypoint {} exceeds u16", raw.start_waypoint))?;
        let waypoint = path
            .waypoints
            .get(usize::from(current_waypoint))
            .ok_or_else(|| {
                format!(
                    "start waypoint {current_waypoint} is outside path {} ({} waypoints)",
                    raw.path_index,
                    path.waypoints.len()
                )
            })?;
        if path.waypoints.len() < 2 {
            return Err(format!(
                "mobile path {} has {} waypoints; C++ movement requires at least two",
                raw.path_index,
                path.waypoints.len()
            ));
        }

        let position = MapPoint::new(waypoint.x as f32, waypoint.y as f32);
        let motion_polygon = raw
            .motion_polygon
            .points
            .iter()
            .map(|&(x, y)| MapPoint::new(x as f32 + position.x, y as f32 + position.y))
            .collect();
        let mut mobile = Self {
            sprite_ids,
            motion_polygon,
            position,
            old_position: position,
            path_index: raw.path_index,
            current_waypoint,
            forward: true,
            layer: waypoint.level,
            sector: waypoint.sector,
            obstacle: None,
            active: true,
            stopped: false,
            speed: 10.0,
            speed_goal: 0.0,
            acceleration: 0.0,
            increment: MapVec::ZERO,
            goal: position,
        };

        mobile.execute_waypoint(sim, path)?;
        Ok(mobile)
    }

    pub fn set_active(&mut self, active: bool) -> bool {
        let changed = self.active != active;
        self.active = active;
        changed
    }

    pub fn start(&mut self) {
        self.stopped = false;
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    pub fn animation_speed(&self) -> f32 {
        if self.speed == 0.0 {
            1.0e6
        } else {
            1.0 / self.speed
        }
    }

    pub fn repulsive_lines(&self) -> Vec<GridLine> {
        let mut lines = Vec::with_capacity(self.motion_polygon.len());
        for pair in self.motion_polygon.windows(2) {
            let mut line = GridLine::new(pair[0], pair[1], false);
            line.set_repulsive(true);
            lines.push(line);
        }
        if let (Some(&first), Some(&last)) =
            (self.motion_polygon.first(), self.motion_polygon.last())
        {
            let mut line = GridLine::new(last, first, false);
            line.set_repulsive(true);
            lines.push(line);
        }
        lines
    }

    /// Reproduce `RHFastFindGrid::CreateRepulsivePoints` for the mobile
    /// motion sector. Mobile sectors are obstacles (not areas), so only
    /// outward/left-turn corners contribute a wedge-limited point. The
    /// master then overrides the normal obstacle force with `(0, 15)`.
    pub fn repulsive_points(&self) -> Vec<RepulsivePoint> {
        let count = self.motion_polygon.len();
        if count < 3 {
            return Vec::new();
        }

        let mut points = Vec::new();
        for index in 0..count {
            let previous = self.motion_polygon[(index + count - 1) % count];
            let current = self.motion_polygon[index];
            let next = self.motion_polygon[(index + 1) % count];
            let incoming = current - previous;
            let outgoing = next - current;
            if incoming.x * outgoing.y - incoming.y * outgoing.x > 0.0 {
                let mut point = RepulsivePoint::new(current, 0.0, 15.0);
                point.set_action_field(right_normal(incoming), right_normal(outgoing));
                points.push(point);
            }
        }
        points
    }

    pub fn contains_point(&self, point: MapPoint) -> bool {
        if self.motion_polygon.len() < 3 {
            return false;
        }
        let mut inside = false;
        let mut previous = *self.motion_polygon.last().expect("polygon is non-empty");
        for &current in &self.motion_polygon {
            if (current.y > point.y) != (previous.y > point.y) {
                let cross_x = (previous.x - current.x) * (point.y - current.y)
                    / (previous.y - current.y)
                    + current.x;
                if point.x < cross_x {
                    inside = !inside;
                }
            }
            previous = current;
        }
        inside
    }

    /// Exact polygon-vs-box test used by RHPositionInterface's mobile
    /// blocker check. A corridor may miss every perimeter line while its
    /// destination move box is nevertheless wholly inside the cart.
    pub fn polygon_intersects_bbox(polygon: &[MapPoint], bbox: &MapBBox) -> bool {
        let vertices = polygon
            .iter()
            .map(|point| point.to_geo())
            .collect::<Vec<_>>();
        crate::geo2d::polygon_vertices_intersect_bbox(&vertices, &bbox.to_geo())
    }

    pub fn is_moving(&self) -> bool {
        self.position != self.old_position
    }

    /// Original `NewMove(); Update()` slice. `None` is the strict early return
    /// for inactive/stopped masters; callers must not run crossing or waypoint
    /// work in that case.
    pub fn begin_hourglass_motion(&mut self) -> Option<MobileMotionPhase> {
        if !self.active || self.stopped {
            return None;
        }

        self.old_position = self.position;
        if self.acceleration != 0.0 {
            self.speed += self.acceleration;
            if (self.acceleration > 0.0 && self.speed >= self.speed_goal)
                || (self.acceleration < 0.0 && self.speed <= self.speed_goal)
            {
                self.acceleration = 0.0;
                self.speed = self.speed_goal;
            }
        }

        let movement = if self.speed != 0.0 {
            MapVec::new(self.increment.x * self.speed, self.increment.y * self.speed)
        } else {
            MapVec::ZERO
        };
        self.position = self.position + movement;
        for point in &mut self.motion_polygon {
            *point = *point + movement;
        }

        let to_goal = self.goal - self.position;
        let reached_goal = self.increment.x * to_goal.x + self.increment.y * to_goal.y <= 0.0;
        Some(MobileMotionPhase {
            movement,
            reached_goal,
        })
    }

    /// Original goal-test/`ExecuteWayPoint` slice. This must run only after
    /// line crossing because waypoint macros may consume probability RNG,
    /// change speed/active state, and replace the path increment.
    pub fn finish_hourglass_waypoint(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        path: &RawHikingPath,
        reached_goal: bool,
    ) -> Result<(), String> {
        if reached_goal {
            self.execute_waypoint(sim, path)?;
        }
        Ok(())
    }

    fn execute_waypoint(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        path: &RawHikingPath,
    ) -> Result<(), String> {
        let waypoint = path
            .waypoints
            .get(usize::from(self.current_waypoint))
            .ok_or_else(|| format!("mobile waypoint {} is out of range", self.current_waypoint))?;
        match &waypoint.command {
            WaypointCommand::None => {}
            WaypointCommand::Script(class) => {
                // Shipped chariot paths never use waypoint scripts. Keep the
                // missing VM owner binding explicit instead of pretending the
                // script succeeded.
                return Err(format!(
                    "mobile waypoint scripts are unsupported (class '{class}')"
                ));
            }
            WaypointCommand::Macro(data) => self.execute_shipped_macro(sim, data, path)?,
        }

        self.advance_path(path)?;
        Ok(())
    }

    fn advance_path(&mut self, path: &RawHikingPath) -> Result<(), String> {
        let last = u16::try_from(path.waypoints.len() - 1)
            .map_err(|_| "mobile path has more than 65536 waypoints".to_string())?;
        if self.forward {
            if self.current_waypoint < last {
                self.current_waypoint += 1;
            } else {
                self.current_waypoint -= 1;
                self.forward = false;
            }
        } else if self.current_waypoint > 0 {
            self.current_waypoint -= 1;
        } else {
            self.current_waypoint += 1;
            self.forward = true;
        }

        let waypoint = &path.waypoints[usize::from(self.current_waypoint)];
        self.goal = MapPoint::new(waypoint.x as f32, waypoint.y as f32);
        let delta = self.goal - self.position;
        let length = (delta.x * delta.x + delta.y * delta.y).sqrt();
        if length == 0.0 {
            return Err(format!(
                "mobile path {} has coincident waypoint {} at ({}, {})",
                self.path_index, self.current_waypoint, waypoint.x, waypoint.y
            ));
        }
        self.increment = MapVec::new(delta.x / length, delta.y / length);
        Ok(())
    }

    fn execute_shipped_macro(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        data: &[u8],
        path: &RawHikingPath,
    ) -> Result<(), String> {
        let directions = read_u16(data, 0)? as usize;
        if directions == 0 || directions > 2 {
            return Err(format!("invalid mobile macro direction count {directions}"));
        }
        let mut selected = None;
        let mut cursor = 2;
        for _ in 0..directions {
            let flag = *data
                .get(cursor)
                .ok_or_else(|| "truncated mobile macro direction".to_string())?;
            let offset = read_u16(data, cursor + 1)? as usize;
            cursor += 3;
            let right_direction = match flag {
                0 => true,
                1 => self.forward,
                2 => !self.forward,
                _ => return Err(format!("invalid mobile direction flag {flag}")),
            };
            if selected.is_none() && right_direction {
                selected = Some(offset);
            }
        }
        let Some(mut block) = selected else {
            return Ok(());
        };

        let probability_blocks = read_u16(data, block)? as usize;
        block += 2;
        if probability_blocks == 0 {
            return Err("mobile macro has no probability blocks".into());
        }
        // The C++ code consumes rand() even for a single 100% block.
        let mut roll = crate::sim_rng::u8(
            sim,
            crate::sim_rng::RngSite::MobileWaypointProbability,
            1..=100,
        );
        let mut commands = None;
        for _ in 0..probability_blocks {
            let probability = *data
                .get(block)
                .ok_or_else(|| "truncated mobile probability block".to_string())?;
            let offset = read_u16(data, block + 1)? as usize;
            block += 3;
            if commands.is_none() && roll <= probability {
                commands = Some(offset);
            } else {
                roll = roll.saturating_sub(probability);
            }
        }
        let Some(mut cursor) = commands else {
            return Ok(());
        };
        let byte_count = read_u16(data, cursor)? as usize;
        cursor += 2;
        let end = cursor
            .checked_add(byte_count)
            .filter(|&end| end <= data.len())
            .ok_or_else(|| "mobile macro command block exceeds waypoint data".to_string())?;

        while cursor < end {
            let command = data[cursor];
            cursor += 1;
            match command {
                COMMAND_MOBILE_SPEED => {
                    self.speed = read_f32(data, cursor)?;
                    cursor += 4;
                    self.active = self.speed != 0.0;
                }
                COMMAND_MOBILE_ACCELERATION => {
                    self.speed_goal = read_f32(data, cursor)?;
                    if self.speed_goal == 0.0 {
                        self.speed_goal = 0.1;
                    }
                    let target = read_u16(data, cursor + 4)? as usize;
                    cursor += 6;
                    let waypoint = path.waypoints.get(target).ok_or_else(|| {
                        format!("mobile acceleration references missing waypoint {target}")
                    })?;
                    let dx = waypoint.x as f32 - self.position.x;
                    let dy = waypoint.y as f32 - self.position.y;
                    let distance = (dx * dx + dy * dy).sqrt();
                    if distance == 0.0 {
                        return Err(format!(
                            "mobile acceleration target waypoint {target} has zero distance"
                        ));
                    }
                    self.acceleration =
                        0.5 * (self.speed_goal - self.speed) * (self.speed_goal + self.speed)
                            / distance;
                }
                other => {
                    return Err(format!(
                        "mobile waypoint opcode {other} is not used by released chariot data"
                    ));
                }
            }
        }
        Ok(())
    }
}

fn right_normal(vector: MapVec) -> MapVec {
    let length = vector.length();
    if length == 0.0 {
        MapVec::ZERO
    } else {
        MapVec::new(vector.y / length, -vector.x / length)
    }
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| "truncated mobile macro u16".to_string())?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_f32(data: &[u8], offset: usize) -> Result<f32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated mobile macro f32".to_string())?;
    Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level_data::{RawWaypoint, SectorPolygon};

    fn macro_block(commands: &[u8]) -> WaypointCommand {
        let mut data = Vec::new();
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.extend_from_slice(&5u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(100);
        data.extend_from_slice(&10u16.to_le_bytes());
        data.extend_from_slice(&(commands.len() as u16).to_le_bytes());
        data.extend_from_slice(commands);
        WaypointCommand::Macro(data)
    }

    fn speed_acceleration(speed: f32, goal: f32, target: u16) -> WaypointCommand {
        let mut commands = vec![COMMAND_MOBILE_SPEED];
        commands.extend_from_slice(&speed.to_le_bytes());
        commands.push(COMMAND_MOBILE_ACCELERATION);
        commands.extend_from_slice(&goal.to_le_bytes());
        commands.extend_from_slice(&target.to_le_bytes());
        macro_block(&commands)
    }

    fn speed(speed: f32) -> WaypointCommand {
        let mut commands = vec![COMMAND_MOBILE_SPEED];
        commands.extend_from_slice(&speed.to_le_bytes());
        macro_block(&commands)
    }

    fn raw_mobile() -> RawMobileElement {
        RawMobileElement {
            sprites: Vec::new(),
            motion_polygon: SectorPolygon {
                points: vec![(0, 0), (10, 0), (0, 10)],
            },
            editor_anchor: (0, 0),
            hook_point: None,
            barrel_point: None,
            path_index: 0,
            start_waypoint: 0,
            stop_waypoint: 3,
            obstacle_index: u16::MAX,
        }
    }

    fn path() -> RawHikingPath {
        RawHikingPath {
            waypoints: vec![
                RawWaypoint {
                    x: 0,
                    y: 0,
                    sector: 4,
                    level: 1,
                    command: speed_acceleration(5.0, 2.0, 1),
                },
                RawWaypoint {
                    x: 100,
                    y: 0,
                    sector: 4,
                    level: 1,
                    command: WaypointCommand::None,
                },
                RawWaypoint {
                    x: 200,
                    y: 0,
                    sector: 4,
                    level: 1,
                    command: WaypointCommand::None,
                },
                RawWaypoint {
                    x: 300,
                    y: 0,
                    sector: 4,
                    level: 1,
                    command: speed(0.0),
                },
            ],
        }
    }

    #[test]
    fn shipped_speed_acceleration_and_terminal_stop_match_cpp() {
        crate::sim_rng::with_seed(7, |sim| {
            let path = path();
            let mut mobile =
                MobileElement::from_raw(sim, &raw_mobile(), &path, Vec::new()).unwrap();
            assert_eq!(mobile.speed, 5.0);
            assert!((mobile.acceleration - -0.105).abs() < 0.000_01);
            assert_eq!(mobile.current_waypoint, 1);

            let first = mobile.begin_hourglass_motion().unwrap();
            mobile
                .finish_hourglass_waypoint(sim, &path, first.reached_goal)
                .unwrap();
            assert!((first.movement.x - 4.895).abs() < 0.000_01);

            for _ in 0..200 {
                let phase = mobile.begin_hourglass_motion().unwrap();
                mobile
                    .finish_hourglass_waypoint(sim, &path, phase.reached_goal)
                    .unwrap();
                if !mobile.active {
                    break;
                }
            }
            assert!(
                !mobile.active,
                "terminal speed=0 command must deactivate the cart"
            );
            assert_eq!(mobile.current_waypoint, 2);
            assert!(!mobile.forward);
        });
    }

    #[test]
    fn motion_sector_builds_closed_lines_and_corner_points() {
        crate::sim_rng::with_seed(9, |sim| {
            let mobile = MobileElement::from_raw(sim, &raw_mobile(), &path(), Vec::new()).unwrap();
            assert_eq!(mobile.repulsive_lines().len(), 3);
            assert_eq!(mobile.repulsive_points().len(), 3);
            assert!(mobile.contains_point(MapPoint::new(2.0, 2.0)));
            assert!(!mobile.contains_point(MapPoint::new(20.0, 20.0)));
            assert!(
                mobile
                    .repulsive_points()
                    .iter()
                    .all(|point| point.radius == 0.0 && point.action_radius == 15.0)
            );
        });
    }

    #[test]
    fn stopped_mobile_freezes_master_motion() {
        crate::sim_rng::with_seed(11, |sim| {
            let path = path();
            let mut mobile =
                MobileElement::from_raw(sim, &raw_mobile(), &path, Vec::new()).unwrap();
            let position = mobile.position;
            mobile.stop();
            assert!(mobile.begin_hourglass_motion().is_none());
            assert_eq!(mobile.position, position);
        });
    }

    #[test]
    fn motion_polygon_blocks_a_box_fully_inside_it() {
        let polygon = vec![
            MapPoint::new(0.0, 0.0),
            MapPoint::new(20.0, 0.0),
            MapPoint::new(20.0, 20.0),
            MapPoint::new(0.0, 20.0),
        ];
        let inside = MapBBox::from_coords(8.0, 8.0, 12.0, 12.0);
        let outside = MapBBox::from_coords(30.0, 30.0, 35.0, 35.0);
        assert!(MobileElement::polygon_intersects_bbox(&polygon, &inside));
        assert!(!MobileElement::polygon_intersects_bbox(&polygon, &outside));
    }
}
