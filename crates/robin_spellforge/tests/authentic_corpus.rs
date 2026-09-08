use robin_engine::natives::NativeFn;
use robin_engine::spellforge::{
    SpellforgeInvocation, SpellforgeRuntime, SpellforgeStep, SpellforgeTape, SpellforgeTarget,
    hex_hash,
};
use robin_spellforge::{ARCHIVE_BYTE_LIMIT, SpellforgeRuntime51, build_package_from_archives};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("corpus/manifest.json");

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    source_page: String,
    archives: Vec<CorpusArchive>,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
struct CorpusArchive {
    file: String,
    #[serde(default)]
    page_url: Option<String>,
    url: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    name: String,
    mission_archive: String,
    rhm_entry: String,
    #[serde(default)]
    shared_library_archive: Option<String>,
    expected_package_sha256: String,
    expected_source_files: usize,
    expected_source_bytes: usize,
}

fn manifest() -> CorpusManifest {
    serde_json::from_str(MANIFEST).expect("authentic corpus manifest must be valid JSON")
}

#[test]
fn authentic_corpus_manifest_is_pinned_and_well_formed() {
    let manifest = manifest();
    assert_eq!(manifest.source_page, "https://rhmods.com/missions/");
    assert!(!manifest.archives.is_empty());
    assert!(!manifest.cases.is_empty());

    let mut archive_names = BTreeSet::new();
    for archive in &manifest.archives {
        assert!(archive_names.insert(archive.file.as_str()));
        assert!(archive.url.starts_with("https://rhmods.com/"));
        if let Some(page_url) = &archive.page_url {
            assert!(page_url.starts_with("https://rhmods.com/missions/"));
        }
        decode_hash(&archive.sha256);
    }

    let mut case_names = BTreeSet::new();
    for case in &manifest.cases {
        assert!(case_names.insert(case.name.as_str()));
        assert!(archive_names.contains(case.mission_archive.as_str()));
        assert!(case.rhm_entry.to_ascii_lowercase().ends_with(".rhm"));
        if let Some(shared) = &case.shared_library_archive {
            assert!(archive_names.contains(shared.as_str()));
        }
        decode_hash(&case.expected_package_sha256);
        assert!(case.expected_source_files > 0);
        assert!(case.expected_source_bytes > 0);
    }
}

#[test]
#[ignore = "requires SHA-pinned upstream mission ZIPs; see tests/corpus/manifest.json"]
fn authentic_luajit_missions_pass_the_production_loader_and_vm() {
    let root = std::env::var_os("SPELLFORGE_CORPUS_DIR")
        .map(PathBuf::from)
        .expect("set SPELLFORGE_CORPUS_DIR to the directory containing the pinned corpus ZIPs");
    let manifest = manifest();
    let mut archives = BTreeMap::new();
    for archive in &manifest.archives {
        let bytes = read_bounded(&root.join(&archive.file));
        let actual_sha256: [u8; 32] = Sha256::digest(&bytes).into();
        assert_eq!(
            actual_sha256,
            decode_hash(&archive.sha256),
            "upstream corpus archive {} does not match its pinned digest",
            archive.file
        );
        archives.insert(archive.file.as_str(), bytes);
    }

    let mut identities_match = true;
    for case in &manifest.cases {
        let mission = archives
            .get(case.mission_archive.as_str())
            .expect("manifest references a missing mission archive");
        let shared = case.shared_library_archive.as_ref().map(|name| {
            archives
                .get(name.as_str())
                .expect("manifest references a missing shared library archive")
                .as_slice()
        });
        let basename = case
            .rhm_entry
            .rsplit('/')
            .next()
            .and_then(|leaf| leaf.rsplit_once('.').map(|(basename, _)| basename))
            .expect("manifest .rhm entry must have a basename");
        let package = build_package_from_archives(mission, &case.rhm_entry, basename, shared)
            .unwrap_or_else(|error| panic!("{} package admission failed: {error}", case.name));
        let actual_identity = hex_hash(&package.sha256);
        if actual_identity != case.expected_package_sha256 {
            eprintln!(
                "{} package identity changed: {}",
                case.name, actual_identity
            );
            identities_match = false;
        }
        assert_eq!(
            package.files.len(),
            case.expected_source_files,
            "{}",
            case.name
        );
        assert_eq!(
            package.files.values().map(Vec::len).sum::<usize>(),
            case.expected_source_bytes,
            "{}",
            case.name
        );
        let runtime = SpellforgeRuntime51::new(package)
            .unwrap_or_else(|error| panic!("{} guest bootstrap failed: {error}", case.name));
        if case.name == "first_lincoln" {
            assert_first_lincoln_published_handlers(&runtime);
        }
    }
    assert!(
        identities_match,
        "package identity changed; review executable ABI and all corpus identities above"
    );
}

