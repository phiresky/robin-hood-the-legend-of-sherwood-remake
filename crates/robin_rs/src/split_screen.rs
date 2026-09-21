//! Shared cameras for nearby local players and rotating Voronoi view boundaries.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SplitScreen {
    pub views: Vec<SplitView>,
    positions: Vec<[f32; 2]>,
    #[serde(default)]
    pub divider_alpha: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitView {
    pub members: Vec<u8>,
    pub center: [f32; 2],
    pub site: [f32; 2],
    pub polygon: Vec<[f32; 2]>,
}
impl SplitScreen {
    pub fn update(&mut self, players: &[(u8, [f32; 2])], width: f32, height: f32, zoom: f32) {
        if players.is_empty() || width <= 0.0 || height <= 0.0 {
            self.views.clear();
            self.divider_alpha *= 0.8;
            if self.divider_alpha < 0.01 {
                self.divider_alpha = 0.0;
            }
            return;
        }
        for &(seat, position) in players {
            if self.positions.len() <= seat as usize {
                self.positions.resize(seat as usize + 1, position);
            }
            let smooth = &mut self.positions[seat as usize];
            for axis in 0..2 {
                smooth[axis] += (position[axis] - smooth[axis]) * 0.25;
            }
        }
        let mut groups: Vec<Vec<u8>> = players.iter().map(|(id, _)| vec![*id]).collect();
        let distance_limit = width.min(height) / zoom.max(0.01);
        loop {
            let mut candidate = None;
            'pairs: for a in 0..groups.len() {
                for b in a + 1..groups.len() {
                    let was_merged = self.views.iter().any(|view| {
                        groups[a]
                            .iter()
                            .chain(&groups[b])
                            .all(|id| view.members.contains(id))
                    });
                    let threshold = distance_limit * if was_merged { 0.65 } else { 0.5 };
                    if groups[a].iter().all(|&i| {
                        groups[b].iter().all(|&j| {
                            distance_squared(self.positions[i as usize], self.positions[j as usize])
                                <= threshold * threshold
                        })
                    }) {
                        candidate = Some((a, b));
                        break 'pairs;
                    }
                }
            }
            let Some((a, b)) = candidate else {
                break;
            };
            let other = groups.remove(b);
            groups[a].extend(other);
        }
        let centers: Vec<[f32; 2]> = groups
            .iter()
            .map(|group| {
                let mut center = [0.0; 2];
                for &id in group {
                    for axis in 0..2 {
                        center[axis] += self.positions[id as usize][axis] / group.len() as f32;
                    }
                }
                center
            })
            .collect();
        let mut mean = [0.0; 2];
        for &(seat, _) in players {
            for axis in 0..2 {
                mean[axis] += self.positions[seat as usize][axis] / players.len() as f32;
            }
        }
        let extent = centers
            .iter()
            .map(|p| ((p[0] - mean[0]).abs() / width).max((p[1] - mean[1]).abs() / height))
            .fold(0.01f32, f32::max);
        // Until the group exceeds the shared view, every camera has exactly
        // the same world transform. Boundaries can appear/disappear without
        // a jump; farther apart, compression keeps each group on screen.
        let projection = zoom.min(0.32 / extent);
        let mut sites: Vec<_> = centers
            .iter()
            .map(|p| {
                [
                    width * 0.5 + (p[0] - mean[0]) * projection,
                    height * 0.5 + (p[1] - mean[1]) * projection,
                ]
            })
            .collect();
        for index in 0..sites.len() {
            for other in 0..index {
                if distance_squared(sites[index], sites[other]) < 0.01 {
                    sites[index][0] += 0.25 * (index + 1) as f32;
                }
            }
        }
        self.views = groups
            .into_iter()
            .enumerate()
            .map(|(index, members)| {
                let mut polygon = vec![[0., 0.], [width, 0.], [width, height], [0., height]];
                for (other, site) in sites.iter().enumerate() {
                    if other != index {
                        polygon = clip_cell(&polygon, sites[index], *site);
                    }
                }
                SplitView {
                    members,
                    center: centers[index],
                    site: sites[index],
                    polygon,
                }
            })
            .collect();
        let target_alpha = if self.views.len() > 1 { 1.0 } else { 0.0 };
        self.divider_alpha += (target_alpha - self.divider_alpha) * 0.2;
        if (self.divider_alpha - target_alpha).abs() < 0.01 {
            self.divider_alpha = target_alpha;
        }
    }
}
fn distance_squared(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}
fn clip_cell(points: &[[f32; 2]], a: [f32; 2], b: [f32; 2]) -> Vec<[f32; 2]> {
    let normal = [b[0] - a[0], b[1] - a[1]];
    let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let signed = |p: [f32; 2]| (p[0] - midpoint[0]) * normal[0] + (p[1] - midpoint[1]) * normal[1];
    let mut output = Vec::new();
    for i in 0..points.len() {
        let p = points[i];
        let q = points[(i + 1) % points.len()];
        let dp = signed(p);
        let dq = signed(q);
        if dp <= 0. {
            output.push(p);
        }
        if (dp <= 0.) != (dq <= 0.) {
            let t = dp / (dp - dq);
            output.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cameras_keep_border_players_in_their_own_cells() {
        let mut layout = SplitScreen::default();
        layout.update(&[(0, [10., 10.]), (1, [2010., 2010.])], 1000., 800., 1.);
        let base = crate::host::ViewportState::new(1000., 800.);
        for view in &layout.views {
            let camera = view.viewport(&base);
            let point = camera.map_to_screen_unclamped(robin_engine::coordinates::MapPoint::new(
                view.center[0],
                view.center[1],
            ));
            assert!((point.x - view.site[0]).abs() < 0.001);
            assert!((point.y - view.site[1]).abs() < 0.001);
            assert!(view.contains([point.x, point.y]));
            let cursor = view.clamp_cursor([1000., -1000.]);
            assert!(view.contains(cursor));
        }
    }
    #[test]
    fn merging_groups_share_the_same_camera_transform() {
        let mut layout = SplitScreen::default();
        // Retain split membership in the hysteresis band, while both cameras
        // already agree spatially before their divider disappears.
        layout.update(&[(0, [1000., 1000.]), (1, [2000., 1000.])], 1000., 800., 1.);
        for _ in 0..60 {
            layout.update(&[(0, [1000., 1000.]), (1, [1500., 1000.])], 1000., 800., 1.);
        }
        assert_eq!(layout.views.len(), 2);
        let base = crate::host::ViewportState::new(1000., 800.);
        assert_eq!(
            layout.views[0].viewport(&base).view_position,
            layout.views[1].viewport(&base).view_position
        );
    }
    #[test]
    fn five_views_partition_the_screen_and_regroup() {
        let mut layout = SplitScreen::default();
        let players = [
            (0, [0., 0.]),
            (1, [2000., 0.]),
            (2, [2000., 2000.]),
            (3, [0., 2000.]),
            (4, [1000., 1000.]),
        ];
        layout.update(&players, 1000., 800., 1.);
        assert_eq!(layout.views.len(), 5);
        let area: f32 = layout
            .views
            .iter()
            .map(|v| {
                (0..v.polygon.len())
                    .map(|i| {
                        let a = v.polygon[i];
                        let b = v.polygon[(i + 1) % v.polygon.len()];
                        a[0] * b[1] - b[0] * a[1]
                    })
                    .sum::<f32>()
                    .abs()
                    * 0.5
            })
            .sum();
        assert!((area - 800000.).abs() < 1.);
        for _ in 0..60 {
            layout.update(
                &[
                    (0, [100., 100.]),
                    (1, [110., 100.]),
                    (2, [120., 100.]),
                    (3, [130., 100.]),
                    (4, [140., 100.]),
                ],
                1000.,
                800.,
                1.,
            );
        }
        assert_eq!(layout.views.len(), 1);
        assert_eq!(layout.views[0].members.len(), 5);
    }
}

impl SplitView {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.polygon.len() >= 3
            && (0..self.polygon.len()).all(|index| {
                let a = self.polygon[index];
                let b = self.polygon[(index + 1) % self.polygon.len()];
                (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]) >= -0.01
            })
    }
    pub fn clamp_cursor(&self, point: [f32; 2]) -> [f32; 2] {
        if self.contains(point) {
            return point;
        }
        let mut low = 0.;
        let mut high = 1.;
        for _ in 0..16 {
            let t = (low + high) * 0.5;
            let p = [
                self.site[0] + (point[0] - self.site[0]) * t,
                self.site[1] + (point[1] - self.site[1]) * t,
            ];
            if self.contains(p) {
                low = t;
            } else {
                high = t;
            }
        }
        [
            self.site[0] + (point[0] - self.site[0]) * low,
            self.site[1] + (point[1] - self.site[1]) * low,
        ]
    }
    pub fn viewport(&self, base: &crate::host::ViewportState) -> crate::host::ViewportState {
        let mut viewport = base.clone();
        // Each group's center must stay inside its cell even at map edges.
        // Clamping to the map bounds can otherwise hide a hero behind another
        // camera's polygon. The renderer supplies empty space outside the map.
        viewport.view_position = robin_engine::coordinates::MapPoint::new(
            self.center[0] - self.site[0] / viewport.zoom_factor,
            self.center[1] - self.site[1] / viewport.zoom_factor,
        );
        viewport
    }
}
