//! Main game binary for the Rust port of Robin Hood — The Legend of Sherwood.
#![deny(clippy::print_stdout, clippy::print_stderr)]

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    install_crash_diagnostics();
    let exit = run_native();
    std::process::exit(exit);
}

/// Native entry: parse CLI, init data, then bring up winit + wgpu and
/// run the async game on a dedicated thread (driven by `pollster`).
#[cfg(not(target_arch = "wasm32"))]
fn run_native() -> i32 {
    let args = robin_rs::main_entry::parse_cli();
    if let Some(addr) = args.lobby_server.as_deref() {
        robin_rs::init_tracing();
        return match robin_rs::multiplayer::lobby::run_lobby_server(addr) {
            Ok(()) => 0,
            Err(e) => {
                tracing::error!("Lobby server failed: {e}");
                1
            }
        };
    }
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

/// JS entry point.  Hand in the contents of `Data/datadir.bin` (the
/// bitcode-serialised + zstd-compressed asset bundle the converter
/// emits) — Rust decodes it, installs it as the asset bundle, then
/// runs the game under winit's web backend.  Returns immediately on
/// success; the game itself is driven by `requestAnimationFrame`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn wasm_boot(datadir_bin: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    let dd = assets_shipping_datadir::ShippingDatadir::from_compressed_bytes(datadir_bin)
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("datadir decode: {e:#}")))?;
    let dd = std::sync::Arc::new(dd);
    assets_shipping_datadir::install_global(dd.clone()).map_err(|e| {
        wasm_bindgen::JsValue::from_str(&format!("install shipping datadir: {e:#}"))
    })?;
    robin_rs::http_server::start_global(0)
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("rpc init: {e}")))?;

    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_main(dd).await {
            tracing::error!("wasm boot failed: {e}");
        }
    });
    Ok(())
}

/// Register one host-preloaded asset before `wasm_boot` starts the
/// game loop.  The browser loader uses this for large per-level files
/// kept outside `datadir.bin` while Rust keeps a synchronous read API.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn wasm_preload_asset(path: &str, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    robin_util::asset_fs::install_preloaded_asset(path, bytes.to_vec())
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("preload asset {path}: {e}")))
}

#[cfg(target_arch = "wasm32")]
async fn wasm_main(
    shipping: std::sync::Arc<assets_shipping_datadir::ShippingDatadir>,
) -> Result<(), String> {
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
    .map_err(|e| format!("Window/event-loop init failed: {e}"))
}
