//! Main game binary for the Rust port of Robin Hood — The Legend of Sherwood.
#![deny(clippy::print_stdout, clippy::print_stderr)]
// GUI subsystem so Windows/Wine doesn't pop up a console window for the game.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(target_arch = "wasm32")]
use anyhow::Context as _;
#[cfg(target_arch = "wasm32")]
use robin_assets::shipping_datadir as assets_shipping_datadir;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // The GUI subsystem detaches us from any console; re-attach to the
    // parent's so stdout/stderr reach the terminal when launched from one.
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
    // Velopack may consume installer/update activation arguments and exit or
    // restart the process, so its startup hook must run before all game setup.
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    velopack::VelopackApp::build().run();

    install_crash_diagnostics();
    robin_rs::init_tracing();
    let args = robin_rs::main_entry::parse_cli();
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    let updater = if args.headless {
        None
    } else {
        robin_rs::auto_update::start_github_auto_update()
    };
    let exit = run_native(args);
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    robin_rs::auto_update::apply_downloaded_update(updater);
    std::process::exit(exit);
}

/// Native entry: parse CLI, init data, then bring up winit + wgpu and
/// run the async game on a dedicated thread (driven by `pollster`).
#[cfg(not(target_arch = "wasm32"))]
fn run_native(args: robin_rs::main_entry::CliArgs) -> i32 {
    let (campaign, profiles, shipping) = match robin_rs::main_entry::rust_init() {
        Ok(c) => {
            tracing::info!("Rust initialization complete.");
            c
        }
        Err(e) => {
            tracing::error!("{}", e);
            return 1;
        }
    };

    if args.headless {
        return match pollster::block_on(robin_rs::main_entry::run_rust_game_headless(
            campaign, profiles, shipping, &args,
        )) {
            Ok(code) => code,
            Err(e) => {
                tracing::error!("Headless game loop failed: {e}");
                1
            }
        };
    }

    match robin_rs::window::run_with_game(
        "Robin Hood — Legend of Sherwood",
        1024,
        768,
        move |mut window| async move {
            match robin_rs::main_entry::run_rust_game(
                &mut window,
                campaign,
                profiles.clone(),
                shipping,
                &args,
            )
            .await
            {
                Ok(code) => code,
                Err(e) => {
                    tracing::error!("Game loop failed: {e}");
                    1
                }
            }
        },
    ) {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("Window/event-loop init failed: {e}");
            1
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn install_crash_diagnostics() {
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        // SAFETY: single-threaded before main; no other thread is
        // reading the environment yet.
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "panic", "{}", info);
        default_hook(info);
    }));

    #[cfg(unix)]
    unsafe {
        for sig in [
            libc_sig::SIGSEGV,
            libc_sig::SIGABRT,
            libc_sig::SIGILL,
            libc_sig::SIGBUS,
        ] {
            libc_sig::signal(sig, crash_handler as *const () as usize);
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
extern "C" fn crash_handler(sig: std::ffi::c_int) {
    let msg: &[u8] = match sig {
        libc_sig::SIGSEGV => b"\n[robin] fatal: SIGSEGV (segfault)\n",
        libc_sig::SIGABRT => b"\n[robin] fatal: SIGABRT (abort -- usually assertion / panic)\n",
        libc_sig::SIGILL => b"\n[robin] fatal: SIGILL (illegal instruction)\n",
        libc_sig::SIGBUS => b"\n[robin] fatal: SIGBUS (bad memory access)\n",
        _ => b"\n[robin] fatal: unknown signal\n",
    };
    unsafe {
        libc_sig::write(2, msg.as_ptr().cast(), msg.len());
        libc_sig::signal(sig, libc_sig::SIG_DFL);
        libc_sig::raise(sig);
    }
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
#[allow(non_camel_case_types)]
mod libc_sig {
    use std::ffi::{c_int, c_void};
    pub const SIGSEGV: c_int = 11;
    pub const SIGABRT: c_int = 6;
    pub const SIGILL: c_int = 4;
    pub const SIGBUS: c_int = 7;
    pub const SIG_DFL: usize = 0;
    unsafe extern "C" {
        pub unsafe fn signal(signum: c_int, handler: usize) -> usize;
        pub unsafe fn raise(sig: c_int) -> c_int;
        pub unsafe fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    }
}

// ---------------------------------------------------------------------
// Wasm entry — wasm-bindgen-driven.  Module instantiation calls
// `wasm_start`, which installs the panic hook + tracing-wasm subscriber
// and spawn_locals the actual `wasm_main` future.  `main()` is kept as
// a no-op stub so cargo's `wasm32-unknown-unknown` bin link succeeds.
// ---------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn main() {}

/// Wasm boot — installed at module-instantiation time by wasm-bindgen.
/// Just sets up panic + tracing.  The JS host calls [`wasm_boot`]
/// after fetching the datadir bundle.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn wasm_start() {
    // Worker-pool builds re-instantiate this module inside each Web Worker
    // against the shared memory, which reruns the start export. Process-wide
    // state (panic hook, global tracing subscriber) must only be installed by
    // the first (main-thread) instantiation — tracing's global-default set
    // would panic on the second call.
    #[cfg(feature = "wasm-threads")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static STARTED: AtomicBool = AtomicBool::new(false);
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
    }
    console_error_panic_hook::set_once();
    let (max_level, invalid_level) = wasm_log_level_from_query();
    let mut tracing_config = tracing_wasm::WASMLayerConfigBuilder::new();
    tracing_config
        .set_max_level(max_level)
        .set_report_logs_in_timings(false);
    tracing_wasm::set_as_global_default_with_config(tracing_config.build());
    if let Some(value) = invalid_level {
        tracing::warn!("invalid wasm-log query value {value:?}; using info");
    }
    tracing::info!("wasm module instantiated; awaiting boot()");
}

#[cfg(target_arch = "wasm32")]
fn wasm_log_level_from_query() -> (tracing::Level, Option<String>) {
    let Some(params) = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
    else {
        return (tracing::Level::INFO, None);
    };

    let Some(value) = params.get("wasm-log").or_else(|| params.get("wasm_log")) else {
        return (tracing::Level::INFO, None);
    };

    match value.to_ascii_lowercase().as_str() {
        "error" => (tracing::Level::ERROR, None),
        "warn" | "warning" => (tracing::Level::WARN, None),
        "info" => (tracing::Level::INFO, None),
        "debug" => (tracing::Level::DEBUG, None),
        "trace" => (tracing::Level::TRACE, None),
        _ => (tracing::Level::INFO, Some(value)),
    }
}

/// JS entry point. Hand in the contents of `Data/datadir.bin` (the
/// bitcode-serialised + zstd-compressed boot manifest the converter emits)
/// and its containing URL. Rust decodes it, installs its boot assets, then
/// runs the game under winit's web backend.  Returns immediately on
/// success; the game itself is driven by `requestAnimationFrame`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn wasm_boot(datadir_bin: &[u8], data_base_url: String) -> Result<(), wasm_bindgen::JsValue> {
    let mut dd = assets_shipping_datadir::ShippingDatadir::from_compressed_bytes(datadir_bin)
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("datadir decode: {e:#}")))?;
    dd.set_remote_base_url(data_base_url);
    let dd = assets_shipping_datadir::install_global(std::sync::Arc::new(dd)).map_err(|e| {
        wasm_bindgen::JsValue::from_str(&format!("install shipping datadir: {e:#}"))
    })?;
    robin_rs::http_server::start_global(0)
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("rpc init: {e}")))?;

    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_main(dd).await {
            tracing::error!("wasm boot failed: {e:#}");
        }
    });
    Ok(())
}

