//! WGPU integration for RetroArch `.slangp` shader presets.

use robin_engine::graphic_config::TextureScaleMode;
#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
use std::collections::{HashMap, HashSet};
#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
use std::fs;
#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
use std::path::{Path, PathBuf};
#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
use std::sync::LazyLock;

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
use librashader::presets::{ShaderFeatures, ShaderPreset};
#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
use librashader::runtime::wgpu::{FilterChain, WgpuOutputView};
#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
use librashader::runtime::{Size, Viewport};

use crate::window::GpuContext;

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
static SLANG_SHADER_ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
    let relative = Path::new("third_party/slang-shaders");
    let mut candidates = vec![
        relative.to_path_buf(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative),
    ];
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        candidates.push(directory.join(relative));
        candidates.push(directory.join("../..").join(relative));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_dir())
        // Keep a deterministic path for explicit errors even when the
        // optional preset collection was not packaged.
        .unwrap_or_else(|| relative.to_path_buf())
});

#[derive(Debug, Clone)]
pub struct RetroArchPresetInfo {
    pub id: String,
    pub label: String,
}

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
static RETROARCH_PRESETS: LazyLock<Vec<RetroArchPresetInfo>> =
    LazyLock::new(discover_retroarch_presets_uncached);

pub fn is_shader_preset_mode(mode: TextureScaleMode) -> bool {
    matches!(mode, TextureScaleMode::RetroArch)
}

pub fn retroarch_presets() -> &'static [RetroArchPresetInfo] {
    #[cfg(not(all(feature = "retroarch-shaders", not(target_arch = "wasm32"))))]
    {
        &[]
    }
    #[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
    &RETROARCH_PRESETS
}

/// RetroArch import is a native-only integration. Browser WebGPU uses the
/// curated WGSL suite because librashader's web path requires offline preset
/// compilation, and WebGL2 cannot run it at all.
pub const fn retroarch_runtime_available() -> bool {
    cfg!(all(
        feature = "retroarch-shaders",
        not(target_arch = "wasm32"),
        any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos"
        )
    ))
}

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
pub struct ShaderPresetRenderer {
    gpu: GpuContext,
    chains: HashMap<String, FilterChain>,
    failed_keys: HashSet<String>,
    frame_count: usize,
}

#[cfg(not(all(feature = "retroarch-shaders", not(target_arch = "wasm32"))))]
pub struct ShaderPresetRenderer;

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
impl ShaderPresetRenderer {
    pub fn new(gpu: GpuContext) -> Self {
        Self {
            gpu,
            chains: HashMap::new(),
            failed_keys: HashSet::new(),
            frame_count: 0,
        }
    }

    pub fn validate_preset(&mut self, key: &str) -> Result<(), String> {
        if self.chains.contains_key(key) {
            return Ok(());
        }
        let chain = self.load_chain(key)?;
        self.failed_keys.remove(key);
        self.chains.insert(key.to_string(), chain);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        mode: TextureScaleMode,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        target_size: [u32; 2],
        dst_rect: [f32; 4],
        target_format: wgpu::TextureFormat,
        frame_count: Option<usize>,
        retroarch_preset: Option<&str>,
    ) -> Result<(), String> {
        if !is_shader_preset_mode(mode) {
            return Err(format!("{mode:?} is not a RetroArch preset mode"));
        }
        let key = preset_key(mode, retroarch_preset)?;
        if self.failed_keys.contains(&key) {
            return Err(format!(
                "shader preset {key} failed earlier in this session"
            ));
        }
        if !self.chains.contains_key(&key) {
            match self.load_chain(&key) {
                Ok(chain) => {
                    self.chains.insert(key.clone(), chain);
                }
                Err(error) => {
                    self.failed_keys.insert(key.clone());
                    return Err(error);
                }
            }
        }
        let chain = self
            .chains
            .get_mut(&key)
            .expect("shader preset chain inserted above");
        let output_size = Size {
            width: dst_rect[2].max(1.0).ceil() as u32,
            height: dst_rect[3].max(1.0).ceil() as u32,
        };
        let target_size = Size {
            width: target_size[0].max(1),
            height: target_size[1].max(1),
        };
        let shader_frame_count = frame_count.unwrap_or(self.frame_count);
        if let Err(e) = chain.frame(
            source,
            &Viewport {
                x: dst_rect[0],
                y: dst_rect[1],
                mvp: None,
                output: WgpuOutputView::new_from_raw(target_view, target_size, target_format),
                size: output_size,
            },
            encoder,
            shader_frame_count,
            None,
        ) {
            tracing::error!("librashader WGPU frame failed for {key}: {e}");
            self.failed_keys.insert(key.clone());
            self.chains.remove(&key);
            return Err(format!("librashader WGPU frame failed for {key}: {e}"));
        }
        if frame_count.is_none() {
            self.frame_count = self.frame_count.wrapping_add(1);
        }
        Ok(())
    }

