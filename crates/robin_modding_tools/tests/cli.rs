use std::process::Command;

const TOOLS: &[&str] = &[
    env!("CARGO_BIN_EXE_cpf_to_json"),
    env!("CARGO_BIN_EXE_disasm_scb"),
    env!("CARGO_BIN_EXE_dump_res"),
    env!("CARGO_BIN_EXE_encode_mod_sprites"),
];

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