/// Register one host-preloaded asset before `wasm_boot` starts the game loop.
/// The browser loader currently uses this for audio assets that retain a
/// synchronous read API; mission data uses the asynchronous shipping loader.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn wasm_preload_asset(path: &str, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    robin_util::asset_fs::install_preloaded_asset(path, bytes.to_vec())
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("preload asset {path}: {e}")))
}

/// Worker-pool bring-up for `wasm-threads` builds. Requires cross-origin
/// isolation (`SharedArrayBuffer`); without it the sprite decode paths stay
/// on their serial fallback, which is a supported configuration — pages
/// served without COOP/COEP (e.g. the very first visit before the
/// coi-serviceworker reload) must still boot.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
async fn wasm_init_thread_pool() {
    let global = js_sys::global();
    let isolated = js_sys::Reflect::get(&global, &"crossOriginIsolated".into())
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !isolated {
        tracing::info!("page is not cross-origin isolated; sprite decode stays single-threaded");
        return;
    }
    let threads = js_sys::Reflect::get(&global, &"navigator".into())
        .and_then(|navigator| js_sys::Reflect::get(&navigator, &"hardwareConcurrency".into()))
        .ok()
        .and_then(|value| value.as_f64())
        .map(|value| value as usize)
        .filter(|&threads| threads >= 1)
        .unwrap_or(1);
    match robin_assets::wasm_threads::init_pool(threads).await {
        Ok(()) => tracing::info!(threads, "wasm rayon worker pool ready"),
        // A failed pool spawn leaves the serial decode path fully functional.
        Err(error) => tracing::warn!("wasm worker pool init failed, staying serial: {error:#}"),
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_main(
    shipping: std::sync::Arc<assets_shipping_datadir::ShippingDatadir>,
) -> anyhow::Result<()> {
    #[cfg(feature = "wasm-threads")]
    wasm_init_thread_pool().await;
    let args = robin_rs::main_entry::parse_cli();
    let (campaign, profiles, shipping) =
        robin_rs::main_entry::rust_init_with_shipping(Some(shipping))?;
    tracing::info!("Rust initialization complete.");

    robin_rs::window::run_with_game(
        "Robin Hood — Legend of Sherwood",
        1024,
        768,
        move |mut window| async move {
            match robin_rs::main_entry::run_rust_game(
                &mut window,
                campaign,
                profiles.clone(),
                shipping,
                &args,
            )
            .await
            {
                Ok(code) => code,
                Err(e) => {
                    tracing::error!("Game loop failed: {e}");
                    1
                }
            }
        },
    )
    .map(|_| ())
    .map_err(anyhow::Error::msg)
    .context("Window/event-loop init failed")
}