fn assert_first_lincoln_published_handlers(runtime: &SpellforgeRuntime51) {
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();

    let combat_calls = drive_event(
        runtime,
        &mut tape,
        SpellforgeInvocation {
            target: SpellforgeTarget::Global,
            event: "Combact_soldier_1".to_owned(),
            args: vec![],
            script_this: 0,
            current_scroll: 0,
        },
    );
    assert!(
        combat_calls.iter().any(|(native, arguments)| {
            *native == NativeFn::RecordPlayAnim as u32 && arguments.get(1) == Some(&172)
        }),
        "the published sibling enums.lua must supply repelArrow=172"
    );

    let alert_calls = drive_event(
        runtime,
        &mut tape,
        SpellforgeInvocation {
            target: SpellforgeTarget::Actor {
                class: "Script_Reinforce_off".to_owned(),
                handle: 0,
            },
            event: "ActionChange".to_owned(),
            args: vec![140, 0],
            script_this: 0,
            current_scroll: 0,
        },
    );
    assert_eq!(
        alert_calls
            .iter()
            .filter(|(native, arguments)| {
                *native == NativeFn::SetAlwaysAttentive as u32 && arguments.get(1) == Some(&1)
            })
            .count(),
        9,
        "published numeric boolean arguments must canonicalize to word 1"
    );
}

fn drive_event(
    runtime: &SpellforgeRuntime51,
    tape: &mut SpellforgeTape,
    invocation: SpellforgeInvocation,
) -> Vec<(u32, Vec<i32>)> {
    let mut calls = Vec::new();
    let mut step = runtime.begin(invocation, tape).unwrap();
    for _ in 0..256 {
        match step {
            SpellforgeStep::Native {
                activation,
                native_index,
                arguments,
            } => {
                calls.push((native_index, arguments));
                step = runtime.resume(activation, 0, tape).unwrap();
            }
            SpellforgeStep::Complete { activation, result } => {
                runtime.commit(activation, result, tape).unwrap();
                return calls;
            }
        }
    }
    panic!("authentic mission handler exceeded 256 native calls")
}

fn read_bounded(path: &Path) -> Vec<u8> {
    let declared = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("cannot inspect {}: {error}", path.display()))
        .len();
    assert!(
        declared <= ARCHIVE_BYTE_LIMIT as u64,
        "{} exceeds archive limit",
        path.display()
    );
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    assert!(
        bytes.len() <= ARCHIVE_BYTE_LIMIT,
        "{} grew beyond archive limit while reading",
        path.display()
    );
    bytes
}

fn decode_hash(encoded: &str) -> [u8; 32] {
    assert_eq!(encoded.len(), 64, "SHA-256 must contain 64 hex digits");
    let mut decoded = [0; 32];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_digit(pair[0]);
        let low = hex_digit(pair[1]);
        decoded[index] = (high << 4) | low;
    }
    decoded
}

fn hex_digit(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => panic!("SHA-256 must use lowercase hexadecimal"),
    }
}
