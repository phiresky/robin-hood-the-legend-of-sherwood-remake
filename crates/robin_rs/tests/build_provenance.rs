//! Exercise build-script provenance in subprocesses so Cargo/environment state
//! is never changed in the test runner or concurrent tests.
#[path = "../../../build-support/robin_build.rs"]
mod build_script;

#[test]
fn provenance_probe() {
    if std::env::var_os("ROBIN_TEST_PROVENANCE_PROBE").is_some() {
        build_script::main();
    }
}

fn probe(manifest: &std::path::Path, strict: bool) -> std::process::Output {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "provenance_probe", "--nocapture"])
        .env("ROBIN_TEST_PROVENANCE_PROBE", "1")
        .env("CARGO_MANIFEST_DIR", manifest)
        .env("TARGET", "test-target")
        .env("PROFILE", "debug")
        .env("GIT_DIR", "/not-the-build-checkout")
        .env_remove("ROBIN_REQUIRE_BUILD_IDENTITY");
    if strict {
        command.env("ROBIN_REQUIRE_BUILD_IDENTITY", "1");
    }
    command.output().unwrap()
}

#[test]
fn source_archives_warn_for_development_and_fail_for_release() {
    let archive = tempfile::tempdir().unwrap();
    let manifest = archive.path().join("crates/client");
    std::fs::create_dir_all(&manifest).unwrap();
    std::fs::write(archive.path().join("Cargo.lock"), "lock bytes").unwrap();
    let developer = probe(&manifest, false);
    assert!(developer.status.success());
    let stdout = String::from_utf8(developer.stdout).unwrap();
    assert!(stdout.contains("cargo:warning=Incomplete developer build provenance"));
    assert!(stdout.contains("cargo:rustc-env=ROBIN_GIT_COMMIT=unknown"));
    assert!(!probe(&manifest, true).status.success());
}

#[test]
fn checkout_or_linked_worktree_emits_current_git_identity_and_watches_head() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = probe(manifest, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let git = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(manifest)
        .output()
        .unwrap();
    assert!(git.status.success());
    assert!(stdout.contains(&format!(
        "cargo:rustc-env=ROBIN_GIT_COMMIT={}",
        String::from_utf8_lossy(&git.stdout).trim()
    )));
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("cargo:rerun-if-changed=") && line.ends_with("/HEAD"))
    );
}
