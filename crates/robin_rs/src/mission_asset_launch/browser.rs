//! Browser builds have no installed mod roots; custom missions arrive only as
//! canonical in-memory distributed content.

use super::PreparedLiveMissionAssets;
use std::sync::Arc;

pub fn prepare_installed_custom_mission(
    _launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
    _files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<PreparedLiveMissionAssets, String> {
    Err("browser custom missions must arrive as canonical in-memory distributed content".to_owned())
}
