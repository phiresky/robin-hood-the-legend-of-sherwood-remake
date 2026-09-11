//! Inventory the exact compressed costs and sprite/profile sharing of mission payloads.
use anyhow::Result;
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};
use std::{collections::BTreeSet, path::PathBuf};
#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    root: PathBuf,
}

fn main() -> Result<()> {
    let root = <Args as clap::Parser>::parse().root;
    let dd = ShippingDatadir::from_compressed_bytes(&std::fs::read(root.join("datadir.bin"))?)?;
    let mut files = BTreeSet::new();
    for m in dd.missions.values() {
        files.extend(m.files.iter().cloned());
    }
    for deps in dd.character_rhs_files.values() {
        files.extend(deps.iter().cloned());
    }
    for name in files {
        let bytes = std::fs::read(root.join(&name))?;
        let part = decode_mission_compressed(&bytes)?;
        for (mission, level) in &part.levels {
            let refs: Vec<_> = level
                .proto
                .patches
                .iter()
                .map(|p| &p.element_fx.sprite)
                .chain(level.proto.animations.iter().map(|a| &a.sprite))
                .chain(
                    level
                        .mission
                        .mission_patches
                        .iter()
                        .map(|p| &p.element_fx.sprite),
                )
                .chain(
                    level
                        .mission
                        .mobile_elements
                        .iter()
                        .flat_map(|m| m.sprites.iter().map(|s| &s.sprite)),
                )
                .map(|r| (&r.frame_profile_name, &r.profile_name))
                .collect();
            eprintln!(
                "{}",
                serde_json::json!({"mission":mission,"animation_refs":refs,"soldiers":level.mission.soldiers.iter().map(|s|s.profile_number).collect::<Vec<_>>(),"civilians":level.mission.civilians.iter().map(|s|s.profile_number).collect::<Vec<_>>() })
            );
        }
        let profiles: Vec<_> = part.rhs_files.iter().map(|(rhs,data)| {
            let ps: Vec<_> = data.profiles.iter().map(|(name,info)| {
                let ids:BTreeSet<_> = info.scripts.iter().flat_map(|s| s.frame_ids.iter().copied()).collect();
                let first_frames: Vec<_> = [1, 4, 8, 16, 32].into_iter().map(|limit| {
                    let ids: BTreeSet<_> = info.scripts.iter().flat_map(|s| s.frame_ids.iter().take(limit).copied()).collect();
                    let tiles: usize = part.sprite_bank.iter().flat_map(|b| b.sprites.iter()).filter(|(id,_)|ids.contains(id)).map(|(_,s)|usize::from(s.width)/4*usize::from(s.height)).sum();
                    serde_json::json!({"limit":limit,"frames":ids.len(),"tiles":tiles})
                }).collect();
                let total_tiles: usize = part.sprite_bank.iter().flat_map(|b|b.sprites.iter()).filter(|(id,_)|ids.contains(id)).map(|(_,s)|usize::from(s.width)/4*usize::from(s.height)).sum();
                serde_json::json!({"profile":name,"frames":ids.len(),"scripts":info.scripts.len(),"tiles":total_tiles,"first_frames":first_frames})
            }).collect();
            serde_json::json!({"rhs":rhs,"profiles":ps})
        }).collect();
        let vq: Vec<_> = part.sprite_bank.iter().flat_map(|b| b.vq_chunks.iter()).map(|c|serde_json::json!({"rhs":c.rhs,"base":c.base_rhs,"base2":c.base2_rhs,"frames":c.sprite_ids.len(),"bytes":c.blob.len()})).collect();
        let rle:Vec<_> = part.sprite_bank.iter().flat_map(|b| b.rle_jxl_chunks.iter()).map(|c|serde_json::json!({"rhs":c.rhs,"frames":c.sprite_ids.len(),"bytes":c.jxl_blobs.iter().map(Vec::len).sum::<usize>()})).collect();
        println!(
            "{}",
            serde_json::json!({"file":name,"bytes":bytes.len(),"profiles":profiles,"vq":vq,"rle":rle})
        );
    }
    eprintln!(
        "{}",
        serde_json::json!({"missions":dd.missions,"characters":dd.character_rhs_files,"profiles":dd.profiles.as_ref().map(|p|p.characters.iter().map(|c|(&c.filename,&c.profile_name)).collect::<Vec<_>>())})
    );
    Ok(())
}
