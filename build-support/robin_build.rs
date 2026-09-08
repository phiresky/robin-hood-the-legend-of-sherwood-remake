// Build script: emit `ROBIN_GIT_HASH` for replay-format tagging.
//
// Shaders are now consumed directly as WGSL by wgpu at runtime — no
// offline compilation step needed.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

pub fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../build-support/robin_build.rs");
    println!("cargo:rerun-if-env-changed=ROBIN_PACKAGE_VERSION");
    println!("cargo:rerun-if-env-changed=ROBIN_REQUIRE_BUILD_IDENTITY");
    emit_git_hash();
    emit_cargo_lock_hash();
    emit_build_identity();
}

fn missing_identity(reason: &str) -> String {
    if std::env::var_os("ROBIN_REQUIRE_BUILD_IDENTITY").is_some() {
        panic!("release build requires complete provenance: {reason}");
    }
    println!(
        "cargo:warning=Incomplete developer build provenance: {reason}; source archives cannot identify an official build"
    );
    "unknown".to_owned()
}

/// Emit `ROBIN_GIT_HASH` as a `cargo:rustc-env` so source code can
/// reference it via `env!("ROBIN_GIT_HASH")`. Used by the replay format
/// to tag recordings with the engine version they were produced on.
fn emit_git_hash() {
    // Read this at build-script execution time. Cargo can legitimately reuse
    // one compiled build-script executable across worktrees that share a
    // target directory; compile-time `env!` would permanently bake the first
    // worktree's path into that executable and emit the wrong compatibility
    // hash for every later worktree.
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .expect("Cargo must provide CARGO_MANIFEST_DIR to build scripts"),
    );
    let mut search = manifest_dir.as_path();
    let git_meta = loop {
        let candidate = search.join(".git");
        if candidate.exists() {
            break Some(candidate);
        }
        match search.parent() {
            Some(p) => search = p,
            None => break None,
        }
    };
    let full_hash = if let Some(git_meta) = git_meta {
        let actual_git = if git_meta.is_file() {
            println!("cargo:rerun-if-changed={}", git_meta.display());
            match std::fs::read_to_string(&git_meta) {
                Ok(s) => s
                    .lines()
                    .next()
                    .and_then(|l| l.strip_prefix("gitdir:").map(str::trim))
                    .map(|p| {
                        let pb = PathBuf::from(p);
                        if pb.is_absolute() {
                            pb
                        } else {
                            git_meta.parent().unwrap_or(Path::new(".")).join(pb)
                        }
                    })
                    .unwrap_or_else(|| git_meta.clone()),
                Err(_) => git_meta.clone(),
            }
        } else {
            git_meta.clone()
        };
        let head = actual_git.join("HEAD");
        if head.exists() {
            println!("cargo:rerun-if-changed={}", head.display());
            let commondir_file = actual_git.join("commondir");
            let common_root = if commondir_file.exists() {
                println!("cargo:rerun-if-changed={}", commondir_file.display());
                let rel = std::fs::read_to_string(&commondir_file)
                    .expect("cannot read required Git worktree commondir metadata");
                let rel = rel.trim();
                let pb = PathBuf::from(rel);
                if pb.is_absolute() {
                    pb
                } else {
                    actual_git.join(pb)
                }
            } else {
                actual_git.clone()
            };
            if let Ok(head_txt) = std::fs::read_to_string(&head)
                && let Some(refname) = head_txt
                    .lines()
                    .next()
                    .and_then(|l| l.strip_prefix("ref:").map(str::trim))
            {
                let ref_path = common_root.join(refname);
                println!("cargo:rerun-if-changed={}", ref_path.display());
            }
            let packed = common_root.join("packed-refs");
            if packed.exists() {
                println!("cargo:rerun-if-changed={}", packed.display());
            }
        }
        match std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&manifest_dir)
            // Identity belongs to this checkout even when the parent process
            // was launched by a Git hook or with another repository selected.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_NAMESPACE")
            .output()
        {
            Ok(out) if out.status.success() => {
                let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if matches!(hash.len(), 40 | 64)
                    && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    hash
                } else {
                    missing_identity("git returned an invalid commit object ID")
                }
            }
            Ok(out) => missing_identity(&format!(
                "git rev-parse HEAD failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )),
            Err(error) => missing_identity(&format!("cannot execute git: {error}")),
        }
    } else {
        missing_identity("no Git metadata found (source archive)")
    };
    let short_hash = full_hash.get(..12).unwrap_or(&full_hash);
    println!("cargo:rustc-env=ROBIN_GIT_HASH={short_hash}");
    println!("cargo:rustc-env=ROBIN_GIT_COMMIT={full_hash}");
}

fn emit_cargo_lock_hash() {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .expect("Cargo must provide CARGO_MANIFEST_DIR to build scripts"),
    );
    let lock_path = manifest_dir.join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let hash = std::fs::read(&lock_path)
        .map(|bytes| {
            Sha256::digest(bytes)
                .iter()
                .fold(String::with_capacity(64), |mut output, byte| {
                    write!(output, "{byte:02x}").expect("writing to a String cannot fail");
                    output
                })
        })
        .unwrap_or_else(|error| {
            missing_identity(&format!("cannot read {}: {error}", lock_path.display()))
        });
    println!("cargo:rustc-env=ROBIN_CARGO_LOCK_SHA256={hash}");
}

fn emit_build_identity() {
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET to build scripts");
    let profile = std::env::var("PROFILE").expect("Cargo must provide PROFILE to build scripts");
    let mut features = std::env::vars()
        .filter_map(|(key, value)| {
            (value == "1")
                .then(|| key.strip_prefix("CARGO_FEATURE_").map(str::to_owned))
                .flatten()
        })
        .map(|feature| feature.to_ascii_lowercase().replace('_', "-"))
        .collect::<Vec<_>>();
    features.sort();
    features.dedup();
    println!("cargo:rustc-env=ROBIN_BUILD_TARGET={target}");
    println!("cargo:rustc-env=ROBIN_BUILD_PROFILE={profile}");
    println!(
        "cargo:rustc-env=ROBIN_BUILD_FEATURES={}",
        features.join(",")
    );
}
