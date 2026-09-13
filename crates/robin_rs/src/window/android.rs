//! Android Activity back-button bridge (JNI entry point).

use robin_util::sync::lock;

use super::{HostMsg, report_window_send};
use crate::gfx_types::GameEvent;

/// Event sender reachable from the JNI callback, which has no handler context.
static ANDROID_BACK_TX: std::sync::Mutex<Option<async_channel::Sender<HostMsg>>> =
    std::sync::Mutex::new(None);

/// Route future Activity back presses into the game's event channel.
pub(super) fn install_back_sender(events_tx: async_channel::Sender<HostMsg>) {
    *lock(&ANDROID_BACK_TX) = Some(events_tx);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_phiresky_robinhood_RobinHoodActivity_nativeOnBackPressed(
    _env: *mut std::ffi::c_void,
    _this: *mut std::ffi::c_void,
) {
    tracing::info!("Android Back pressed");
    if let Some(tx) = lock(&ANDROID_BACK_TX).as_ref() {
        report_window_send(tx.try_send(HostMsg::Event(GameEvent::MenuToggleRequested)));
    }
}
