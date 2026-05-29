//! F7 teleport cheat — dispatch a teleport movement element for every
//! selected PC.
//!
//! Triggered by the `MSG_TELEPORT` input event.  The first selected PC
//! is teleported to the mouse map point; every subsequent PC keeps its
//! offset relative to the first.
//!
//! When there is no selection and the free-shadow-polygon dev cheat is
//! active, the teleport instead repositions the free-floating shadow-
//! polygon viewer.  That branch writes to non-deterministic dev state
//! (`DevState::cheat_free_shadow_polygon_pos`) so it is handled host-
//! side in `game_session.rs` rather than going through the
//! PlayerCommand / replay pipeline.

use super::EngineInner;
use crate::element::{Command, EntityId, Point2D};
use crate::geo2d;
use crate::order::OrderType;
use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData};

impl EngineInner {
    /// Teleport every selected PC.  The first selected PC lands on
    /// `dest`; subsequent PCs keep their offset from the first (the
    /// displacement is `dest - first_pc.position`).
    ///
    /// Each teleport is dispatched as a one-element sequence carrying
    /// a [`Command::Teleport`] movement element — the actual position
    /// snap + star-burst effects are handled by the existing
    /// `Command::Teleport` branch in `engine::tick`.
    pub(crate) fn manage_input_process_teleport(
        &mut self,
        dest: geo2d::Point2D,
        layer: u16,
        sector: Option<crate::position_interface::SectorHandle>,
    ) {
        let selected = self.seats[0].selection.clone();
        if selected.is_empty() {
            return;
        }

        let dest = Point2D {
            x: dest.x,
            y: dest.y,
        };

        // Snapshot the first PC's current position to compute the
        // displacement that the rest of the group preserves.
        let first_pos = match self
            .get_entity(selected[0])
            .map(|e| e.element_data().position_map())
        {
            Some(p) => p,
            None => return,
        };

        for (idx, pc_id) in selected.iter().enumerate() {
            let pc_dest = if idx == 0 {
                dest
            } else {
                let pos = match self
                    .get_entity(*pc_id)
                    .map(|e| e.element_data().position_map())
                {
                    Some(p) => p,
                    None => continue,
                };
                Point2D {
                    x: pos.x + (dest.x - first_pos.x),
                    y: pos.y + (dest.y - first_pos.y),
                }
            };

            let mut elem = SequenceElement::new_movement(
                1,
                Command::Teleport,
                Some(*pc_id),
                OrderType::RunningUpright,
            );
            elem.data = SequenceElementData::Movement {
                destination: crate::coordinates::MapPoint {
                    x: pc_dest.x,
                    y: pc_dest.y,
                },
                layer,
                sector,
                gate_id: None,
                line_id: None,
                element: None,
                flags: MoveFlags::empty(),
                tolerance: 0.0,
                direction: 0,
                action: OrderType::RunningUpright,
                speed_factor: 1.0,
                post_seek_sequence: None,
            };
            self.launch_element(elem);
        }
    }

    /// Jitter an element's map position by ±5 units on both axes and
    /// randomise its facing to one of 16 directions.
    ///
    /// The position jitter is only committed when the candidate bbox is
    /// collision-free (`is_position_authorized`) AND a straight-line
    /// path from the current position to the candidate is clear
    /// (`is_reachable_thin`).  The facing is ALWAYS reseeded — including
    /// when the position commit is rejected.
    ///
    /// Wired in at `engine::level_loading::spawn_sherwood_pcs` — every
    /// returning PC with a Sherwood beam-me index gets a randomised
    /// position around the spawn anchor.
    pub(crate) fn randomize_position(&mut self, eid: EntityId) {
        const RANDOM_SHERWOOD_POSITION: f32 = 5.0;

        // Snapshot what we need before the mutable borrow.  `sim_rng`
        // must fire in a fixed sequence (two axis jitters, then the
        // direction) so replay stays deterministic — do all draws up
        // front rather than inside the authorization branch.
        let (current_pos, layer, move_box) = {
            let Some(e) = self.get_entity(eid) else {
                tracing::warn!(?eid, "randomize_position: missing entity");
                return;
            };
            let ed = e.element_data();
            let pos = ed.position_map();
            let layer = ed.layer();
            let move_box = *ed.sprite.position_iface.get_move_box();
            (pos, layer, move_box)
        };

        let dx = (crate::sim_rng::f32() * 2.0 - 1.0) * RANDOM_SHERWOOD_POSITION;
        let dy = (crate::sim_rng::f32() * 2.0 - 1.0) * RANDOM_SHERWOOD_POSITION;
        let new_direction = crate::sim_rng::u32(0..16) as i16;

        let new_pos = crate::coordinates::MapPoint {
            x: current_pos.x + dx,
            y: current_pos.y + dy,
        };
        let bbox_at_new = move_box.translated(geo2d::pt(new_pos.x, new_pos.y));

        let authorized = self.fast_grid.is_position_authorized(&bbox_at_new, layer)
            && self.fast_grid.is_reachable_thin(
                geo2d::pt(current_pos.x, current_pos.y),
                geo2d::pt(new_pos.x, new_pos.y),
                layer,
            );

        let Some(entity) = self.get_entity_mut(eid) else {
            return;
        };
        if authorized {
            entity.element_data_mut().sprite.position_iface.new_move();
            entity.element_data_mut().set_position_map(new_pos);
        }
        entity
            .element_data_mut()
            .set_direction_instantly(new_direction);
    }
}
