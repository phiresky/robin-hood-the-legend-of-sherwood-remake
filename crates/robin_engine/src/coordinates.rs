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
