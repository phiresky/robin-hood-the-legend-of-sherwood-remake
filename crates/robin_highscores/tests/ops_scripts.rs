//! Runs the shell test for `ops/deploy.sh` and `ops/rollback.sh` (stubbed
//! systemctl/curl/binaries in a temporary HOME). Skipped when bash is missing.

use std::process::Command;

#[test]
fn deploy_and_rollback_scripts() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/ops/tests/deploy-rollback.sh");
    let output = match Command::new("bash").arg(script).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping ops script test: bash is not installed");
            return;
        }
        Err(error) => panic!("failed to run {script}: {error}"),
    };
    assert!(
        output.status.success(),
        "{script} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
