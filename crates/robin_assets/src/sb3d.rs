//! 3D math helpers (vectors, planes, axis-aligned bounding boxes).
//!
//! All structs use `#[repr(C)]` with three contiguous f32s for `Vec3` and
//! a flat layout for `Plane3D` / `BBox3D`, so they can be passed across
//! `extern "C"` entry points if needed.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Vec3 — three contiguous f32s, doubles as a 3D point or vector
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl std::ops::Add for Vec3 {
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

impl std::ops::Sub for Vec3 {
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

#[allow(clippy::should_implement_trait)]
impl Vec3 {
    #[inline]
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub fn add(self, o: Self) -> Self {
        self + o
    }

    /// `operator-`
    #[inline]
    pub fn sub(self, o: Self) -> Self {
        self - o
    }

    /// `operator*` — dot product
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// `operator^` — cross product
    #[inline]
    pub fn cross(self, o: Self) -> Self {
        Self {
            x: self.y * o.z - self.z * o.y,
            y: self.z * o.x - self.x * o.z,
            z: self.x * o.y - self.y * o.x,
        }
    }

    /// Infinity-norm (`MaxNorm`)
    pub fn max_norm(self) -> f32 {
        let ax = self.x.abs();
        let ay = self.y.abs();
        let az = self.z.abs();
        if ax > ay {
            if ax > az { ax } else { az }
        } else if ay > az {
            ay
        } else {
            az
        }
    }

    /// Euclidean norm
    #[inline]
    pub fn norm(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    /// Normalize in place
    #[inline]
    pub fn normalize(&mut self) {
        let inv = 1.0 / self.norm();
        self.x *= inv;
        self.y *= inv;
        self.z *= inv;
    }

    /// Scalar multiply
    #[inline]
    pub fn scale(self, k: f32) -> Self {
        Self {
            x: k * self.x,
            y: k * self.y,
            z: k * self.z,
        }
    }

    /// `DistanceVectorToLine` — rejection of `self − ptA` from the direction
    /// `ptB − ptA` (i.e. the component perpendicular to the line).
    pub fn distance_vector_to_line(self, pt_a: Self, pt_b: Self) -> Self {
        let mut dir = pt_b.sub(pt_a);
        dir.normalize();
        let diff = self.sub(pt_a);
        let proj = diff.dot(dir);
        diff.sub(dir.scale(proj))
    }
}

// ---------------------------------------------------------------------------
// Plane3D
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct Plane3D {
    pub o: Vec3,
    pub a: Vec3,
    pub b: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub n: Vec3,
    pub d: f32,
    pub az: f32,
    pub bz: f32,
    pub dz: f32,
}

impl Plane3D {
    pub fn compute_vectors(&mut self) {
        self.u = self.a.sub(self.o);
        self.v = self.b.sub(self.o);
    }

    pub fn compute_normal(&mut self) {
        self.n.x = self.u.y * self.v.z - self.v.y * self.u.z;
        self.n.y = self.u.z * self.v.x - self.v.z * self.u.x;
        self.n.z = self.u.x * self.v.y - self.v.x * self.u.y;

        let norm = self.n.norm();
        self.n.x /= norm;
        self.n.y /= norm;
        self.n.z /= norm;
    }

    pub fn compute_homogen_equation(&mut self) {
        self.d = -self.o.x * self.n.x - self.o.y * self.n.y - self.o.z * self.n.z;
    }

    pub fn compute_z_equation(&mut self) {
        assert!(self.n.z != 0.0);
        let k = -1.0 / self.n.z;
        self.az = self.n.x * k;
        self.bz = self.n.y * k;
        self.dz = self.d * k;
    }

    #[inline]
    pub fn compute_z(&self, x: f32, y: f32) -> f32 {
        x * self.az + y * self.bz + self.dz
    }

    #[inline]
    pub fn compute_z_increment(&self, x: f32, y: f32) -> f32 {
        self.compute_z(self.o.x + x, self.o.y + y) - self.o.z
    }

    pub fn initialize_all(&mut self) {
        self.compute_vectors();
        self.compute_normal();
        self.compute_homogen_equation();
        self.compute_z_equation();
    }

    /// Translate the plane by a 3D vector.
    pub fn translate_3d(&mut self, t: Vec3) {
        self.o = self.o.add(t);
        self.a = self.a.add(t);
        self.b = self.b.add(t);
        self.d = -self.o.x * self.n.x - self.o.y * self.n.y - self.o.z * self.n.z;
        self.dz = -self.d / self.n.z;
    }
}

// ---------------------------------------------------------------------------
// BBox3D — axis-aligned bounding box
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BBox3D {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
    pub z_min: f32,
    pub z_max: f32,
}

impl Default for BBox3D {
    /// Seeds the empty-box sentinels so the first `expand` captures the point.
    fn default() -> Self {
        Self {
            x_min: 1e30,
            x_max: -1e30,
            y_min: 1e30,
            y_max: -1e30,
            z_min: 1e30,
            z_max: -1e30,
        }
    }
}

impl BBox3D {
    pub fn expand(&mut self, p: Vec3) {
        if self.x_min > p.x {
            self.x_min = p.x;
        }
        if self.x_max < p.x {
            self.x_max = p.x;
        }
        if self.y_min > p.y {
            self.y_min = p.y;
        }
        if self.y_max < p.y {
            self.y_max = p.y;
        }
        if self.z_min > p.z {
            self.z_min = p.z;
        }
        if self.z_max < p.z {
            self.z_max = p.z;
        }
    }

    pub fn is_inside(&self, p: Vec3) -> bool {
        !(self.x_min > p.x
            || self.x_max < p.x
            || self.y_min > p.y
            || self.y_max < p.y
            || self.z_min > p.z
            || self.z_max < p.z)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec3_add_sub() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(a.add(b), Vec3::new(5.0, 7.0, 9.0));
        assert_eq!(a.sub(b), Vec3::new(-3.0, -3.0, -3.0));
    }

    #[test]
    fn vec3_dot() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(a.dot(b), 32.0);
    }

    #[test]
    fn vec3_cross() {
        let i = Vec3::new(1.0, 0.0, 0.0);
        let j = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(i.cross(j), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(j.cross(i), Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn vec3_max_norm() {
        assert_eq!(Vec3::new(3.0, -5.0, 1.0).max_norm(), 5.0);
        assert_eq!(Vec3::new(-7.0, 2.0, 4.0).max_norm(), 7.0);
        assert_eq!(Vec3::new(1.0, 2.0, 9.0).max_norm(), 9.0);
    }

    #[test]
    fn vec3_norm_and_normalize() {
        let v = Vec3::new(3.0, 4.0, 0.0);
        assert!((v.norm() - 5.0).abs() < 1e-6);

        let mut n = v;
        n.normalize();
        assert!((n.x - 0.6).abs() < 1e-6);
        assert!((n.y - 0.8).abs() < 1e-6);
        assert!(n.z.abs() < 1e-6);
    }

    #[test]
    fn vec3_distance_to_line() {
        // Point (1,1,0) → line from origin along X axis → distance = (0,1,0)
        let d = Vec3::new(1.0, 1.0, 0.0)
            .distance_vector_to_line(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0));
        assert!(d.x.abs() < 1e-6);
        assert!((d.y - 1.0).abs() < 1e-6);
        assert!(d.z.abs() < 1e-6);
    }

    #[test]
    fn plane_initialize_all_xy() {
        let mut p = Plane3D {
            o: Vec3::new(0.0, 0.0, 0.0),
            a: Vec3::new(1.0, 0.0, 0.0),
            b: Vec3::new(0.0, 1.0, 0.0),
            u: Vec3::new(0.0, 0.0, 0.0),
            v: Vec3::new(0.0, 0.0, 0.0),
            n: Vec3::new(0.0, 0.0, 0.0),
            d: 0.0,
            az: 0.0,
            bz: 0.0,
            dz: 0.0,
        };
        p.initialize_all();
        assert_eq!(p.u, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(p.v, Vec3::new(0.0, 1.0, 0.0));
        assert!((p.n.z - 1.0).abs() < 1e-6);
        assert!(p.d.abs() < 1e-6);
        assert!(p.compute_z(5.0, 7.0).abs() < 1e-6);
    }

    #[test]
    fn plane_compute_z_tilted() {
        // z = x + 2y + 3: O=(0,0,3), A=(1,0,4), B=(0,1,5)
        let mut p = Plane3D {
            o: Vec3::new(0.0, 0.0, 3.0),
            a: Vec3::new(1.0, 0.0, 4.0),
            b: Vec3::new(0.0, 1.0, 5.0),
            u: Vec3::new(0.0, 0.0, 0.0),
            v: Vec3::new(0.0, 0.0, 0.0),
            n: Vec3::new(0.0, 0.0, 0.0),
            d: 0.0,
            az: 0.0,
            bz: 0.0,
            dz: 0.0,
        };
        p.initialize_all();
        assert!((p.compute_z(0.0, 0.0) - 3.0).abs() < 1e-4);
        assert!((p.compute_z(1.0, 0.0) - 4.0).abs() < 1e-4);
        assert!((p.compute_z(0.0, 1.0) - 5.0).abs() < 1e-4);
        assert!((p.compute_z(2.0, 3.0) - 11.0).abs() < 1e-4);
    }

    #[test]
    fn plane_z_increment() {
        let mut p = Plane3D {
            o: Vec3::new(0.0, 0.0, 3.0),
            a: Vec3::new(1.0, 0.0, 4.0),
            b: Vec3::new(0.0, 1.0, 5.0),
            u: Vec3::new(0.0, 0.0, 0.0),
            v: Vec3::new(0.0, 0.0, 0.0),
            n: Vec3::new(0.0, 0.0, 0.0),
            d: 0.0,
            az: 0.0,
            bz: 0.0,
            dz: 0.0,
        };
        p.initialize_all();
        // increment(1,0) = z(0+1,0+0) - z(0,0) = 4 - 3 = 1
        assert!((p.compute_z_increment(1.0, 0.0) - 1.0).abs() < 1e-4);
        // increment(0,1) = z(0,1) - z(0,0) = 5 - 3 = 2
        assert!((p.compute_z_increment(0.0, 1.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn plane_translate_3d() {
        let mut p = Plane3D {
            o: Vec3::new(0.0, 0.0, 0.0),
            a: Vec3::new(1.0, 0.0, 0.0),
            b: Vec3::new(0.0, 1.0, 0.0),
            u: Vec3::new(0.0, 0.0, 0.0),
            v: Vec3::new(0.0, 0.0, 0.0),
            n: Vec3::new(0.0, 0.0, 0.0),
            d: 0.0,
            az: 0.0,
            bz: 0.0,
            dz: 0.0,
        };
        p.initialize_all();
        p.translate_3d(Vec3::new(0.0, 0.0, 5.0));
        // After translating along Z, z(0,0) should be 5
        assert!((p.compute_z(0.0, 0.0) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn bbox_expand_and_inside() {
        let mut bbox = BBox3D::default();
        bbox.expand(Vec3::new(1.0, 2.0, 3.0));
        bbox.expand(Vec3::new(-1.0, -2.0, -3.0));
        assert!(bbox.is_inside(Vec3::new(0.0, 0.0, 0.0)));
        assert!(bbox.is_inside(Vec3::new(1.0, 2.0, 3.0)));
        assert!(!bbox.is_inside(Vec3::new(2.0, 0.0, 0.0)));
    }
}
