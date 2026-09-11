use robin_engine::audio_durations::AudioDurations;
use robin_engine::engine::LevelAssets;
use robin_engine::profiles::ProfileManager;
use std::path::Path;

const TIMING: &str = r#"{"version":1,"locale":"en-US","samples_ms":{"speech/a.wav":81},"speech_groups":{"65536":["speech/a.wav"]}}"#;

fn core(root: &Path, contents: &str) {
    std::fs::create_dir_all(root.join("Data")).unwrap();
    std::fs::write(root.join("Data/AudioDurations.json"), contents).unwrap();
}

#[test]
fn explicit_core_is_required_valid_and_prepared_only_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("core");
    assert!(robin_parity::prepare_core_audio_timing(&path).is_err());
    std::fs::create_dir(&path).unwrap();
    assert!(robin_parity::prepare_core_audio_timing(&path).is_err());
    core(&path, "invalid json");
    assert!(robin_parity::prepare_core_audio_timing(&path).is_err());
    core(&path, TIMING);
    let admitted = robin_parity::prepare_core_audio_timing(&path).unwrap();
    assert_eq!(admitted.duration_frames("speech/a.wav").unwrap(), 3);
    // Changing the source after admission must not alter deterministic timing.
    core(&path, "invalid replacement");
    let profiles = ProfileManager::new();
    let mut prepared_assets = LevelAssets::new();
    prepared_assets.audio.required_exclamation_ids.insert(65536);
    robin_parity::populate_sound_duration_tables(&mut prepared_assets, &profiles, &admitted)
        .unwrap();
    let mut direct_assets = LevelAssets::new();
    direct_assets.audio.required_exclamation_ids.insert(65536);
    AudioDurations::from_json(TIMING.as_bytes())
        .unwrap()
        .populate(&mut direct_assets.audio, &profiles)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&prepared_assets.audio).unwrap(),
        serde_json::to_value(&direct_assets.audio).unwrap()
    );
}

#[test]
fn relative_core_is_prepared_before_datadir_chdir() {
    let invocation = tempfile::tempdir().unwrap();
    core(&invocation.path().join("chosen-core"), TIMING);
    core(
        &invocation.path().join("licensed-data"),
        &TIMING.replace("81", "999"),
    );
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "prepared_timing_chdir_child", "--nocapture"])
        .env("PARITY_TEST_CORE_CHDIR", "1")
        .current_dir(invocation.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn prepared_timing_chdir_child() {
    if std::env::var_os("PARITY_TEST_CORE_CHDIR").is_none() {
        return;
    }
    let core = std::path::absolute("chosen-core").unwrap();
    let timing = robin_parity::prepare_core_audio_timing(&core).unwrap();
    std::env::set_current_dir("licensed-data").unwrap();
    assert_eq!(timing.duration_frames("speech/a.wav").unwrap(), 3);
    assert_eq!(
        robin_parity::prepare_core_audio_timing(&core)
            .unwrap()
            .duration_frames("speech/a.wav")
            .unwrap(),
        3
    );
}

#[cfg(feature = "client")]
#[test]
fn client_runner_rejects_cpu_core_override_instead_of_ignoring_it() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_original_parity_replay"))
        .args(["--core-datadir", "unused", "missing-trace"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only supported by the CPU runner"));
}

#[cfg(not(feature = "client"))]
#[test]
fn copied_runner_uses_invocation_relative_core_not_build_checkout() {
    let install = tempfile::tempdir().unwrap();
    let binary = install.path().join("copied-runner");
    std::fs::copy(env!("CARGO_BIN_EXE_original_parity_replay"), &binary).unwrap();
    let launch = |args: &[&str]| {
        let output = std::process::Command::new(&binary)
            .current_dir(install.path())
            .env(
                "ROBINHOOD_DATA_DIR",
                install.path().join("different-data-cwd"),
            )
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    // Build checkout still exists, but runtime default is absent: no hidden
    // compile-time fallback may supply its AudioDurations.json.
    assert!(launch(&["missing-trace.jsonl"]).contains("prepare replay core input"));
    core(&install.path().join("chosen-core"), TIMING);
    let error = launch(&["--core-datadir", "chosen-core", "missing-trace.jsonl"]);
    assert!(!error.contains("prepare replay core input"), "{error}");
    assert!(error.contains("missing-trace"), "{error}");
    core(&install.path().join("assets/core-datadir"), TIMING);
    assert!(
        launch(&["--core-datadir", "absent", "missing-trace.jsonl"])
            .contains("prepare replay core input")
    );
    core(&install.path().join("chosen-core"), "corrupt");
    assert!(
        launch(&["--core-datadir", "chosen-core", "missing-trace.jsonl"])
            .contains("invalid Data/AudioDurations.json")
    );
}
