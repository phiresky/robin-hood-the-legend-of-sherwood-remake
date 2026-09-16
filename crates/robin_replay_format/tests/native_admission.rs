#![cfg(all(feature = "native-admission", unix))]

use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayData, ReplayFile, ReplayHeader};
use robin_replay_format::native_admission::{self, AdmissionError, HELPER_NAME};
use robin_replay_format::{ENGINE_VERSION_HASH, LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS};

// Serialize every test that copies or spawns helpers, not just the copier.
// Admission's pre_exec containment requires fork: a concurrent fork could
// inherit the copier's writable destination descriptor until the child execs.
// Closing the parent's descriptor alone would then leave the installed helper
// temporarily non-executable (ETXTBSY). This prevents that race; it does not
// establish it as the cause of any particular transient failure.
static HELPER_FIXTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn replay() -> ReplayFile {
    ReplayFile {
        header: ReplayHeader {
            mission_id: "Dem_Lei_MP".into(),
            mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "Dem_Lei_MP",
                "Leicester",
                "Leicester",
            )
            .unwrap(),
            rng_seed: 7,
            sim_config: Default::default(),
            spellforge_package: None,
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 0,
            rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
            campaign: bitcode::encode(&robin_engine::campaign::Campaign::default()),
        },
        frames: Default::default(),
        hashes: Default::default(),
        save_markers: Default::default(),
        load_backs: Default::default(),
    }
}

fn compact(file: ReplayFile) -> Vec<u8> {
    let data: ReplayData = file.try_into().unwrap();
    robin_replay_format::encode_compact(&data, ENGINE_VERSION_HASH).unwrap()
}

#[test]
fn cargo_and_installed_helpers_accept_exact_current_artifacts() {
    let _fixture_guard = HELPER_FIXTURE_LOCK
        .lock()
        .expect("helper fixture lock poisoned");
    let input = compact(replay());
    native_admission::validate_in_native_child(&input).unwrap();
    let install = tempfile::tempdir().unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_robin-replay-admission"),
        install.path().join(HELPER_NAME),
    )
    .unwrap();
    native_admission::validate_next_to(&input, &install.path().join("renamed-game")).unwrap();
}

#[test]
fn hostile_transport_and_oversized_input_fail_closed() {
    let _fixture_guard = HELPER_FIXTURE_LOCK
        .lock()
        .expect("helper fixture lock poisoned");
    assert!(matches!(
        native_admission::validate_in_native_child(&vec![
            b'x';
            LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS
                .max_input_bytes
                + 1
        ]),
        Err(AdmissionError::Compact(_))
    ));
    assert!(
        native_admission::validate_in_native_child(
            format!("rhrec-{ENGINE_VERSION_HASH}-AA").as_bytes()
        )
        .is_err()
    );
    let mut malformed_prefix = compact(replay());
    malformed_prefix[6] = b'z';
    assert!(native_admission::validate_in_native_child(&malformed_prefix).is_err());
    // The recorded source hash is provenance, not a compatibility gate.
    let mut other_commit = compact(replay());
    other_commit[6..18].copy_from_slice(b"0123456789ab");
    native_admission::validate_in_native_child(&other_commit).unwrap();
}

#[test]
fn helper_rejects_wire_valid_packages_for_a_different_spellforge_vm() {
    let _fixture_guard = HELPER_FIXTURE_LOCK
        .lock()
        .expect("helper fixture lock poisoned");
    let mut file = replay();
    use robin_engine::mission_assets::*;
    file.header.mission_assets = MissionAssetDescriptor::archive(
        "Dem_Lei_MP",
        "Leicester",
        "Leicester",
        ArchiveMissionAssets {
            mission_archive: ArchiveIdentity {
                sha256: [1; 32],
                bytes: 1,
            },
            selected_rhm_entry: "Dem_Lei_MP.rhm".into(),
            shared_archive: None,
            installed: Some(InstalledArchiveLocator {
                root: InstalledModsRoot::ConfiguredMods,
                mission_relative_path: "custom/v1.zip".into(),
                shared_relative_path: None,
            }),
            distributed_cache: None,
        },
    )
    .unwrap();
    let mut package = robin_engine::spellforge::SpellforgePackage {
        contract_version: robin_engine::spellforge::SPELLFORGE_CONTRACT_VERSION,
        vm_abi: format!(
            "{}{}",
            robin_engine::spellforge::SPELLFORGE_VM_ABI_SCHEME,
            "1".repeat(64)
        ),
        script_mode: robin_engine::spellforge::SpellforgeScriptMode::Replace,
        entrypoint: "dem_lei_mp.lua".into(),
        files: [("dem_lei_mp.lua".into(), b"return {}".to_vec())].into(),
        sha256: [0; 32],
    };
    package.sha256 = package.computed_sha256();
    package.validate_wire().unwrap();
    file.header.spellforge_package = Some(package);
    let error = native_admission::validate_in_native_child(&compact(file.clone())).unwrap_err();
    assert!(
        matches!(error, AdmissionError::AdmissionRejected(ref message)
        if message.contains("Spellforge")),
        "{error}"
    );
    let package = file.header.spellforge_package.as_mut().unwrap();
    package.vm_abi = robin_spellforge::spellforge_vm_abi().into();
    package.sha256 = package.computed_sha256();
    native_admission::validate_in_native_child(&compact(file)).unwrap();
}

#[test]
fn an_unrelated_executable_is_not_an_admission_helper() {
    let _fixture_guard = HELPER_FIXTURE_LOCK
        .lock()
        .expect("helper fixture lock poisoned");
    let install = tempfile::tempdir().unwrap();
    std::fs::copy("/bin/true", install.path().join(HELPER_NAME)).unwrap();
    assert!(
        native_admission::validate_next_to(&compact(replay()), &install.path().join("robin"))
            .is_err()
    );
}
