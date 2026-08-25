//! Build script: emit `ROBIN_GIT_HASH` for replay-format tagging.
//!
//! Shaders are now consumed directly as WGSL by wgpu at runtime — no
//! offline compilation step needed.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ROBIN_PACKAGE_VERSION");
    emit_git_hash();
}

/// Emit `ROBIN_GIT_HASH` as a `cargo:rustc-env` so source code can
/// reference it via `env!("ROBIN_GIT_HASH")`. Used by the replay format
/// to tag recordings with the engine version they were produced on.
fn emit_git_hash() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
    let hash = if let Some(git_meta) = git_meta {
        let actual_git = if git_meta.is_file() {
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
                let rel = std::fs::read_to_string(&commondir_file).unwrap_or_default();
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
            .args(["rev-parse", "--short=12", "HEAD"])
            .current_dir(&manifest_dir)
            .output()
        {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            _ => "unknown".to_string(),
        }
    } else {
        "unknown".to_string()
    };
    println!("cargo:rustc-env=ROBIN_GIT_HASH={hash}");
}
