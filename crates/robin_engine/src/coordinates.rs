//! Domain-specific coordinate types.
//!
//! The original C++ uses `SBGeoPoint2D` for several incompatible spaces.
//! These newtypes make the most common footgun explicit: map coordinates are
//! already the isometric projection `(world.x, world.y - world.z)`, while
//! ground coordinates are raw world `(x, y)`.

use serde::{Deserialize, Serialize};

use crate::geo2d;
use geo::Rect;

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
            pub fn from_geo(p: geo2d::GeoPoint2D) -> Self {
                Self { x: p.x, y: p.y }
            }

            #[inline]
            pub fn to_geo(self) -> geo2d::GeoPoint2D {
                geo2d::pt(self.x, self.y)
            }
        }

        impl From<geo2d::GeoPoint2D> for $name {
            #[inline]
            fn from(p: geo2d::GeoPoint2D) -> Self {
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
    /// Point local to a cursor image, used as the cursor hotspot.
    CursorHotspot,
    cursor_hotspot
);
coord2!(
    /// Screen-space point after viewport transform.
    ScreenPoint,
    screen_pt
);

/// Screen-space axis-aligned bounding box.
///
/// This is intentionally distinct from `geo2d::BBox2D`: widget, menu, HUD,
/// and renderer hit-test rectangles live in screen/UI coordinates, not map or
/// ground coordinates. `None` is the same legacy "hyperspace" unset state as
/// `BBox2D`.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct ScreenBBox(pub Option<Rect<f32>>);

impl Default for ScreenBBox {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenBBox {
    #[inline]
    pub const fn new() -> Self {
        Self(None)
    }

    #[inline]
    pub fn from_geo(bbox: geo2d::BBox2D) -> Self {
        Self(bbox.0)
    }

    #[inline]
    pub fn to_geo(self) -> geo2d::BBox2D {
        geo2d::BBox2D(self.0)
    }

    #[inline]
    pub fn from_corners(min: ScreenPoint, max: ScreenPoint) -> Self {
        Self(Some(Rect::new(min.to_geo(), max.to_geo())))
    }

    #[inline]
    pub fn from_coords(x_min: f32, y_min: f32, x_max: f32, y_max: f32) -> Self {
        Self::from_geo(geo2d::BBox2D::from_coords(x_min, y_min, x_max, y_max))
    }

    #[inline]
    pub fn from_point(point: ScreenPoint) -> Self {
        Self(Some(Rect::new(point.to_geo(), point.to_geo())))
    }

    #[inline]
    pub fn from_point_size(origin: ScreenPoint, width: f32, height: f32) -> Self {
        Self::from_geo(geo2d::BBox2D::from_point_size(
            origin.to_geo(),
            width,
            height,
        ))
    }

    #[inline]
    pub fn is_somewhere(&self) -> bool {
        self.0.is_some()
    }

    #[inline]
    pub fn x_min(&self) -> f32 {
        self.0.unwrap().min().x
    }

    #[inline]
    pub fn y_min(&self) -> f32 {
        self.0.unwrap().min().y
    }

    #[inline]
    pub fn x_max(&self) -> f32 {
        self.0.unwrap().max().x
    }

    #[inline]
    pub fn y_max(&self) -> f32 {
        self.0.unwrap().max().y
    }

    #[inline]
    pub fn width(&self) -> f32 {
        let r = self.0.unwrap();
        r.max().x - r.min().x
    }

    #[inline]
    pub fn height(&self) -> f32 {
        let r = self.0.unwrap();
        r.max().y - r.min().y
    }

    #[inline]
    pub fn top_left(&self) -> ScreenPoint {
        ScreenPoint::from_geo(self.0.unwrap().min())
    }

    #[inline]
    pub fn bottom_right(&self) -> ScreenPoint {
        ScreenPoint::from_geo(self.0.unwrap().max())
    }

    #[inline]
    pub fn contains_point(&self, point: ScreenPoint) -> bool {
        self.to_geo().contains_point(point.to_geo())
    }

    #[inline]
    pub fn is_boxed_point(&self, point: ScreenPoint) -> bool {
        self.to_geo().is_boxed_point(point.to_geo())
    }

    #[inline]
    pub fn is_on_boundary(&self, point: ScreenPoint) -> bool {
        self.to_geo().is_on_boundary(point.to_geo())
    }

    #[inline]
    pub fn intersects_bbox(&self, other: &ScreenBBox) -> bool {
        self.to_geo().intersects_bbox(&other.to_geo())
    }

    #[inline]
    pub fn contains_bbox(&self, other: &ScreenBBox) -> bool {
        self.to_geo().contains_bbox(&other.to_geo())
    }

    #[inline]
    pub fn translated(&self, v: geo2d::Vec2D) -> Self {
        Self::from_geo(self.to_geo().translated(v))
    }

    #[inline]
    pub fn translate(&mut self, v: geo2d::Vec2D) {
        *self = self.translated(v);
    }
}

/// World-space 3D point: raw Spellbound `(x, y, z)`.
///
/// Project to map space with [`WorldPoint3D::to_map`], which applies the
/// game projection `(x, y - z)`. This type is for actual 3D positions; use
/// [`MapPoint`] for already-projected 2D map coordinates.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct WorldPoint3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for WorldPoint3D {
    fn default() -> Self {
        Self::ZERO
    }
}

impl WorldPoint3D {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Project to 2D map coordinates: `(x, y - z)`.
    #[inline]
    pub const fn to_map(self) -> MapPoint {
        MapPoint::new(self.x, self.y - self.z)
    }

    #[inline]
    pub fn norm(&self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    #[inline]
    pub const fn scale(self, k: f32) -> Self {
        Self {
            x: self.x * k,
            y: self.y * k,
            z: self.z * k,
        }
    }
}

impl std::ops::Sub for WorldPoint3D {
    type Output = WorldVec3D;

    #[inline]
    fn sub(self, o: Self) -> WorldVec3D {
        WorldVec3D {
            x: self.x - o.x,
            y: self.y - o.y,
            z: self.z - o.z,
        }
    }
}

impl std::ops::Add<WorldVec3D> for WorldPoint3D {
    type Output = Self;

    #[inline]
    fn add(self, o: WorldVec3D) -> Self {
        Self {
            x: self.x + o.x,
            y: self.y + o.y,
            z: self.z + o.z,
        }
    }
}

impl std::ops::AddAssign<WorldVec3D> for WorldPoint3D {
    #[inline]
    fn add_assign(&mut self, o: WorldVec3D) {
        self.x += o.x;
        self.y += o.y;
        self.z += o.z;
    }
}

impl std::ops::Sub<WorldVec3D> for WorldPoint3D {
    type Output = Self;

    #[inline]
    fn sub(self, o: WorldVec3D) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
            z: self.z - o.z,
        }
    }
}

