//! Encode all custom sprite directories in an existing mod, retaining its
//! other authored assets and converting terrain PNGs to JXL quality 80.
//! Usage: encode_mod_sprites SOURCE DESTINATION
use anyhow::{Context, Result, ensure};
use std::path::Path;

fn encode_map(source: &Path, destination: &Path) -> Result<()> {
    ensure!(!destination.exists(), "map destination exists");
    let input =
        png::Decoder::new(std::io::BufReader::new(std::fs::File::open(source)?)).read_info()?;
    let dimensions = (input.info().width, input.info().height);
    let status = std::process::Command::new("cjxl")
        .arg(source)
        .arg(destination)
        .args(["-q", "80", "-e", "7", "--num_threads=4"])
        .status()
        .context("run cjxl")?;
    ensure!(status.success(), "cjxl failed for {}", source.display());
    let picture =
        robin_assets::picture::Picture::load_terrain_from_bytes(&std::fs::read(destination)?)?;
    ensure!(
        (u32::from(picture.width), u32::from(picture.height)) == dimensions,
        "map dimensions changed"
    );
    println!(
        "Verified JXL map: {} ({} bytes)",
        destination.display(),
        std::fs::metadata(destination)?.len()
    );
    Ok(())
}

fn copy_mod(source: &Path, destination: &Path) -> Result<usize> {
    ensure!(source.is_dir(), "source is not a directory");
    ensure!(!destination.exists(), "destination already exists");
    let canonical_source = source
        .canonicalize()
        .with_context(|| format!("resolve source {}", source.display()))?;
    // create_dir requires an existing parent. Resolve that parent before any
    // output is created so aliases cannot hide output inside the input tree.
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_destination = parent
        .canonicalize()
        .with_context(|| format!("resolve destination parent {}", parent.display()))?
        .join(
            destination
                .file_name()
                .context("destination has no directory name")?,
        );
    ensure!(
        !canonical_destination.starts_with(&canonical_source),
        "destination {} is inside source {}",
        destination.display(),
        source.display()
    );
    std::fs::create_dir(destination)?;
    if source
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.ends_with(".rhs.d"))
    {
        let count = robin_assets::custom_sprites::encode_custom_sprite_dir(
            source,
            &destination.join("sprites.vq.zst"),
        )?;
        println!("Verified {count} frames: {}", source.display());
        return Ok(count);
    }
    let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut frames = 0;
    let mut families = std::collections::BTreeMap::<String, Vec<std::path::PathBuf>>::new();
    for entry in entries {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || name == "__pycache__" {
            continue;
        }
        let path = entry.path();
        let target = destination.join(name);
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(".rhs.d"))
        {
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path.join("manifest.json"))?)?;
            let profiles = manifest
                .get("profiles")
                .context("missing sprite profiles")?;
            families
                .entry(serde_json::to_string(profiles)?)
                .or_default()
                .push(path);
            continue;
        }
        if entry.file_type()?.is_dir() {
            frames += copy_mod(&path, &target)?;
        } else {
            ensure!(
                entry.file_type()?.is_file(),
                "unsupported source entry: {}",
                path.display()
            );
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .context("non-UTF8 asset name")?;
            if let Some(map_name) = name.strip_suffix(".map.png") {
                encode_map(&path, &destination.join(format!("{map_name}.map")))?;
            } else if !(name.ends_with(".map") && path.with_extension("map.png").exists()) {
                std::fs::copy(&path, &target)?;
            }
        }
    }
    for (index, sources) in families.values().enumerate() {
        frames += robin_assets::custom_sprites::encode_custom_sprite_family(
            sources,
            &destination.join(format!("family-{index:02}.sprites.vq.zst")),
        )?;
    }
    Ok(frames)
}

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
#[command(about = "Encode custom sprites and terrain in a mod (terrain requires cjxl on PATH)")]
struct Args {
    /// Encode a sprite family: --family OUTPUT INPUT_RHS_DIR...
    #[arg(long, num_args = 2.., conflicts_with_all = ["map", "source", "destination"])]
    family: Vec<std::path::PathBuf>,
    /// Encode a terrain PNG: --map INPUT_PNG OUTPUT_MAP
    #[arg(long, num_args = 2, conflicts_with_all = ["source", "destination"])]
    map: Vec<std::path::PathBuf>,
    #[arg(required_unless_present_any = ["family", "map"])]
    source: Option<std::path::PathBuf>,
    #[arg(required_unless_present_any = ["family", "map"])]
    destination: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let args = <Args as clap::Parser>::parse();
    if !args.family.is_empty() {
        robin_assets::custom_sprites::encode_custom_sprite_family(
            &args.family[1..],
            &args.family[0],
        )?;
        return Ok(());
    }
    if !args.map.is_empty() {
        return encode_map(&args.map[0], &args.map[1]);
    }
    let source = args.source.as_deref().expect("required source");
    let destination = args.destination.as_deref().expect("required destination");
    let count = copy_mod(source, destination).with_context(|| source.display().to_string())?;
    println!("Verified {count} frames in {}", destination.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_output_is_rejected_before_any_output_creation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let nested = source.join("authored");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(source.join("details.json"), b"authored data").unwrap();
        for destination in [source.join("output"), nested.join("output")] {
            let error = copy_mod(&source, &destination).unwrap_err();
            assert!(error.to_string().contains("inside source"), "{error:#}");
            assert!(!destination.exists());
        }
        assert_eq!(
            std::fs::read(source.join("details.json")).unwrap(),
            b"authored data"
        );
        assert_eq!(std::fs::read_dir(&source).unwrap().count(), 2);
        assert_eq!(std::fs::read_dir(&nested).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_destination_parent_cannot_hide_output_inside_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let destination = alias.join("output");
        let error = copy_mod(&source, &destination).unwrap_err();
        assert!(error.to_string().contains("inside source"), "{error:#}");
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&source).unwrap().count(), 0);
    }

    #[test]
    fn sibling_copy_succeeds_and_existing_destination_remains_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("details.json"), b"original").unwrap();
        // A shared textual prefix is not path ancestry.
        let destination = temp.path().join("source-encoded");
        assert_eq!(copy_mod(&source, &destination).unwrap(), 0);
        assert_eq!(
            std::fs::read(destination.join("details.json")).unwrap(),
            b"original"
        );
        std::fs::write(source.join("details.json"), b"changed source").unwrap();
        let error = copy_mod(&source, &destination).unwrap_err();
        assert!(
            error.to_string().contains("destination already exists"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(destination.join("details.json")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn missing_destination_parent_does_not_create_directories() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let parent = temp.path().join("missing");
        assert!(copy_mod(&source, &parent.join("output")).is_err());
        assert!(!parent.exists());
    }

    #[test]
    fn cli_preserves_all_modes_and_rejects_ambiguous_invocations() {
        use clap::Parser;
        let copy = Args::try_parse_from(["encode", "source", "destination"]).unwrap();
        assert_eq!(copy.source.unwrap(), Path::new("source"));
        let family = Args::try_parse_from(["encode", "--family", "output", "a", "b"]).unwrap();
        assert_eq!(family.family.len(), 3);
        let map = Args::try_parse_from(["encode", "--map", "input.png", "output.map"]).unwrap();
        assert_eq!(map.map.len(), 2);
        for arguments in [
            vec!["encode"],
            vec!["encode", "source"],
            vec!["encode", "--map", "only-input"],
            vec!["encode", "--family", "only-output"],
            vec!["encode", "--map", "a", "b", "source", "destination"],
            vec!["encode", "--family", "a", "b", "--map", "c", "d"],
        ] {
            assert!(Args::try_parse_from(arguments).is_err());
        }
    }

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
        for name in ["TestBlue.rhs.d", "TestRed.rhs.d"] {
            let sibling = character.parent().unwrap().join(name);
            std::fs::create_dir(&sibling).unwrap();
            for file in ["frame.png", "manifest.json"] {
                std::fs::copy(character.join(file), sibling.join(file)).unwrap();
            }
        }
        std::fs::write(source.join("details.json"), "{\"author\":\"Test\"}").unwrap();
        let destination = temp.path().join("encoded");
        // The export API verifies all pixels, widths, offsets, timing, sounds
        // and animation mappings using the same decoder as mission loading.
        assert_eq!(copy_mod(&source, &destination).unwrap(), 3);
        let encoded = destination.join("Data/Characters");
        assert!(encoded.join("family-00.sprites.vq.zst").is_file());
        assert!(!encoded.join("Test.rhs.d").exists());
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
