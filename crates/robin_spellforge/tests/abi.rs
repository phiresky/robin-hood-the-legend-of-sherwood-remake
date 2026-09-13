use robin_script_types::spellforge::{
    SPELLFORGE_CONTRACT_VERSION, SpellforgeGuestErrorKind, SpellforgePackage, SpellforgeScriptMode,
};
use robin_spellforge::{compute_package_sha256, spellforge_vm_abi, validate_package};
use std::collections::BTreeMap;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn executable_abi_digest_is_stable() {
    // Contract extraction changed runtime import paths, the build-script source
    // root, and registry dispatch visibility. The source input scope is unchanged.
    // Raw source authentication deliberately includes these bytes (including
    // comments and build.rs). The 274 native signatures/IDs/exposure flags,
    // 51 aliases, vendored interpreter, limits and wire codec are unchanged.
    // Keep the conservative identity boundary: do not normalize source or admit
    // the previous ABI merely because some edits are behavior-preserving.
    assert_eq!(
        spellforge_vm_abi(),
        "spellforge-v1-sha256:6714bc3c41e08bcf534c9e553f20a2298cb5629642e7b4b7b9f422b8770e6eaa"
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn previous_source_identity_is_not_admitted_after_package_roundtrip() {
    let mut package = SpellforgePackage {
        contract_version: SPELLFORGE_CONTRACT_VERSION,
        vm_abi: spellforge_vm_abi().to_owned(),
        script_mode: SpellforgeScriptMode::Replace,
        entrypoint: "mission.lua".to_owned(),
        files: BTreeMap::from([("mission.lua".to_owned(), b"return 0".to_vec())]),
        sha256: [0; 32],
    };
    package.sha256 = compute_package_sha256(&package);
    validate_package(&package).expect("current executable identity is admitted");
    let current_hash = package.sha256;

    package.vm_abi =
        "spellforge-v1-sha256:eb7b23e8b0f7e62bf5756eadc1fcf93de03aa5293d9140d492b02b42816aa7bc"
            .to_owned();
    package.sha256 = compute_package_sha256(&package);
    assert_ne!(
        package.sha256, current_hash,
        "ABI remains part of package identity"
    );
    package
        .validate_wire()
        .expect("old package is internally consistent");
    let decoded: SpellforgePackage =
        serde_json::from_slice(&serde_json::to_vec(&package).unwrap()).unwrap();
    assert_eq!(
        decoded, package,
        "decoding must retain the exact declared ABI"
    );
    let error = validate_package(&decoded).expect_err("old executable identity stays incompatible");
    assert_eq!(error.kind, SpellforgeGuestErrorKind::Compatibility);
    assert!(error.message.contains("unsupported Spellforge VM ABI"));
    assert!(error.message.contains(&package.vm_abi));
    assert!(error.message.contains(spellforge_vm_abi()));
}