/// World-space 3D vector/delta: velocity, movement, or offset in raw
/// Spellbound `(x, y, z)` axes.
///
/// This is intentionally distinct from [`WorldPoint3D`]: vectors can be
/// added to points, but they are not positions and should not be projected or
/// stored as entity locations.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct WorldVec3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for WorldVec3D {
    fn default() -> Self {
        Self::ZERO
    }
}

impl WorldVec3D {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub fn norm(&self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    #[inline]
    pub const fn scale(self, k: f32) -> Self {
        Self {
            x: self.x * k,
            y: self.y * k,
            z: self.z * k,
        }
    }
}

impl std::ops::Add for WorldVec3D {
    type Output = Self;

    #[inline]
    fn add(self, o: Self) -> Self {
        Self {
            x: self.x + o.x,
            y: self.y + o.y,
            z: self.z + o.z,
        }
    }
}

impl std::ops::AddAssign for WorldVec3D {
    #[inline]
    fn add_assign(&mut self, o: Self) {
        self.x += o.x;
        self.y += o.y;
        self.z += o.z;
    }
}

impl std::ops::Sub for WorldVec3D {
    type Output = Self;

    #[inline]
    fn sub(self, o: Self) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
            z: self.z - o.z,
        }
    }
}

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

impl std::ops::Sub for MapPoint {
    type Output = MapVec;

    #[inline]
    fn sub(self, o: Self) -> MapVec {
        MapVec {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
}

impl std::ops::Add<MapVec> for MapPoint {
    type Output = Self;

    #[inline]
    fn add(self, o: MapVec) -> Self {
        Self {
            x: self.x + o.x,
            y: self.y + o.y,
        }
    }
}

impl std::ops::Sub<MapVec> for MapPoint {
    type Output = Self;

    #[inline]
    fn sub(self, o: MapVec) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
}

impl std::ops::Add for MapVec {
    type Output = Self;

    #[inline]
    fn add(self, o: Self) -> Self {
        Self {
            x: self.x + o.x,
            y: self.y + o.y,
        }
    }
}

impl std::ops::Sub for MapVec {
    type Output = Self;

    #[inline]
    fn sub(self, o: Self) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
}

impl std::ops::AddAssign for MapVec {
    #[inline]
    fn add_assign(&mut self, o: Self) {
        self.x += o.x;
        self.y += o.y;
    }
}

impl MapVec {
    #[inline]
    pub const fn scale(self, k: f32) -> Self {
        Self {
            x: self.x * k,
            y: self.y * k,
        }
    }

    #[inline]
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
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
