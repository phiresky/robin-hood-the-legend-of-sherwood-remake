//! Dumps a complete level (proto .rhp + mission .rhm) to JSON.
//!
//! Example:
//!   dump_level --level-dir datadirs/demo/Data/Levels --proto leicester \
//!              --mission Dem_Lei_MP --output level.json

use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Dump a Robin Hood level (proto + optional mission) to JSON")]
struct Args {
    /// Directory containing the proto (.rhp) and mission (.rhm) files.
    #[arg(long)]
    level_dir: String,

    /// Proto-level base name (e.g. `leicester` for `leicester.rhp`).
    #[arg(long)]
    proto: String,

    /// Mission base name (e.g. `Dem_Lei_MP` for `Dem_Lei_MP.rhm`).
    /// Omit to dump only the proto.
    #[arg(long)]
    mission: Option<String>,

    /// Path to `profile.cpf`.  Used to resolve beggar civilian profiles
    /// so the mission parser reads the correct civilian payload.  If
    /// omitted, defaults to `<level-dir>/../Configuration/profile.cpf`
    /// (the canonical layout under `Data/`), and if that's missing the
    /// loader falls back to the `is_beggar=false` stub.
    #[arg(long)]
    profile_cpf: Option<String>,

    /// Output JSON path. Writes to stdout if omitted.
    #[arg(long)]
    output: Option<String>,
}

fn main() {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    use robin_engine::level_data::{ChunkReader, LevelFormat, load_proto_level};
    use robin_engine::profiles::{CivilianType, ProfileManager};
    use robin_engine::sbfile::{SB_FILE_READ, SbFile};
    use std::collections::BTreeSet;

    // Load the proto file (geometry, motion areas, buildings, etc.)
    let proto_path = format!("{}/{}.rhp", args.level_dir, args.proto);
    let proto_file = SbFile::open(&proto_path, SB_FILE_READ).unwrap_or_else(|e| {
        eprintln!("Failed to open {proto_path}: error {e}");
        std::process::exit(1);
    });
    let mut proto_reader = ChunkReader::new(proto_file);
    let format = {
        let tag = proto_reader.peek_next_chunk().unwrap();
        LevelFormat::detect(&tag).unwrap()
    };
    let proto = load_proto_level(&mut proto_reader, format).unwrap_or_else(|e| {
        eprintln!("Failed to load proto: {e:?}");
        std::process::exit(1);
    });

    // Build the `is_beggar` predicate from the profile.cpf (if
    // reachable). The mission's civilian payload has an extra scroll-set
    // block for beggar profiles, so missions with beggars would mis-parse
    // under a naive `|_| false` stub.
    let default_cpf = format!("{}/../Configuration/profile.cpf", args.level_dir);
    let cpf_path = args.profile_cpf.clone().unwrap_or(default_cpf);
    let beggar_indices: BTreeSet<u32> = match SbFile::open(&cpf_path, SB_FILE_READ) {
        Ok(mut file) => {
            let mut mgr = ProfileManager::new();
            match mgr.load_all_legacy_cpf(&mut file) {
                Ok(()) => {
                    eprintln!("Loaded profile.cpf from {cpf_path}");
                    mgr.civilians
                        .iter()
                        .enumerate()
                        .filter_map(|(i, c)| {
                            (c.civilian_type == CivilianType::Beggar).then_some(i as u32)
                        })
                        .collect()
                }
                Err(e) => {
                    eprintln!(
                        "Warning: failed to parse {cpf_path} (error {e}); using is_beggar=false stub"
                    );
                    BTreeSet::new()
                }
            }
        }
        Err(e) => {
            eprintln!("Warning: could not open {cpf_path} (error {e}); using is_beggar=false stub");
            BTreeSet::new()
        }
    };
    let is_beggar = |id: u32| beggar_indices.contains(&id);

    // Optionally load mission too.
    let mission = if let Some(ref name) = args.mission {
        let mission_path = format!("{}/{}.rhm", args.level_dir, name);
        let mission_file = SbFile::open(&mission_path, SB_FILE_READ).unwrap_or_else(|e| {
            eprintln!("Failed to open {mission_path}: error {e}");
            std::process::exit(1);
        });
        let mut mission_reader = ChunkReader::new(mission_file);
        match robin_engine::level_data::load_mission(&mut mission_reader, format, &is_beggar) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("Warning: mission load failed ({e:?}), dumping proto only");
                None
            }
        }
    } else {
        None
    };

    eprintln!(
        "Loaded proto: {} patches, {} animations, {} lifts, {} buildings",
        proto.patches.len(),
        proto.animations.len(),
        proto.lifts.len(),
        proto.buildings.len(),
    );
    if let Some(ref m) = mission {
        eprintln!(
            "Loaded mission: {} soldiers, {} civilians",
            m.soldiers.len(),
            m.civilians.len(),
        );
    }
    if let Some(ref motion) = proto.motion_data {
        eprintln!(
            "Motion: {} layers, {} graph bytes",
            motion.layers.len(),
            motion.graph_bytes.len(),
        );
        for (i, layer) in motion.layers.iter().enumerate() {
            eprintln!("  Layer {}: {} areas", i, layer.len());
            for (j, area) in layer.iter().enumerate() {
                eprintln!(
                    "    Area {}: {} polygon pts, {} skeleton segs, {} obstacles",
                    j,
                    area.polygon.points.len(),
                    area.skeleton_segments.len(),
                    area.obstacles.len(),
                );
            }
        }
    }

    #[derive(serde::Serialize)]
    struct Dump {
        proto: robin_engine::level_data::LoadedProtoLevel,
        mission: Option<robin_engine::level_data::LoadedMission>,
    }
    let dump = Dump { proto, mission };
    let json = serde_json::to_string_pretty(&dump).unwrap();

    if let Some(ref out) = args.output {
        std::fs::write(out, &json).unwrap_or_else(|e| {
            eprintln!("Failed to write {out}: {e}");
            std::process::exit(1);
        });
        eprintln!("Written to {out}");
    } else {
        std::io::Write::write_all(&mut std::io::stdout(), json.as_bytes())
            .expect("write to stdout");
    }
}
