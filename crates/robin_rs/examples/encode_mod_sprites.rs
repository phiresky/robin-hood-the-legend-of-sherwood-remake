//! Encode all custom sprite directories in an existing mod, retaining its
//! other authored assets. Usage: encode_mod_sprites SOURCE DESTINATION
use anyhow::{Context, Result, ensure};
use std::path::Path;

fn copy_mod(source: &Path, destination: &Path) -> Result<usize> {
    std::fs::create_dir(destination)?;
    if source
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.ends_with(".rhs.d"))
    {
        let count = robin_rs::game_session::encode_custom_sprite_dir(
            source,
            &destination.join("sprites.vq.zst"),
        )?;
        println!("Verified {count} frames: {}", source.display());
        return Ok(count);
    }
    let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut frames = 0;
    for entry in entries {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || name == "__pycache__" {
            continue;
        }
        let path = entry.path();
        let target = destination.join(name);
        if entry.file_type()?.is_dir() {
            frames += copy_mod(&path, &target)?;
        } else {
            ensure!(
                entry.file_type()?.is_file(),
                "unsupported source entry: {}",
                path.display()
            );
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(frames)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: encode_mod_sprites SOURCE DESTINATION"
    );
    let source = Path::new(&args[0]);
    let destination = Path::new(&args[1]);
    ensure!(source.is_dir(), "source is not a directory");
    ensure!(!destination.exists(), "destination already exists");
    let count = copy_mod(source, destination).with_context(|| source.display().to_string())?;
    println!("Verified {count} frames in {}", destination.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_conversion_verifies_frames_and_retains_authored_assets() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let character = source.join("Data/Characters/Test.rhs.d");
        std::fs::create_dir_all(&character).unwrap();
        let manifest = serde_json::json!({
            "pixel_format": "legacy_color_keys", "profiles": [{
                "name": "Test", "width": 5.0, "height": 1.0,
                "center_x": 2.0, "center_y": 1.0,
                "rows": [{"action_id": 3, "action_done": 0, "average_speed": 1.0,
                    "hotspot_x": 0.0, "hotspot_y": 0.0, "path": ".",
                    "frames": [{"file": "frame.png", "delay": 7, "distance": 2,
                        "offset_x": -3.0, "offset_y": 4.0, "sound_id": 5}]}]
            }]
        });
        std::fs::write(character.join("manifest.json"), manifest.to_string()).unwrap();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 5, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[0, 248, 0, 0, 0, 255, 255, 0, 0, 0, 248, 0, 255, 255, 255])
                .unwrap();
        }
        std::fs::write(character.join("frame.png"), png).unwrap();
        std::fs::write(source.join("details.json"), "{\"author\":\"Test\"}").unwrap();
        let destination = temp.path().join("encoded");
        // The export API verifies all pixels, widths, offsets, timing, sounds
        // and animation mappings using the same decoder as mission loading.
        assert_eq!(copy_mod(&source, &destination).unwrap(), 1);
        let encoded = destination.join("Data/Characters/Test.rhs.d");
        assert!(encoded.join("sprites.vq.zst").is_file());
        assert!(!encoded.join("frame.png").exists());
        assert!(!encoded.join("manifest.json").exists());
        assert_eq!(
            std::fs::read(source.join("details.json")).unwrap(),
            std::fs::read(destination.join("details.json")).unwrap()
        );
        assert!(
            copy_mod(&source, &destination).is_err(),
            "never overwrite an existing mod"
        );
        std::fs::remove_file(character.join("frame.png")).unwrap();
        assert!(copy_mod(&source, &temp.path().join("missing-frame")).is_err());
    }
}
