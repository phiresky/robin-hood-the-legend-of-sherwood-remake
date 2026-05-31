//! Path — a series of waypoints that characters follow.
//!
//! Rust port of `RHPath` / `RHWaypoint` semantics.

/// A single waypoint on a path.
#[derive(
    Debug, Clone, Default, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Waypoint {
    pub x: f32,
    pub y: f32,
    pub layer: u16,
    pub sector: u16,
}

/// A path consisting of an ordered series of waypoints, with a current position index.
#[derive(
    Debug, Clone, Default, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Path {
    pub waypoints: Vec<Waypoint>,
    pub current_index: usize,
}

impl Waypoint {
    pub fn new(x: f32, y: f32, layer: u16, sector: u16) -> Self {
        Self {
            x,
            y,
            layer,
            sector,
        }
    }
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a waypoint to the end of the path.
    pub fn add_waypoint(&mut self, wp: Waypoint) {
        self.waypoints.push(wp);
    }

    /// Return a reference to the current waypoint, or `None` if the path is empty.
    pub fn get_current(&self) -> Option<&Waypoint> {
        self.waypoints.get(self.current_index)
    }

    /// Advance to the next waypoint. Returns `true` if the index moved,
    /// `false` if already at the end (or the path is empty).
    pub fn advance(&mut self) -> bool {
        if self.current_index < self.waypoints.len().saturating_sub(1) {
            self.current_index += 1;
            true
        } else {
            false
        }
    }

    /// Returns `true` when the current index has reached the last waypoint
    /// (or the path is empty).
    pub fn is_complete(&self) -> bool {
        self.waypoints.is_empty() || self.current_index >= self.waypoints.len() - 1
    }

    /// Number of waypoints remaining after the current one.
    pub fn remaining_count(&self) -> usize {
        self.waypoints
            .len()
            .saturating_sub(1)
            .saturating_sub(self.current_index)
    }

    /// Total length of the path (sum of Euclidean distances between consecutive waypoints).
    pub fn total_length(&self) -> f32 {
        self.waypoints
            .windows(2)
            .map(|pair| {
                let dx = pair[1].x - pair[0].x;
                let dy = pair[1].y - pair[0].y;
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }

    /// Reverse the order of waypoints and reset `current_index` to 0.
    pub fn reverse(&mut self) {
        self.waypoints.reverse();
        self.current_index = 0;
    }

    /// Remove all waypoints and reset the index.
    pub fn clear(&mut self) {
        self.waypoints.clear();
        self.current_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_path() -> Path {
        let mut p = Path::new();
        p.add_waypoint(Waypoint::new(0.0, 0.0, 0, 0));
        p.add_waypoint(Waypoint::new(3.0, 4.0, 0, 0));
        p.add_waypoint(Waypoint::new(6.0, 8.0, 0, 1));
        p
    }

    #[test]
    fn build_and_traverse() {
        let mut p = sample_path();
        assert_eq!(p.waypoints.len(), 3);
        assert_eq!(p.current_index, 0);
        assert!(!p.is_complete());
        assert_eq!(p.remaining_count(), 2);

        // Advance through all waypoints
        assert!(p.advance());
        assert_eq!(p.current_index, 1);
        assert!(p.advance());
        assert_eq!(p.current_index, 2);
        assert!(p.is_complete());
        assert_eq!(p.remaining_count(), 0);

        // Cannot advance past end
        assert!(!p.advance());
        assert_eq!(p.current_index, 2);
    }

    #[test]
    fn total_length_calculation() {
        let p = sample_path();
        // Two segments of length 5 each (3-4-5 triangle)
        let expected = 10.0_f32;
        assert!((p.total_length() - expected).abs() < 1e-4);
    }

    #[test]
    fn reverse_path() {
        let mut p = sample_path();
        p.advance(); // index = 1
        p.reverse();
        assert_eq!(p.current_index, 0);
        let first = p.get_current().unwrap();
        assert!((first.x - 6.0).abs() < 1e-6);
        assert_eq!(first.sector, 1);
    }

    #[test]
    fn clear_path() {
        let mut p = sample_path();
        p.advance();
        p.clear();
        assert!(p.waypoints.is_empty());
        assert_eq!(p.current_index, 0);
        assert!(p.is_complete());
        assert_eq!(p.remaining_count(), 0);
    }

    #[test]
    fn empty_path() {
        let p = Path::new();
        assert!(p.is_complete());
        assert_eq!(p.remaining_count(), 0);
        assert_eq!(p.total_length(), 0.0);
        assert!(p.get_current().is_none());
    }

    #[test]
    fn out_of_range_index_is_complete_without_underflow() {
        let mut p = sample_path();
        p.current_index = usize::MAX;

        assert!(p.is_complete());
        assert_eq!(p.remaining_count(), 0);
        assert!(!p.advance());
    }

    #[test]
    fn single_waypoint() {
        let mut p = Path::new();
        p.add_waypoint(Waypoint::new(1.0, 2.0, 3, 4));
        assert!(p.is_complete());
        assert_eq!(p.remaining_count(), 0);
        assert!(!p.advance());
        assert_eq!(p.total_length(), 0.0);
        let wp = p.get_current().unwrap();
        assert!((wp.x - 1.0).abs() < 1e-6);
        assert_eq!(wp.layer, 3);
    }

    #[test]
    fn serde_roundtrip() {
        let p = sample_path();
        let json = serde_json::to_string(&p).unwrap();
        let p2: Path = serde_json::from_str(&json).unwrap();
        assert_eq!(p2.waypoints.len(), 3);
        assert_eq!(p2.current_index, 0);
        assert!((p2.total_length() - p.total_length()).abs() < 1e-6);
    }
}
