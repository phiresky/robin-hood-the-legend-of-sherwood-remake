//! Android NativeActivity entry point.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use winit::platform::android::activity::AndroidApp;

static ANDROID_APP: std::sync::OnceLock<AndroidApp> = std::sync::OnceLock::new();

/// Entry point called by `android-activity`'s NativeActivity glue.
///
/// This symbol uses the Rust ABI required by `android-activity`. It is not
/// referenced by Rust code, but must remain exported from the Android cdylib.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    crate::init_tracing();

    let exit_code = match run_android(app.clone()) {
        Ok(0) => 0,
        Ok(code) => {
            tracing::error!("Android game exited with code {code}");
            code
        }
        Err(error) => {
            tracing::error!("Android startup failed: {error:#}");
            1
        }
    };

    if let Err(error) = request_activity_finish(&app, exit_code) {
        tracing::warn!("Android Activity finish bridge failed before process exit: {error}");
    }

    // winit/android only permits one EventLoop per process. If the native
    // entry returns, Android may keep the process around and call android_main
    // again on the next launcher tap, which then fails because the EventLoop
    // cannot be recreated. std::process::exit runs Android runtime cleanup
    // here and has hit HWUI destroyed-mutex aborts after a normal menu exit.
    // Ask Java to finish/remove the Activity from the task first, then
    // terminate the native process directly.
    //
    // Manual Android verification: launch the APK, exit from the main menu,
    // confirm `finishFromNative` is logged, and confirm a second launcher tap
    // starts a fresh process instead of reusing the old EventLoop.
    unsafe { libc::_exit(exit_code) };
}

fn run_android(app: AndroidApp) -> anyhow::Result<i32> {
    ANDROID_APP
        .set(app.clone())
        .map_err(|_| anyhow::anyhow!("Android asset manager was already installed"))?;
    install_android_paths(&app);

    let mut args = crate::main_entry::parse_cli();
    // The bundled demo would otherwise auto-launch its mission. Keep the full
    // menu available on Android while preserving all other parsed options.
    args.force_main_menu = true;
    // Android does not currently expose the script-RPC endpoint. On some
    // devices the loopback bind fails with EPERM, so disable the desktop-only
    // listener without changing other application configuration.
    args.http_server = 0;
    let shipping = load_bundled_shipping_datadir(&app)?;
    install_bundled_core_overlay()?;
    let (campaign, profiles, application_context) =
        crate::main_entry::rust_init_with_shipping(Some(shipping))?;

    crate::window::run_with_android_game(
        app,
        "Robin Hood - Legend of Sherwood",
        1024,
        768,
        move |mut window| async move {
            match crate::main_entry::run_rust_game(
                &mut window,
                campaign,
                profiles.clone(),
                application_context,
                &args,
            )
            .await
            {
                Ok(code) => code,
                Err(error) => {
                    tracing::error!("Game loop failed: {error}");
                    1
                }
            }
        },
    )
    .map_err(anyhow::Error::msg)
    .context("Android window/event-loop init failed")
}

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
            Err("timed out waiting for Java main-thread finish callback".to_owned())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("Java main-thread finish callback channel disconnected".to_owned())
        }
    }
}

fn finish_activity_from_java_thread(app: &AndroidApp, exit_code: i32) -> Result<(), String> {
    use jni::objects::{JObject, JValue};
    use jni::refs::Global;
    use jni::signature::RuntimeMethodSignature;

    // SAFETY: android-activity exposes the process JavaVM pointer for exactly
    // this use. ManuallyDrop keeps this borrowed wrapper from trying to
    // destroy or detach the VM when it leaves scope.
    let vm = unsafe { std::mem::ManuallyDrop::new(jni::JavaVM::from_raw(app.vm_as_ptr().cast())) };
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let finish_sig = RuntimeMethodSignature::from_str("(I)V")?;
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        // SAFETY: `activity_as_ptr` returns an unowned global reference that
        // remains valid while `app` is alive. `as_cast_raw` borrows it without
        // taking ownership, so the android-activity global ref is not deleted.
        let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&raw_activity)? };
        env.call_method(
            activity.as_ref(),
            jni::jni_str!("finishFromNative"),
            finish_sig.method_signature(),
            &[JValue::Int(exit_code as jni::sys::jint)],
        )?;
        Ok(())
    })
    .map_err(|error| error.to_string())
}

fn install_android_paths(app: &AndroidApp) {
    if std::env::var_os("ROBINHOOD_SAVE_DIR").is_none()
        && let Some(dir) = app.internal_data_path()
    {
        set_env("ROBINHOOD_SAVE_DIR", dir.join("saves"));
    }

    if std::env::var_os("ROBINHOOD_DATA_DIR").is_some() {
        return;
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
    // process-level Rust code. The game thread is spawned later by the window
    // bootstrap.
    unsafe { std::env::set_var(key, value) };
}

fn load_bundled_shipping_datadir(
    app: &AndroidApp,
) -> anyhow::Result<std::sync::Arc<robin_assets::shipping_datadir::ShippingDatadir>> {
    let _ = app;
    let bytes = read_bundled_asset("Data/datadir.bin")?;

    let datadir = robin_assets::shipping_datadir::ShippingDatadir::from_compressed_bytes(&bytes)
        .context("decode APK asset Data/datadir.bin")?;
    let datadir = robin_assets::shipping_datadir::install_global(std::sync::Arc::new(datadir))
        .context("install Android shipping datadir")?;
    tracing::info!("Loaded bundled Android shipping datadir from APK assets");
    Ok(datadir)
}

fn install_bundled_core_overlay() -> anyhow::Result<()> {
    let manifest_bytes = read_bundled_asset(crate::core_overlay::CORE_OVERLAY_MANIFEST_PATH)
        .context("read bundled Android core overlay manifest")?;
    let (manifest, bundle) =
        crate::core_overlay::load_validated_bundle(&manifest_bytes, read_bundled_asset)
            .context("validate bundled Android core overlay")?;
    crate::core_overlay::install_validated_bundle(
        robin_util::asset_fs::global(),
        &manifest,
        bundle,
    )
    .context("install bundled Android core overlay")?;
    tracing::info!(
        files = manifest.files.len(),
        shipping_schema = manifest.shipping_datadir_schema,
        "Mounted validated Android core overlay ahead of shipping content"
    );
    Ok(())
}

pub(crate) fn read_bundled_asset(path: &str) -> anyhow::Result<Vec<u8>> {
    use std::ffi::CString;
    use std::io::Read;

    let app = ANDROID_APP
        .get()
        .ok_or_else(|| anyhow::anyhow!("Android asset manager is not installed"))?;
    let name = CString::new(path).context("APK asset path contains an interior nul")?;
    let mut asset = app
        .asset_manager()
        .open(&name)
        .ok_or_else(|| anyhow::anyhow!("Android APK asset {path} is missing"))?;
    let mut bytes = Vec::with_capacity(asset.length());
    asset
        .read_to_end(&mut bytes)
        .with_context(|| format!("read APK asset {path}"))?;
    Ok(bytes)
}
