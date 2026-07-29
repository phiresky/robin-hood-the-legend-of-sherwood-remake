//! One-time migration for schema-10 traces recorded before the campaign/RNG
//! boundary was moved ahead of `InitializeFromMission`.
//!
//! The old recorder could snapshot campaign descriptions and registered names
//! after PRIS construction while also retaining the draws which produced
//! them. This utility only rewrites an artifact when it can prove that:
//! - the affected non-VIP PRIS descriptions are an unreferenced trailing
//!   campaign suffix, and
//! - replaying the prefix through Original's exact name generator regenerates
//!   every removed name byte-for-byte.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

use robin_assets::resource_manager::ResourceManager;
use robin_engine::profiles::ProfileManager;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .expect("usage: normalize_schema10_campaign_boundary INPUT OUTPUT"),
    );
    let output = PathBuf::from(args.next().expect("missing OUTPUT"));
    assert!(args.next().is_none(), "unexpected extra argument");
    let input = input.canonicalize().expect("canonicalize input trace");
    let output = if output.is_absolute() {
        output
    } else {
        std::env::current_dir()
            .expect("current directory")
            .join(output)
    };

    if let Ok(datadir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(datadir).expect("chdir to ROBINHOOD_DATA_DIR");
    }
    robin_rs::main_entry::register_language_data_paths_for_tool();

    let source = std::fs::File::open(&input).expect("open input trace");
    let mut reader = BufReader::new(source);
    let mut header_line = String::new();
    let mut prefix_line = String::new();
    assert!(reader.read_line(&mut header_line).expect("read header") != 0);
    assert!(reader.read_line(&mut prefix_line).expect("read rng_prefix") != 0);
    let mut header: serde_json::Value =
        serde_json::from_str(&header_line).expect("parse trace header");
    let prefix: serde_json::Value = serde_json::from_str(&prefix_line).expect("parse rng_prefix");
    assert_eq!(header["schema"], 10);
    assert_eq!(header["start_state"], "mission_start");
    assert_eq!(prefix["type"], "rng_prefix");

    let mut profiles = ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open(
        "Data/Configuration/profile.cpf",
        robin_engine::sbfile::SB_FILE_READ,
    )
    .expect("open profile.cpf");
    profiles
        .load_all_legacy_cpf(&mut cpf)
        .expect("parse profile.cpf");

    let mission = header["mission"].as_str().expect("header mission");
    let proto = header["proto_level"].as_str().expect("header proto_level");
    let loaded = robin_engine::level_data::load_level(
        mission,
        proto,
        "Data/Levels",
        &|profile_id| {
            profiles
                .get_civilian(profile_id)
                .is_some_and(|p| p.civilian_type == robin_engine::profiles::CivilianType::Beggar)
        },
        &mut |_| {},
    )
    .expect("load mission data");

    let non_vip_pris: Vec<u32> = loaded
        .mission
        .pcs_to_rescue
        .iter()
        .filter_map(|raw| {
            let profile = profiles
                .get_character(raw.profile_index)
                .expect("PRIS profile is present");
            (!profile.vip).then_some(raw.profile_index)
        })
        .collect();
    assert!(
        !non_vip_pris.is_empty(),
        "mission has no non-VIP PRIS descriptions to normalize"
    );

    let campaign = header["campaign"].as_object_mut().expect("campaign object");
    let characters = campaign["characters"]
        .as_array()
        .expect("campaign characters");
    let pris_profiles = non_vip_pris.iter().copied().collect::<BTreeSet<_>>();
    let first_suffix = characters
        .iter()
        .position(|desc| {
            desc["profile_index"]
                .as_u64()
                .is_some_and(|idx| pris_profiles.contains(&(idx as u32)))
        })
        .expect("campaign has no post-construction PRIS descriptions");
    assert!(
        characters[first_suffix..].iter().all(|desc| {
            desc["profile_index"]
                .as_u64()
                .is_some_and(|idx| pris_profiles.contains(&(idx as u32)))
        }),
        "PRIS descriptions are not a trailing campaign suffix; refusing lossy migration"
    );

    let removed_indices = (first_suffix..characters.len()).collect::<BTreeSet<_>>();
    for key in ["gang_indices", "reservist_indices", "mission_team_indices"] {
        for value in campaign[key].as_array().into_iter().flatten() {
            let index = value.as_u64().expect("character reference index") as usize;
            assert!(
                !removed_indices.contains(&index),
                "{key} references post-construction PRIS description {index}"
            );
        }
    }
    for sector in campaign["production_sectors"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for occupant in sector["occupants"].as_array().into_iter().flatten() {
            let index = occupant["character_index"]
                .as_u64()
                .expect("production occupant character index") as usize;
            assert!(
                !removed_indices.contains(&index),
                "production occupant references post-construction PRIS description {index}"
            );
        }
    }

    let removed = campaign["characters"]
        .as_array_mut()
        .expect("campaign characters")
        .split_off(first_suffix);
    let mut removed_names = BTreeMap::<u32, VecDeque<String>>::new();
    for desc in removed {
        let profile = desc["profile_index"].as_u64().expect("profile_index") as u32;
        let name = desc["status"]["name"]
            .as_str()
            .expect("PRIS status name")
            .to_owned();
        removed_names.entry(profile).or_default().push_back(name);
    }
    assert_eq!(
        removed_names.values().map(VecDeque::len).sum::<usize>(),
        non_vip_pris.len(),
        "trailing descriptions do not correspond one-for-one with authored non-VIP PRIS entries"
    );

    let names = campaign["peasant_names"]
        .as_array_mut()
        .expect("campaign peasant_names");
    let original_registry = names
        .iter()
        .map(|name| name.as_str().expect("peasant name").to_owned())
        .collect::<Vec<_>>();
    for expected in removed_names.values().flatten() {
        let position = names
            .iter()
            .position(|name| name.as_str() == Some(expected))
            .unwrap_or_else(|| panic!("removed PRIS name {expected:?} is absent from registry"));
        names.remove(position);
    }

    let mut text = ResourceManager::new();
    text.attach_resource_file("Data/Text/Level.res")
        .expect("load Level.res");
    let (firstnames, surnames) = robin_rs::game_session::load_peasant_name_pool(&mut text);
    assert_eq!((firstnames.len(), surnames.len()), (22, 22));
    let draws = prefix["draws"]["values"]
        .as_array()
        .expect("rng_prefix draws.values");
    let mut cursor = 0usize;
    let mut regenerated_registry = names
        .iter()
        .map(|name| name.as_str().expect("peasant name").to_owned())
        .collect::<Vec<_>>();
    for profile in non_vip_pris {
        let expected = removed_names
            .get_mut(&profile)
            .and_then(VecDeque::pop_front)
            .expect("removed name for authored PRIS profile");
        let mut generated = None;
        for _ in 0..10 {
            let first_raw = draws[cursor].as_u64().expect("firstname RNG draw");
            let surname_raw = draws[cursor + 1].as_u64().expect("surname RNG draw");
            cursor += 2;
            let candidate = format!(
                "{} {}",
                firstnames[first_raw as usize % 22],
                surnames[surname_raw as usize % 22]
            );
            if !regenerated_registry.contains(&candidate) {
                regenerated_registry.push(candidate.clone());
                generated = Some(candidate);
                break;
            }
        }
        assert_eq!(
            generated.as_deref(),
            Some(expected.as_str()),
            "prefix does not regenerate removed PRIS name for profile {profile}"
        );
    }
    assert_eq!(
        regenerated_registry, original_registry,
        "regenerated peasant registry differs from recorded post-construction registry"
    );

    assert!(
        !output.exists(),
        "refusing to overwrite existing output {}",
        output.display()
    );
    let temp_output = output.with_extension(format!(
        "{}.tmp-{}",
        output
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("jsonl"),
        std::process::id()
    ));
    let target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_output)
        .expect("create temporary normalized trace");
    let mut writer = BufWriter::new(target);
    serde_json::to_writer(&mut writer, &header).expect("write normalized header");
    writer.write_all(b"\n").expect("finish header line");
    writer
        .write_all(prefix_line.as_bytes())
        .expect("copy rng_prefix");
    // The same recorder generation classified `RHCLASSID_OBJECT_BONUS_NET`
    // (0x4007) as a live net. Normalize that independently verifiable class
    // identity while rewriting the artifact; future Original traces emit it
    // through the ordinary `IsBonus()` branch.
    let mut record_line = String::new();
    loop {
        record_line.clear();
        if reader
            .read_line(&mut record_line)
            .expect("read trace record")
            == 0
        {
            break;
        }
        let mut record: serde_json::Value =
            serde_json::from_str(&record_line).expect("parse trace record");
        if let Some(elements) = record["elements"].as_array_mut() {
            for element in elements {
                if element.is_null() {
                    continue;
                }
                if element["class_id"].as_u64() == Some(0x4007) {
                    assert_eq!(element["kind"], "net");
                    assert_eq!(element["entity_id"]["kind"], "net");
                    element["kind"] = serde_json::Value::String("bonus".to_owned());
                    element["entity_id"]["kind"] = serde_json::Value::String("bonus".to_owned());
                }
            }
        }
        serde_json::to_writer(&mut writer, &record).expect("write normalized trace record");
        writer.write_all(b"\n").expect("finish trace record");
    }
    writer.flush().expect("flush normalized trace");
    drop(writer);
    std::fs::rename(&temp_output, &output).expect("publish normalized trace atomically");
    eprintln!(
        "normalized {} PRIS descriptions using {} verified RNG draws: {}",
        removed_indices.len(),
        cursor,
        output.display()
    );
}