    fn load_chain(&self, key: &str) -> Result<FilterChain, String> {
        let path = preset_path(key);
        let preset = match ShaderPreset::try_parse(&path, ShaderFeatures::NONE) {
            Ok(preset) => preset,
            Err(e) => {
                tracing::error!("failed to parse shader preset {}: {e}", path.display());
                return Err(format!(
                    "failed to parse shader preset {}: {e}",
                    path.display()
                ));
            }
        };
        match FilterChain::load_from_preset(preset, &self.gpu.device, &self.gpu.queue, None) {
            Ok(chain) => Ok(chain),
            Err(e) => {
                tracing::error!("failed to compile shader preset {}: {e}", path.display());
                Err(format!(
                    "failed to compile shader preset {}: {e}",
                    path.display()
                ))
            }
        }
    }
}

#[cfg(not(all(feature = "retroarch-shaders", not(target_arch = "wasm32"))))]
impl ShaderPresetRenderer {
    pub fn new(_gpu: GpuContext) -> Self {
        Self
    }

    pub fn validate_preset(&mut self, _key: &str) -> Result<(), String> {
        Err("RetroArch preset support is unavailable in this build; rebuild with --features retroarch-shaders".to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        _mode: TextureScaleMode,
        _encoder: &mut wgpu::CommandEncoder,
        _source: &wgpu::Texture,
        _target_view: &wgpu::TextureView,
        _target_size: [u32; 2],
        _dst_rect: [f32; 4],
        _target_format: wgpu::TextureFormat,
        _frame_count: Option<usize>,
        _retroarch_preset: Option<&str>,
    ) -> Result<(), String> {
        Err("RetroArch preset support is unavailable in this build; rebuild with --features retroarch-shaders".to_string())
    }
}

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
fn preset_key(mode: TextureScaleMode, retroarch_preset: Option<&str>) -> Result<String, String> {
    match mode {
        TextureScaleMode::RetroArch => retroarch_preset
            .filter(|preset| !preset.trim().is_empty())
            .or_else(|| retroarch_presets().first().map(|preset| preset.id.as_str()))
            .map(str::to_string)
            .ok_or_else(|| {
                "RetroArch shader mode selected but no .slangp preset was chosen".to_string()
            }),
        _ => unreachable!("non-preset mode checked before preset_key"),
    }
}

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
fn preset_path(key: &str) -> PathBuf {
    let path = PathBuf::from(key);
    if path.is_absolute() {
        path
    } else {
        SLANG_SHADER_ROOT.join(path)
    }
}

#[cfg(all(feature = "retroarch-shaders", not(target_arch = "wasm32")))]
fn discover_retroarch_presets_uncached() -> Vec<RetroArchPresetInfo> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<RetroArchPresetInfo>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out);
            } else if path.extension().is_some_and(|ext| ext == "slangp") {
                let Ok(relative) = path.strip_prefix(root) else {
                    continue;
                };
                let id = relative.to_string_lossy().replace('\\', "/");
                let label = id.trim_end_matches(".slangp").replace('/', " / ");
                out.push(RetroArchPresetInfo { id, label });
            }
        }
    }

    let mut presets = Vec::new();
    visit(&SLANG_SHADER_ROOT, &SLANG_SHADER_ROOT, &mut presets);
    presets.sort_by(|a, b| a.label.cmp(&b.label));
    presets
}
