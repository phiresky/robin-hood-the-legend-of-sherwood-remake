//! Enemy-AI vector operations on the typed map-space vector
//! [`crate::coordinates::MapVec`]: compass-sector vectors, dot/determinant,
//! Chebyshev / squared norms, plain and aspect-corrected normals, and
//! aspect-corrected norm / normalization.
//!
//! AI `Position` coordinates are projected map space, so per
//! `docs/COORDINATES.md` their deltas are `MapVec` (`a.map_point() -
//! b.map_point()`), not generic `geo2d` points.
//!
//! Every method reproduces the exact f32 operation order of the tuple
//! helpers it replaced (`tests.rs` keeps those helpers verbatim and compares
//! bit patterns), so results that feed movement goals stay bit-identical. In
//! particular [`AiMapVec::iso_normalize`] divides unconditionally: a zero
//! vector becomes `(NaN, NaN)`, as in the original game.

use crate::coordinates::MapVec;

/// Enemy-AI extension methods for [`MapVec`].
pub(super) trait AiMapVec: Sized {
    /// Unit direction vector of a 0–15 compass sector (sector 0 = north
    /// `(0, -1)`, increasing clockwise), not aspect-compressed.
    fn from_sector(sector: u16) -> Self;

    /// Sector direction with Y compressed by the standard `ASPECT_RATIO`,
    /// via [`crate::position_interface::sector_to_vector_iso`].
    fn from_sector_iso(sector: u16) -> Self;

    /// Sector direction with its Y component scaled by a caller-chosen
    /// `aspect_ratio` (`1.0` or a sword-fight aspect ratio).
    fn from_sector_with_aspect(sector: u16, aspect_ratio: f32) -> Self;

    /// Dot product `self.x * other.x + self.y * other.y`.
    fn dot(self, other: Self) -> f32;

    /// 2D determinant `self.x * other.y - self.y * other.x`: positive if
    /// `other` is to the left of `self`.
    fn det(self, other: Self) -> f32;

    /// Chebyshev (maximum) norm.
    fn max_norm(self) -> f32;

    /// Squared Euclidean norm (`x * x + y * y`).
    fn square_norm(self) -> f32;

    /// Left normal: 90° counter-clockwise rotation `(-y, x)`.
    fn normal_left(self) -> Self;

    /// Right normal: 90° clockwise rotation `(y, -x)`.
    fn normal_right(self) -> Self;

    /// Aspect-corrected Euclidean norm `sqrt(x² + (y / aspect_ratio)²)`.
    /// Most callers pass `ASPECT_RATIO`; a few sword-fight call sites pass
    /// `SWORDFIGHT_ASPECT_RATIO`.
    fn iso_norm(self, aspect_ratio: f32) -> f32;

    /// Divide by [`Self::iso_norm`] without a zero check (zero vector ⇒ NaN).
    fn iso_normalize(self, aspect_ratio: f32) -> Self;

    /// Aspect-corrected normal, via
    /// [`crate::position_interface::vector_normal_iso`]: `direct = true`
    /// yields the left normal, `false` the right one.
    fn normal_iso(self, direct: bool) -> Self;

    /// 0–15 sector of this vector under `aspect_ratio`, via
    /// [`crate::position_interface::vector_to_sector_0_to_15_with_aspect`].
    fn sector_with_aspect(self, aspect_ratio: f32) -> u16;
}

impl AiMapVec for MapVec {
    fn from_sector(sector: u16) -> Self {
        let [x, y] = crate::shadow_polygon::sector_to_direction(sector as i16);
        MapVec::new(x, y)
    }

    fn from_sector_iso(sector: u16) -> Self {
        let [x, y] = crate::position_interface::sector_to_vector_iso(sector as i16);
        MapVec::new(x, y)
    }

    fn from_sector_with_aspect(sector: u16, aspect_ratio: f32) -> Self {
        let [x, y] = crate::shadow_polygon::sector_to_direction(sector as i16);
        MapVec::new(x, y * aspect_ratio)
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    fn det(self, other: Self) -> f32 {
        self.x * other.y - self.y * other.x
    }

    fn max_norm(self) -> f32 {
        self.x.abs().max(self.y.abs())
    }

    fn square_norm(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    fn normal_left(self) -> Self {
        MapVec::new(-self.y, self.x)
    }

    fn normal_right(self) -> Self {
        MapVec::new(self.y, -self.x)
    }

    fn iso_norm(self, aspect_ratio: f32) -> f32 {
        let yi = self.y / aspect_ratio;
        (self.x * self.x + yi * yi).sqrt()
    }

    fn iso_normalize(self, aspect_ratio: f32) -> Self {
        let norm = self.iso_norm(aspect_ratio);
        MapVec::new(self.x / norm, self.y / norm)
    }

    fn normal_iso(self, direct: bool) -> Self {
        let [x, y] = crate::position_interface::vector_normal_iso(self.x, self.y, direct);
        MapVec::new(x, y)
    }

    fn sector_with_aspect(self, aspect_ratio: f32) -> u16 {
        crate::position_interface::vector_to_sector_0_to_15_with_aspect(
            self.x,
            self.y,
            aspect_ratio,
        ) as u16
    }
}

#[cfg(test)]
mod tests;
