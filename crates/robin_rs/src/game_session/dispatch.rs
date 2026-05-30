//! Local command dispatch: routes player commands either to the local
//! engine (single-player) or the multiplayer transport.

use crate::Host;
use crate::geo2d;
use crate::player_command::{FrameCommands, PlayerCommand};
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};

/// Apply a batch of locally-produced [`PlayerCommand`]s.
///
/// In single-player (`host.net.is_none()`), commands are applied
/// directly to the engine — same behaviour as the old
/// `engine.apply_local_commands` call.
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
    engine: &mut Engine,
    assets: &LevelAssets,
    cmds: &[PlayerCommand],
) {
    if let Some(net) = host.net.as_ref() {
        for cmd in cmds {
            net.send_input(cmd.clone());
        }
    } else {
        engine.apply_local_commands(&mut host.engine_display, &mut host.input, assets, cmds);
    }
}

/// Single-command convenience wrapper around [`dispatch_local_commands`].
///
/// In single-player: pushes `cmd` to `frame_cmds` (so the replay
/// recorder / rewind buffer / rollback checker capture it) and then
/// applies it to the engine.  In multiplayer: sends `cmd` over the
/// wire and DOES NOT push to `frame_cmds` or mutate the local engine
/// — the command echoes back through the server at `target_frame =
/// sim_frame + INPUT_DELAY_FRAMES` and `drain_net_inputs` populates
/// `frame_cmds` at that frame instead.  This is the reason MP
/// dispatch must be sealed: a direct `engine.apply_command(...)` would
/// mutate the local engine but not the peers' engines, instant
/// desync.
///
pub(crate) fn dispatch_local_command(
    host: &mut Host,
    engine: &mut Engine,
    frame_cmds: &mut FrameCommands,
    assets: &LevelAssets,
    cmd: &PlayerCommand,
) {
    if let Some(net) = host.net.as_ref() {
        net.send_input(cmd.clone());
    } else {
        frame_cmds.push(cmd.clone());
        engine.apply_local_commands(
            &mut host.engine_display,
            &mut host.input,
            assets,
            std::slice::from_ref(cmd),
        );
    }
}

pub(super) fn apply_local_viewport_scroll(host: &mut Host, dir: engine_api::ScrollDirection) {
    const STEP: f32 = 24.0;
    let delta = match dir {
        engine_api::ScrollDirection::Up => geo2d::pt(0.0, -STEP),
        engine_api::ScrollDirection::Down => geo2d::pt(0.0, STEP),
        engine_api::ScrollDirection::Left => geo2d::pt(-STEP, 0.0),
        engine_api::ScrollDirection::Right => geo2d::pt(STEP, 0.0),
    };
    host.viewport.scroll_by(delta);
    host.input.cancel_multi_selection();
}
