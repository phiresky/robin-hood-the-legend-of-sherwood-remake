use robin_spellforge::spellforge_vm_abi;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn executable_abi_digest_is_stable() {
    assert_eq!(
        spellforge_vm_abi(),
        "spellforge-v1-sha256:ce382c5485879d8d2eb8629bb19d40ef237562f0978f6ec1cef6640ff11421ea"
    );
}
