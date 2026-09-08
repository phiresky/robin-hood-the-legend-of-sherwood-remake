//! Seat membership updates. Host authorization belongs to the outer dispatcher;
//! payload identity, rather than the issuing seat, names the member being changed.

use crate::engine::EngineInner;
use crate::player_command::PlayerId;

impl EngineInner {
    pub(super) fn dispatch_connect_seat(&mut self, target: &PlayerId, nickname: &str) {
        let idx = self.ensure_seat(*target);
        let was_connected = self.players.seats[idx].connected;
        self.players.seats[idx].connected = true;
        self.players.seats[idx].nickname = nickname.to_owned();
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
