use super::*;
use wasm_bindgen_test::wasm_bindgen_test;
use winit::keyboard::KeyCode;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn durable_key_config_reload_rejects_unknown_schema() {
    let storage = browser_key_config_storage().unwrap();
    storage.remove_item(BROWSER_KEY_CONFIG_STORE_KEY).unwrap();

    let mut first = KeyConfigStore::load("browser-save").unwrap();
    first
        .entry_or_default(7)
        .active
        .set_binding("ZoomIn", Some(KeyCode::Backspace), None);
    first.save().unwrap();

    let reloaded = KeyConfigStore::load("ignored-after-load").unwrap();
    assert_eq!(
        reloaded
            .get(7)
            .unwrap()
            .active
            .get_binding("ZoomIn")
            .unwrap()
            .primary_key,
        Some(KeyCode::Backspace)
    );
    assert_eq!(reloaded.save_directory, "ignored-after-load");

    storage
        .set_item(
            BROWSER_KEY_CONFIG_STORE_KEY,
            r#"{"schema_version":999,"store":{}}"#,
        )
        .unwrap();
    assert!(KeyConfigStore::load("browser-save").is_err());
    assert!(
        decode_browser_key_config_archive(
            &"x".repeat(BROWSER_KEY_CONFIG_BYTE_LIMIT + 1),
            "browser-save"
        )
        .unwrap_err()
        .to_string()
        .contains("limit")
    );
    storage.remove_item(BROWSER_KEY_CONFIG_STORE_KEY).unwrap();
}
