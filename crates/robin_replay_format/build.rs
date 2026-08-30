//! Emit the source identity that is part of the compact replay envelope.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut search = manifest_dir.as_path();
    let git_meta = loop {
        let candidate = search.join(".git");
        if candidate.exists() {
            break Some(candidate);
        }
        match search.parent() {
            Some(parent) => search = parent,
            None => break None,
        }
    };

    let commit = if let Some(git_meta) = git_meta {
        let actual_git = if git_meta.is_file() {
            std::fs::read_to_string(&git_meta)
                .ok()
                .and_then(|contents| {
                    contents
                        .lines()
                        .next()
                        .and_then(|line| line.strip_prefix("gitdir:").map(str::trim))
                        .map(|path| {
                            let path = PathBuf::from(path);
                            if path.is_absolute() {
                                path
                            } else {
                                git_meta.parent().unwrap_or(Path::new(".")).join(path)
                            }
                        })
                })
                .unwrap_or_else(|| git_meta.clone())
        } else {
            git_meta
        };
        let head = actual_git.join("HEAD");
        if head.exists() {
            println!("cargo:rerun-if-changed={}", head.display());
            let common_root = std::fs::read_to_string(actual_git.join("commondir"))
                .ok()
                .map(|path| {
                    let path = PathBuf::from(path.trim());
                    if path.is_absolute() {
                        path
                    } else {
                        actual_git.join(path)
                    }
                })
                .unwrap_or(actual_git);
            if let Ok(head_text) = std::fs::read_to_string(&head)
                && let Some(reference) = head_text
                    .lines()
                    .next()
                    .and_then(|line| line.strip_prefix("ref:").map(str::trim))
            {
                println!(
                    "cargo:rerun-if-changed={}",
                    common_root.join(reference).display()
                );
            }
            let packed_refs = common_root.join("packed-refs");
            if packed_refs.exists() {
                println!("cargo:rerun-if-changed={}", packed_refs.display());
            }
        }
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&manifest_dir)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_else(|| "unknown".to_owned())
    } else {
        "unknown".to_owned()
    };
    let short = commit.get(..12).unwrap_or(&commit);
    println!("cargo:rustc-env=ROBIN_GIT_HASH={short}");
}
