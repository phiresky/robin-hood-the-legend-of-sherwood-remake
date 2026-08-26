//! Local command dispatch: stages player commands into the authoritative
//! frame transaction or routes them to the multiplayer transport.

use crate::host::Host;
use robin_engine::coordinates::ScreenVec;
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::{FrameCommands, PlayerCommand};

/// Admit a batch of locally-produced [`PlayerCommand`]s.
///
/// In single-player (`host.transport.net.is_none()`), commands are staged in
/// `frame_cmds` and are applied only by `Engine::advance_frame`.
///
/// In multiplayer, commands are sent through the net layer instead.
/// On the server the broadcast pump immediately echoes them back
/// into the incoming queue, where the per-frame drain applies them
/// — so the server still sees zero apply lag for its own inputs.
/// Clients send to the server, which stamps the seat and broadcasts;
/// the originating client receives the echo over the wire and
/// applies it then.  This keeps every machine's apply order
/// identical to the order the server saw the inputs in (deterministic
/// across machines), at the cost of one network RTT of input lag for
/// clients.
pub(crate) fn dispatch_local_commands(
    host: &mut Host,
    _engine: &mut Engine,
    frame_cmds: &mut FrameCommands,
    _assets: &LevelAssets,
    cmds: &[PlayerCommand],
) {
    if let Some(net) = host.transport.net.as_ref() {
        for cmd in cmds {
            net.send_input(cmd.clone());
        }
    } else {
        frame_cmds
            .commands
            .extend(cmds.iter().cloned().map(Into::into));
    }
}

/// Single-command convenience wrapper around [`dispatch_local_commands`].
///
/// In single-player: pushes `cmd` to `frame_cmds` (so the replay
/// recorder / rewind buffer / rollback checker capture it); the frame
/// transaction applies it. In multiplayer: sends `cmd` over the
/// wire and DOES NOT push to `frame_cmds` or mutate the local engine
/// — the command echoes back through the server at `target_frame =
/// sim_frame + INPUT_DELAY_FRAMES` and `drain_net_inputs` populates
/// `frame_cmds` at that frame instead.  This is the reason MP
/// dispatch must be sealed: using the legacy immediate engine mutator would
/// mutate the local engine but not the peers' engines, instant
/// desync.
///
pub(crate) fn dispatch_local_command(
    host: &mut Host,
    _engine: &mut Engine,
    frame_cmds: &mut FrameCommands,
    _assets: &LevelAssets,
    cmd: &PlayerCommand,
) {
    if let Some(net) = host.transport.net.as_ref() {
        net.send_input(cmd.clone());
    } else {
        frame_cmds.push(cmd.clone());
    }
}

pub(super) fn apply_local_viewport_scroll(host: &mut Host, dir: engine_api::ScrollDirection) {
    const STEP: f32 = 24.0;
    let delta = match dir {
        engine_api::ScrollDirection::Up => ScreenVec::new(0.0, -STEP),
        engine_api::ScrollDirection::Down => ScreenVec::new(0.0, STEP),
        engine_api::ScrollDirection::Left => ScreenVec::new(-STEP, 0.0),
        engine_api::ScrollDirection::Right => ScreenVec::new(STEP, 0.0),
    };
    host.viewport.scroll_by(delta);
    host.input.cancel_multi_selection();
}

#[cfg(test)]
mod tests {
    use super::dispatch_local_command;
    use crate::host::Host;
    use crate::multiplayer::{NetChannels, NetOutbound};
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{Engine, LevelAssets};
    use robin_engine::player_command::{FrameCommands, PlayerCommand};

    #[test]
    fn single_player_dispatch_records_each_command_exactly_once() {
        let mut assets = LevelAssets::default();
        let mut engine = Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut assets)
            .expect("fixture engine");
        let mut host = Host::scratch(640.0, 480.0);
        let mut commands = FrameCommands::new();

        dispatch_local_command(
            &mut host,
            &mut engine,
            &mut commands,
            &assets,
            &PlayerCommand::QuitMissionRequested,
        );

        assert_eq!(commands.commands.len(), 1);
    }

    #[test]
    fn multiplayer_dispatch_defers_recording_until_the_server_echo() {
        let mut assets = LevelAssets::default();
        let mut engine = Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut assets)
            .expect("fixture engine");
        let mut host = Host::scratch(640.0, 480.0);
        let (channels, _incoming, outgoing, _, _) = NetChannels::new();
        host.transport.net = Some(channels);
        let mut commands = FrameCommands::new();

        dispatch_local_command(
            &mut host,
            &mut engine,
            &mut commands,
            &assets,
            &PlayerCommand::QuitMissionRequested,
        );

        assert!(commands.commands.is_empty());
        assert!(matches!(
            outgoing.recv().expect("outbound command"),
            NetOutbound::Input {
                command: PlayerCommand::QuitMissionRequested,
                ..
            }
        ));
    }
}
