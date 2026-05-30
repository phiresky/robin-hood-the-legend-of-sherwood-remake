//! Android NativeActivity entry point.

use std::path::{Path, PathBuf};
use std::time::Duration;

use winit::platform::android::activity::AndroidApp;

/// Entry point called by `android-activity`'s NativeActivity glue.
///
/// The symbol intentionally uses Rust ABI, matching android-activity's
/// contract.  It may be invoked again after an Activity recreation.
#[unsafe(no_mangle)]

fn request_activity_finish(app: &AndroidApp, exit_code: i32) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let app = app.clone();
    let callback_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let result = finish_activity_from_java_thread(&callback_app, exit_code);
        let _ = tx.send(result);
    }));

    match rx.recv_timeout(Duration::from_millis(750)) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("timed out waiting for Java main-thread finish callback".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("Java main-thread finish callback channel disconnected".to_string())
        }
    }
}

fn finish_activity_from_java_thread(app: &AndroidApp, exit_code: i32) -> Result<(), String> {
    use jni::objects::{JObject, JValue};
    use jni::refs::Global;
    use jni::signature::RuntimeMethodSignature;

    // SAFETY: android-activity exposes the process JavaVM pointer for
    // exactly this use. ManuallyDrop keeps this borrowed wrapper from
    // trying to destroy or detach the VM when it leaves scope.
    let vm = unsafe { std::mem::ManuallyDrop::new(jni::JavaVM::from_raw(app.vm_as_ptr().cast())) };
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let finish_sig = RuntimeMethodSignature::from_str("(I)V")?;
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        // SAFETY: `activity_as_ptr` returns an unowned global reference
        // that remains valid while `app` is alive. `as_cast_raw` borrows
        // it without taking ownership, so the android-activity global ref
        // is not deleted here.
        let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&raw_activity)? };
        env.call_method(
            activity.as_ref(),
            jni::jni_str!("finishFromNative"),
            finish_sig.method_signature(),
            &[JValue::Int(exit_code as jni::sys::jint)],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

fn install_android_paths(app: &AndroidApp) {
    if std::env::var_os("ROBINHOOD_SAVE_DIR").is_none()
        && let Some(dir) = app.internal_data_path()
    {
        set_env("ROBINHOOD_SAVE_DIR", dir.join("saves"));
    }

    for root in candidate_data_roots(app) {
        if root.join("Data").is_dir() {
            set_env("ROBINHOOD_DATA_DIR", root);
            return;
        }
    }
}

fn candidate_data_roots(app: &AndroidApp) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = app.external_data_path() {
        roots.push(path);
    }
    if let Some(path) = app.internal_data_path() {
        roots.push(path);
    }
    roots.push(Path::new(".").to_path_buf());
    roots
}

fn set_env(key: &str, value: impl AsRef<Path>) {
    let value = value.as_ref();
    tracing::info!("Android {key}={}", value.display());
    // SAFETY: android_main is still in single-threaded startup for our
    // process-level Rust code. The game thread is spawned later by the
    // window bootstrap.
    unsafe { std::env::set_var(key, value) };
}

fn load_bundled_shipping_datadir(
    app: &AndroidApp,
) -> Result<std::sync::Arc<robin_assets::shipping_datadir::ShippingDatadir>, String> {
    use std::ffi::CString;
    use std::io::Read;

    let name = CString::new("Data/datadir.bin").expect("asset path has no interior nul");
    let mut asset = app
        .asset_manager()
        .open(&name)
        .ok_or("Android APK asset Data/datadir.bin is missing")?;
    let mut bytes = Vec::with_capacity(asset.length());
    asset
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read APK asset Data/datadir.bin: {e}"))?;

    let dd = robin_assets::shipping_datadir::ShippingDatadir::from_compressed_bytes(&bytes)
        .map_err(|e| format!("decode APK asset Data/datadir.bin: {e:#}"))?;
    let dd = std::sync::Arc::new(dd);
    let _ = robin_assets::shipping_datadir::install_global(dd.clone());
    let _ = robin_util::asset_fs::install_bundle(std::sync::Arc::new(dd.raw.clone()));
    tracing::info!("Loaded bundled Android shipping datadir from APK assets");
    Ok(dd)
}
