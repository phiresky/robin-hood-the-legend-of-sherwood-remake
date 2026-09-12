//! Guard the fast default client build against opt-in integrations leaking in.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[test]
fn video_dependency_features_exclude_capture_and_filter_subsystems() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--locked",
            "--format-version=1",
            "--features=video",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run video cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("decode cargo metadata");
    for (name, required, forbidden) in [
        ("ffmpeg-next", "format", ["device", "filter"]),
        ("ffmpeg-sys-next", "avformat", ["avdevice", "avfilter"]),
    ] {
        let package = metadata["packages"]
            .as_array()
            .expect("packages")
            .iter()
            .find(|package| package["name"] == name)
            .expect("video dependency package");
        let node = metadata["resolve"]["nodes"]
            .as_array()
            .expect("resolve nodes")
            .iter()
            .find(|node| node["id"] == package["id"])
            .expect("enabled video dependency");
        let features = node["features"].as_array().expect("resolved features");
        assert!(
            features.iter().any(|feature| feature == required),
            "{name} needs {required}"
        );
        for feature in forbidden {
            assert!(
                !features.iter().any(|enabled| enabled == feature),
                "unused {name}/{feature} expands the native worker dependency footprint"
            );
        }
    }
}

#[test]
fn default_dependency_closure_excludes_optional_integrations() {
    assert_client_dependency_policy(false);
}

#[test]
fn script_rpc_dependency_closure_does_not_enable_multiplayer() {
    assert_client_dependency_policy(true);
}

fn assert_client_dependency_policy(script_rpc: bool) {
    let mut command = std::process::Command::new(env!("CARGO"));
    command.args(["metadata", "--locked", "--format-version=1"]);
    if script_rpc {
        command.arg("--features=script-rpc");
    }
    let output = command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("decode cargo metadata");
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages array");
    let package_names = packages
        .iter()
        .map(|package| {
            (
                package["id"].as_str().expect("package id").to_owned(),
                package["name"].as_str().expect("package name").to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let graph = metadata["resolve"]["nodes"]
        .as_array()
        .expect("metadata resolve nodes")
        .iter()
        .map(|node| {
            let id = node["id"].as_str().expect("node id").to_owned();
            let dependencies = node["deps"]
                .as_array()
                .expect("node deps")
                .iter()
                .filter(|dependency| {
                    dependency["dep_kinds"]
                        .as_array()
                        .expect("dependency kinds")
                        .iter()
                        .any(|kind| kind["kind"].is_null())
                })
                .map(|dependency| {
                    dependency["pkg"]
                        .as_str()
                        .expect("dependency package id")
                        .to_owned()
                })
                .collect::<Vec<_>>();
            (id, dependencies)
        })
        .collect::<BTreeMap<_, _>>();
    let root = package_names
        .iter()
        .find_map(|(id, name)| (name == "robin_rs").then_some(id.clone()))
        .expect("robin_rs package in metadata");
    // Tokio remains shared native infrastructure. Only the opt-in listener
    // owns direct Hyper dependencies; neither configuration needs multiplayer.
    // Transitive Hyper use by HTTP clients is independent of listener authority.
    let direct_dependencies = graph.get(&root).expect("robin_rs dependency node");
    assert!(
        direct_dependencies
            .iter()
            .any(|id| package_names.get(id).is_some_and(|name| name == "tokio")),
        "native client infrastructure requires Tokio without enabling multiplayer"
    );
    for listener_dependency in ["hyper", "hyper-util"] {
        let directly_enabled = direct_dependencies.iter().any(|id| {
            package_names
                .get(id)
                .is_some_and(|name| name == listener_dependency)
        });
        assert_eq!(
            directly_enabled, script_rpc,
            "direct {listener_dependency} dependency must follow script-rpc={script_rpc}"
        );
    }
    // OS data/save directories are standard native functionality, not an
    // optional desktop integration. Metadata includes target-specific edges.
    assert!(
        direct_dependencies
            .iter()
            .any(|id| package_names.get(id).is_some_and(|name| name == "dirs")),
        "native builds require standard OS data directories"
    );
    let mut queue = VecDeque::from([root]);
    let mut closure = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if !closure.insert(id.clone()) {
            continue;
        }
        queue.extend(graph.get(&id).into_iter().flatten().cloned());
    }
    let names = closure
        .iter()
        .filter_map(|id| package_names.get(id))
        .cloned()
        .collect::<BTreeSet<_>>();
    let forbidden = [
        "cpal",
        "ffmpeg-next",
        "ffmpeg-sys-next",
        "gilrs",
        "glslang",
        "glslang-sys",
        "iroh",
        "iroh-gossip",
        "iroh-mainline-address-lookup",
        "kira",
        "librashader",
        "mainline",
        "mlua",
        "mlua-sys",
        "ogg",
        "rfd",
        "robin_lua",
        "spirv-cross-sys",
        "sysinfo",
        "tiny_http",
        "velopack",
    ];
    let present = forbidden
        .into_iter()
        .filter(|name| names.contains(*name))
        .collect::<Vec<_>>();
    assert!(
        present.is_empty(),
        "optional integrations reached the robin_rs closure with script-rpc={script_rpc}: {present:?}"
    );
}
