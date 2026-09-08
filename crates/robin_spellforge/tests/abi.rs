use robin_spellforge::spellforge_vm_abi;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn executable_abi_digest_is_stable() {
    assert_eq!(
        spellforge_vm_abi(),
        "spellforge-v1-sha256:ef0f0b9cfd12b01cd3037fcde3622c4e4f5706ae669b0e43338837a743853d29"
    );
}
