use std::process::Command;

const TOOLS: &[&str] = &[
    env!("CARGO_BIN_EXE_cpf_to_json"),
    env!("CARGO_BIN_EXE_disasm_scb"),
    env!("CARGO_BIN_EXE_dump_res"),
    env!("CARGO_BIN_EXE_encode_mod_sprites"),
];

#[test]
fn cpf_json_export_applies_ordered_patches_and_round_trips_the_output() {
    use robin_engine::profiles::{ProfileManager, SoldierProfile};
    use serde_json::json;
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("profile.cpf.json");
    let profiles = ProfileManager {
        soldiers: vec![SoldierProfile {
            filename: "Guard".into(),
            life_point: 10,
            ..Default::default()
        }],
        ..Default::default()
    };
    let original = robin_engine::content_patch::profile_document(&profiles).unwrap();
    std::fs::write(&input, serde_json::to_vec(&original).unwrap()).unwrap();
    let first = temp.path().join("first.json");
    let second = temp.path().join("second.json");
    std::fs::write(
        &first,
        json!([
            {"op":"replace", "path":"/soldiers/Guard/life_point", "value":17}
        ])
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &second,
        json!([
            {"op":"test", "path":"/soldiers/Guard/life_point", "value":17},
            {"op":"replace", "path":"/soldiers/Guard/life_point", "value":23}
        ])
        .to_string(),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_cpf_to_json"))
        .arg("--patch")
        .arg(&first)
        .arg("--patch")
        .arg(&second)
        .arg(&input)
        .env("RUST_LOG", "info")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let patched: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(patched["soldiers"]["Guard"]["life_point"], 23);
    assert_eq!(patched["soldier_order"], original["soldier_order"]);
    let patched_file = temp.path().join("patched.json");
    std::fs::write(&patched_file, &result.stdout).unwrap();
    let validated_file = temp.path().join("validated.json");
    let result = Command::new(env!("CARGO_BIN_EXE_cpf_to_json"))
        .arg(&patched_file)
        .arg(&validated_file)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert_eq!(
        std::fs::read(validated_file).unwrap(),
        std::fs::read(patched_file).unwrap()
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(input).unwrap()).unwrap(),
        original
    );
}

#[test]
fn tools_expose_help_and_reject_unknown_flags_without_touching_data() {
    for tool in TOOLS {
        let help = Command::new(tool).arg("--help").output().unwrap();
        assert!(
            help.status.success(),
            "{tool}: {}",
            String::from_utf8_lossy(&help.stderr)
        );
        assert!(
            String::from_utf8_lossy(&help.stdout).contains("Usage:"),
            "{tool}"
        );
        let invalid = Command::new(tool).arg("--unknown-option").output().unwrap();
        assert_eq!(invalid.status.code(), Some(2), "{tool}");
        assert!(invalid.stdout.is_empty(), "{tool}");
    }
}

#[test]
fn failed_json_exports_keep_diagnostics_out_of_stdout() {
    let temp = tempfile::tempdir().unwrap();
    for tool in [
        env!("CARGO_BIN_EXE_cpf_to_json"),
        env!("CARGO_BIN_EXE_dump_res"),
    ] {
        let output = Command::new(tool)
            .arg(temp.path().join("missing-input"))
            .env("RUST_LOG", "info")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}
