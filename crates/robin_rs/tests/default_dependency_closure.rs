//! Guard the fast default client build against opt-in integrations leaking in.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[test]
fn default_dependency_closure_excludes_optional_integrations() {
    let output = std::process::Command::new(env!("CARGO"))
        .args(["metadata", "--format-version=1"])
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
        "dirs",
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
        "tokio",
        "velopack",
    ];
    let present = forbidden
        .into_iter()
        .filter(|name| names.contains(*name))
        .collect::<Vec<_>>();
    assert!(
        present.is_empty(),
        "optional integrations reached the default robin_rs closure: {present:?}"
    );
}
