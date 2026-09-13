use super::actor_line_crossing_eligible;
use crate::element::Posture;

#[test]
fn wall_and_ladder_climbers_still_check_elevation_lines() {
    assert!(actor_line_crossing_eligible(Posture::OnWall, false, true));
    assert!(actor_line_crossing_eligible(Posture::OnLadder, false, true));
    assert!(!actor_line_crossing_eligible(Posture::Flying, false, true));
    assert!(!actor_line_crossing_eligible(Posture::OnWall, true, true));
    assert!(!actor_line_crossing_eligible(Posture::OnWall, false, false));
}
