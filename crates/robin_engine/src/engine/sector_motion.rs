//! Motion-area sector helpers.
//!
//! Currently exposes `get_projection_area_index`: given a motion-area
//! sector, a layer, and a point (screen-space for screen-coord callers,
//! ground-space for ground-coord callers — for ground-level projection
//! areas the two coincide since `z == 0`), it returns the obstacle
//! index of the projection area containing the point, or `None` if no
//! projection area matches.

use super::{EngineInner, LevelAssets};

impl EngineInner {
    /// Look up the projection-area obstacle containing `point` for the
    /// given motion-area sector + layer.
    ///
    /// Iterates the sight-obstacle table, restricts to projection-area
    /// obstacles whose `(sector, layer)` match, then returns the
    /// obstacle whose projected bbox **and** projected polygon
    /// (vertices `(x, y - z_top)`) both contain `point`.  When several
    /// candidates match, picks the one with the greatest `box_3d_max.z`
    /// ("highest obstacle" rule).
    ///
    /// `point` is treated as screen-space — the obstacle's top polygon
    /// is already screen-projected.  Callers pass ground `(x, y)`
    /// because for ground-level entities (`z == 0`) screen-Y equals
    /// ground-Y; lifted entities should subtract their own z before
    /// calling.
    pub fn get_projection_area_index(
        &self,
        assets: &LevelAssets,
        sector: u16,
        layer: u16,
        point: crate::coordinates::MapPoint,
    ) -> Option<u16> {
        let mut best: Option<(u16, f32)> = None;
        for (oi, obs) in self.sight_obstacles(assets).iter_indexed() {
            if !obs.is_projection_area() {
                continue;
            }
            if obs.sector != sector || obs.layer != layer {
                continue;
            }
            if !obs.box_projection.contains_point(point) {
                continue;
            }
            if !obs.contains_point_projection(point) {
                continue;
            }
            let z_max = obs.box_3d_max[2];
            let oi = oi as u16;
            match best {
                None => best = Some((oi, z_max)),
                Some((_, prev_z)) if z_max > prev_z => best = Some((oi, z_max)),
                _ => {}
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// Resolve a 3D point from a motion-sector membership + map-space
    /// `(x, y)`.  Returns `(x, y + z, z)` where `z` is the top-plane
    /// altitude of the projection-area obstacle containing `(x, y)`
    /// under the named motion sector.  Callers:
    ///
    /// - Purse scatter goal elevation (`engine/purse.rs::burst_purse`),
    ///   replacing the naive "reuse source z" approximation.
    /// - `RecordEnterGame` native (`natives/mod.rs`), to compute the
    ///   spawn elevation matching the destination sector.
    ///
    /// `sector` is the motion-area `sector_number` (u16) that the point
    /// belongs to; passing an unrelated sector returns `None` for the
    /// projection-area lookup and the result falls back to `z = 0`.
    /// We panic when the sector is present but flagged non-motion (per
    /// the project "No fake data" rule); a missing sector yields
    /// `z = 0` + `y + z = y`.
    pub fn position_to_point_3d(
        &self,
        assets: &LevelAssets,
        sector: Option<crate::position_interface::SectorHandle>,
        layer: u16,
        x: f32,
        y: f32,
    ) -> crate::coordinates::WorldPoint3D {
        let z = match sector {
            None => 0.0,
            Some(handle) => {
                let sn = crate::sector::SectorNumber::new(handle.get() as i16);
                let grid_idx = self.fast_grid.level.sector_number_map.get(&sn).copied();
                let is_motion = grid_idx.is_some_and(|gi| {
                    self.fast_grid
                        .level
                        .sectors
                        .get(gi)
                        .is_some_and(|gs| gs.sector_type.is_motion())
                });
                if grid_idx.is_some() && !is_motion {
                    panic!(
                        "position_to_point_3d: sector {} is not a motion sector",
                        handle.get()
                    );
                }
                match self.get_projection_area_index(
                    assets,
                    handle.get(),
                    layer,
                    crate::coordinates::MapPoint::new(x, y),
                ) {
                    Some(obs_idx) => self
                        .sight_obstacles(assets)
                        .get(obs_idx as usize)
                        .map(|obs| obs.compute_top_z(x, y))
                        .unwrap_or(0.0),
                    None => 0.0,
                }
            }
        };
        crate::coordinates::WorldPoint3D { x, y: y + z, z }
    }
}
