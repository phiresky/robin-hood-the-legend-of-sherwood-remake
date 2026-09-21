//! Seat membership updates. Host authorization belongs to the outer dispatcher;
//! payload identity, rather than the issuing seat, names the member being changed.

use crate::engine::EngineInner;
use crate::player_command::PlayerId;

impl EngineInner {
    pub(super) fn dispatch_connect_seat(&mut self, target: &PlayerId, nickname: &str) {
        // A local controller can finish joining on the first gameplay frame.
        // Expand a single-player shared party before filtering the new seat's
        // selection, so late admission cannot leave a connected player with
        // no playable character. Network co-op already carries its final
        // player count in SimConfig and therefore skips this path.
        let required_players = target.0.saturating_add(1).min(5);
        if target.0 > 0
            && required_players > self.control.sim_config.coop.players
            && self.control.sim_config.coop.control == crate::coop::CharacterControl::Shared
        {
            self.control.sim_config.coop.players = required_players;
            self.initialize_coop_party();
            tracing::info!(
                players = self.control.sim_config.coop.players,
                "Expanded local co-op party for late seat"
            );
        }
        let idx = self.ensure_seat(*target);
        let was_connected = self.players.seats[idx].connected;
        self.players.seats[idx].connected = true;
        self.players.seats[idx].nickname = nickname.to_owned();
        let selection = self.players.seats[idx].selection.clone();
        self.players.seats[idx].selection = selection
            .into_iter()
            .filter(|&id| self.coop_can_select(idx, id))
            .collect();
        if was_connected {
            tracing::info!(
                player_id = ?target,
                nickname = %nickname,
                "seat reconnected (nickname updated)"
            );
        } else {
            tracing::info!(
                player_id = ?target,
                nickname = %nickname,
                "seat connected"
            );
        }
    }

    pub(super) fn dispatch_disconnect_seat(&mut self, target: &PlayerId) {
        let idx = target.0 as usize;
        if let Some(s) = self.players.seats.get_mut(idx) {
            if s.connected {
                tracing::info!(
                    player_id = ?target,
                    nickname = %s.nickname,
                    selection_size = s.selection.len(),
                    "seat disconnected (selection preserved)"
                );
            }
            s.connected = false;
        } else {
            tracing::debug!(
                player_id = ?target,
                "DisconnectSeat for unknown seat — ignored"
            );
        }
    }
}
