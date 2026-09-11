//! Terrain candidate ordering and checked optional reads. Encoding eligibility
//! (including authored PNG overlays) and shipping precedence remain caller policy.

use robin_data_io::sbfile::{SB_FILE_READ, SbFile, SbFileSystem};

pub fn candidate_paths(root: &str, ambiance: &str, map: &str, extension: &str) -> [String; 3] {
    [
        format!("{root}/{ambiance}/{map}.{extension}"),
        format!("{root}/Day/{map}.{extension}"),
        format!("{root}/{map}.{extension}"),
    ]
}

/// Only an absent candidate permits fallback. Once a candidate exists, a
/// read failure (including disappearance between probe and open) is an error.
/// Retain the opened stream so decoding uses these bytes, not a second lookup.
pub fn open_candidate(path: &str, files: &SbFileSystem) -> Result<Option<SbFile>, String> {
    if !files
        .try_exists(path)
        .map_err(|status| format!("failed to probe terrain image '{path}': {status}"))?
    {
        return Ok(None);
    }
    files
        .open(path, SB_FILE_READ)
        .map(Some)
        .map_err(|status| format!("failed to open terrain image '{path}': {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn candidate_order_preserves_authored_case_and_extension() {
        assert_eq!(
            candidate_paths("Data/Levels", "Night", "Castle", "min"),
            [
                "Data/Levels/Night/Castle.min",
                "Data/Levels/Day/Castle.min",
                "Data/Levels/Castle.min",
            ]
        );
    }

    #[test]
    fn missing_candidate_allows_day_fallback_but_invalid_path_does_not() {
        let vfs = Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("Levels/Day/Test.map", b"day".to_vec())
            .unwrap();
        let files = SbFileSystem::new(vfs);
        let candidates = candidate_paths("Levels", "Night", "Test", "map");
        assert!(open_candidate(&candidates[0], &files).unwrap().is_none());
        assert_eq!(
            &*open_candidate(&candidates[1], &files)
                .unwrap()
                .unwrap()
                .into_shared_bytes(),
            b"day"
        );
        assert!(
            open_candidate("Levels/../escape.map", &files)
                .err()
                .unwrap()
                .contains("failed to probe")
        );
    }

    #[test]
    fn corrupt_first_candidate_is_selected_not_replaced_by_a_valid_fallback() {
        let vfs = Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("Levels/Night/Test.map", vec![1])
            .unwrap();
        vfs.install_preloaded_asset("Levels/Day/Test.map", vec![1, 0, 2, 0])
            .unwrap();
        let files = SbFileSystem::new(vfs);
        let candidates = candidate_paths("Levels", "Night", "Test", "map");
        let bytes = open_candidate(&candidates[0], &files)
            .unwrap()
            .unwrap()
            .into_shared_bytes();
        assert!(crate::picture::Picture::terrain_dimensions(&bytes).is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn present_unreadable_candidate_is_an_error_even_with_a_valid_day_fallback() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Night/Test.map")).unwrap();
        std::fs::create_dir_all(root.path().join("Day")).unwrap();
        std::fs::write(root.path().join("Day/Test.map"), [1, 0, 2, 0]).unwrap();
        let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(files.set_primary_path(root.path().to_str().unwrap()), 0);
        assert!(files.try_exists("Night/Test.map").unwrap());
        let error = open_candidate("Night/Test.map", &files)
            .err()
            .expect("a directory cannot be read as terrain");
        assert!(
            error.contains("failed to open terrain image 'Night/Test.map'"),
            "{error}"
        );
        assert!(open_candidate("Day/Test.map", &files).unwrap().is_some());
    }
}
