#![cfg(all(feature = "native-admission", not(target_arch = "wasm32")))]

#[path = "../admission_identity.rs"]
mod admission_identity;

#[test]
fn dirty_engine_schema_and_configuration_change_admission_identity() {
    let checkout = tempfile::tempdir().unwrap();
    for path in admission_identity::SOURCES {
        let path = checkout.path().join(path);
        if path.extension().is_some() {
            std::fs::write(path, "manifest or lock").unwrap();
        } else {
            std::fs::create_dir_all(path).unwrap();
        }
    }
    let schema = checkout.path().join("crates/robin_engine/replay.rs");
    std::fs::write(&schema, "struct Snapshot { frame: u32 }").unwrap();
    let original = admission_identity::fingerprint(checkout.path(), &[], |_| {}).unwrap();
    // No commit/HEAD change: only the serialized source changed.
    std::fs::write(&schema, "struct Snapshot { frame: u64 }").unwrap();
    let dirty = admission_identity::fingerprint(checkout.path(), &[], |_| {}).unwrap();
    assert_ne!(original, dirty);
    assert_eq!(
        dirty,
        admission_identity::fingerprint(checkout.path(), &[], |_| {}).unwrap()
    );
    assert_ne!(
        dirty,
        admission_identity::fingerprint(
            checkout.path(),
            &[("CARGO_CFG_TARGET_ENDIAN".into(), "big".into())],
            |_| {}
        )
        .unwrap()
    );
    let added = checkout.path().join("crates/robin_engine/new_schema.rs");
    std::fs::write(added, "struct Added {}").unwrap();
    assert_ne!(
        dirty,
        admission_identity::fingerprint(checkout.path(), &[], |_| {}).unwrap()
    );
}
