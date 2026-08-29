//! Audit reusable-cloak animation eligibility for every character profile in
//! one or more loose game-data directories.
//!
//! Usage: `cloak_art_audit <datadir> [<datadir> ...]`

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use robin_engine::order::OrderType;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::{SB_FILE_READ, SbFile};
use robin_engine::sprite_script::{FrameKind, SpriteInfo, SpriteScriptor, UNMAPPED};

fn main() -> Result<()> {
    let roots = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(anyhow!("usage: cloak_art_audit <datadir> [<datadir> ...]"));
    }

    for root in roots {
        audit_datadir(&root)?;
    }
    Ok(())
}

fn audit_datadir(root: &Path) -> Result<()> {
    let data = [root.join("Data"), root.join("DATA")]
        .into_iter()
        .find(|path| path.is_dir())
        .ok_or_else(|| anyhow!("{} has neither Data/ nor DATA/", root.display()))?;
    let cpf = data.join("Configuration/profile.cpf");
    let mut file = SbFile::open(&cpf.to_string_lossy(), SB_FILE_READ).map_err(|code| {
        anyhow!(
            "open {} failed with legacy file error {code}",
            cpf.display()
        )
    })?;
    let mut profiles = ProfileManager::new();
    profiles
        .load_all_legacy_cpf(&mut file)
        .with_context(|| format!("decode {}", cpf.display()))?;

    let mut scriptor = SpriteScriptor::new();
    let mut tracks = 0usize;
    let mut eligible = 0usize;
    let mut unavailable = 0usize;
    let mut partial = 0usize;
    println!("datadir={}", root.display());
    for profile in &profiles.characters {
        audit_track(
            &mut scriptor,
            &data,
            profile.index,
            &profile.filename,
            "primary",
            &profile.profile_name,
            &mut tracks,
            &mut eligible,
            &mut unavailable,
            &mut partial,
        )?;
        if profile.valid_alternative_profile && !profile.alternative_profile_name.is_empty() {
            audit_track(
                &mut scriptor,
                &data,
                profile.index,
                &profile.filename,
                "alternate",
                &profile.alternative_profile_name,
                &mut tracks,
                &mut eligible,
                &mut unavailable,
                &mut partial,
            )?;
        }
    }
    println!(
        "summary characters={} tracks={} available={} eligible={} ineligible={} unavailable={} partial={}",
        profiles.characters.len(),
        tracks,
        tracks - unavailable,
        eligible,
        tracks - unavailable - eligible,
        unavailable,
        partial,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn audit_track(
    scriptor: &mut SpriteScriptor,
    data: &Path,
    profile_index: u32,
    filename: &str,
    track: &str,
    profile_name: &str,
    tracks: &mut usize,
    eligible: &mut usize,
    unavailable: &mut usize,
    partial: &mut usize,
) -> Result<()> {
    let rhs = data.join("Characters").join(format!("{filename}.rhs"));
    *tracks += 1;
    if !rhs.is_file() {
        *unavailable += 1;
        println!(
            "character={profile_index} file={filename:?} track={track} profile={profile_name:?} status=unavailable"
        );
        return Ok(());
    }
    let cache_key = format!("{filename}/{profile_name}");
    let info = scriptor
        .load(
            &rhs.to_string_lossy(),
            profile_name,
            &cache_key,
            FrameKind::Character,
            |file| {
                let mut signature = 0u32;
                file.serialize_u32(&mut signature)
                    .map_err(|error| format!("read RHS signature: {error}"))
            },
        )
        .map_err(|error| anyhow!("{} profile {:?}: {error}", rhs.display(), profile_name))?;
    let waiting = has_animation(info, OrderType::WaitingCape);
    let transition = has_animation(info, OrderType::TransitionWaitingCapeWaitingUpright);
    let can_cloak = waiting && transition;
    *eligible += usize::from(can_cloak);
    *partial += usize::from(waiting != transition);
    println!(
        "character={profile_index} file={filename:?} track={track} profile={profile_name:?} waiting_cape={waiting} transition={transition} eligible={can_cloak}"
    );
    Ok(())
}

fn has_animation(info: &SpriteInfo, animation: OrderType) -> bool {
    info.conversion
        .get(animation as usize)
        .is_some_and(|&row| row != UNMAPPED)
}
