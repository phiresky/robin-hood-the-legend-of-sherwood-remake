//! Domain-specific coordinate types.
//!
//! The original C++ uses `SBGeoPoint2D` for several incompatible spaces.
//! These newtypes make the most common footgun explicit: map coordinates are
//! already the isometric projection `(world.x, world.y - world.z)`, while
//! ground coordinates are raw world `(x, y)`.

use serde::{Deserialize, Serialize};

use crate::geo2d;

macro_rules! coord2 {
    (
        $(#[$meta:meta])*
        $name:ident,
        $ctor:ident
    ) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            Copy,
            Default,
            PartialEq,
            Serialize,
            Deserialize,
            robin_state_hash_derive::StateHash,
        )]
        pub struct $name {
            pub x: f32,
            pub y: f32,
        }

        impl $name {
            pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

            #[inline]
            pub const fn new(x: f32, y: f32) -> Self {
                Self { x, y }
            }

            #[inline]
            pub fn from_geo(p: geo2d::Point2D) -> Self {
                Self { x: p.x, y: p.y }
            }

            #[inline]
            pub fn to_geo(self) -> geo2d::Point2D {
                geo2d::pt(self.x, self.y)
            }
        }

        impl From<geo2d::Point2D> for $name {
            #[inline]
            fn from(p: geo2d::Point2D) -> Self {
                Self::from_geo(p)
            }
        }

        #[inline]
        pub const fn $ctor(x: f32, y: f32) -> $name {
            $name::new(x, y)
        }
    };
}

coord2!(
    /// Projected map point: `(world.x, world.y - world.z)`.
    MapPoint,
    map_pt
);
coord2!(
    /// Projected map vector. Deltas in the same space as [`MapPoint`].
    MapVec,
    map_vec
);
coord2!(
    /// Ground/world-XY point: `(world.x, world.y)`.
    GroundPoint,
    ground_pt
);
coord2!(
    /// Ground/world-XY vector. Deltas in the same space as [`GroundPoint`].
    GroundVec,
    ground_vec
);
coord2!(
    /// Sprite top-left point before viewport transform.
    ///
    /// C++ `PositionSprite`: a floored draw origin. Convert to map space with
    /// `sprite_top_left + sprite_anchor = map_position`.
    SpriteTopLeft,
    sprite_top_left
);
coord2!(
    /// Sprite anchor/center offset.
    ///
    /// C++ `SpriteCenter`: offset from [`SpriteTopLeft`] to the entity's
    /// projected [`MapPoint`].
    SpriteAnchor,
    sprite_anchor
);
coord2!(
    /// Point local to a sprite frame, such as a hand/action hotspot.
    SpriteLocalPoint,
    sprite_local_pt
);
coord2!(
    /// Screen-space point after viewport transform.
    ScreenPoint,
    screen_pt
);

impl MapPoint {
    /// Project a world-space `(x, y, z)` position into map coordinates.
    ///
    /// This is the core Spellbound/C++ projection rule: map `(x, y)` is
    /// world `(x, y - z)`.
    #[inline]
    pub const fn from_world_xyz(x: f32, y: f32, z: f32) -> Self {
        Self { x, y: y - z }
    }
}

impl GroundPoint {
    /// Reconstruct ground/world-XY coordinates from a projected map point and
    /// an elevation.
    #[inline]
    pub const fn from_map_and_z(map: MapPoint, z: f32) -> Self {
        Self {
            x: map.x,
            y: map.y + z,
        }
    }
}

impl MapVec {
    /// Project a world-space `(x, y, z)` delta into map-vector coordinates.
    #[inline]
    pub const fn from_world_xyz(x: f32, y: f32, z: f32) -> Self {
        Self { x, y: y - z }
    }
}

impl GroundVec {
    /// Reconstruct a world-XY delta from a projected map delta and an
    /// elevation delta.
    #[inline]
    pub const fn from_map_and_z(map: MapVec, z: f32) -> Self {
        Self {
            x: map.x,
            y: map.y + z,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_xyz_projection_is_named_and_reversible_with_z() {
        let map = MapPoint::from_world_xyz(10.0, 25.0, 7.0);
        assert_eq!(map, MapPoint { x: 10.0, y: 18.0 });

        let ground = GroundPoint::from_map_and_z(map, 7.0);
        assert_eq!(ground, GroundPoint { x: 10.0, y: 25.0 });
    }

    #[test]
    fn world_xyz_delta_projection_is_named_and_reversible_with_z_delta() {
        let map = MapVec::from_world_xyz(2.0, 9.0, 4.0);
        assert_eq!(map, MapVec { x: 2.0, y: 5.0 });

        let ground = GroundVec::from_map_and_z(map, 4.0);
        assert_eq!(ground, GroundVec { x: 2.0, y: 9.0 });
    }

    #[test]
    fn sprite_anchor_is_not_a_map_point_even_with_same_components() {
        let anchor = SpriteAnchor::new(8.0, 11.0);
        let map = MapPoint::new(8.0, 11.0);

        assert_eq!(anchor.to_geo(), map.to_geo());
    }
}
