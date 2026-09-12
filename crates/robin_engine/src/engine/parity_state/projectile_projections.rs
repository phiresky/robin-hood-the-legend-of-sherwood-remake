//! Exact subtype and projectile parity schema, separate from live entity storage.

use super::{
    ParityEntityReference,
    projections::{Point2, Point3},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct TrajectoryPoint {
    pub position: Point3,
    pub time: u16,
    pub bounce: Option<bool>,
    pub material: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct TrajectoryOrigin {
    pub map: Point2,
    pub sector: Option<u16>,
    pub layer: Option<u16>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Projectile {
    pub flying: bool,
    pub dive: bool,
    pub magic_bullet: bool,
    pub frame_count: u16,
    pub trajectory_origin: TrajectoryOrigin,
    pub flight_direction: u16,
    pub start: Point3,
    pub end: Point3,
    pub shooter: Option<ParityEntityReference>,
    pub trajectory: Vec<TrajectoryPoint>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Subtype {
    Target {
        animation: u32,
        progression: u32,
        linked_fx: Vec<ParityEntityReference>,
        force_display: bool,
        restore_background: bool,
    },
    Scroll {
        status: i32,
        script_hourglass_timeout: u32,
    },
    Net {
        projectile: Projectile,
        victims: Vec<ParityEntityReference>,
        time_till_unfolding: u32,
        crumpled: bool,
        was_flying: bool,
    },
    Arrow {
        projectile: Projectile,
        bow_profile: Option<u32>,
        flat_shot: bool,
        falling: bool,
        falling_direction: u16,
        last_sector: u16,
        last_azimuth: i16,
        play_impact: bool,
    },
    Purse {
        projectile: Projectile,
        number_of_coins: u16,
    },
    Coin {
        projectile: Projectile,
        source_purse: Option<ParityEntityReference>,
    },
    Wasp {
        nest: Option<ParityEntityReference>,
        victim: Option<ParityEntityReference>,
        stinging: bool,
        timeout: u32,
        movement: Point3,
    },
    WaspNest {
        projectile: Projectile,
        flying_wasp_count: u32,
    },
    Projectile {
        projectile: Projectile,
    },
}
