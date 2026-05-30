//! WGPU integration for RetroArch `.slangp` shader presets.

#[cfg(not(target_arch = "wasm32"))]
use robin_engine::graphic_config::TextureScaleMode;
use std::collections::{HashMap, HashSet};
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::LazyLock;

#[cfg(not(target_arch = "wasm32"))]
use librashader::presets::{ShaderFeatures, ShaderPreset};
#[cfg(not(target_arch = "wasm32"))]
use librashader::runtime::wgpu::{FilterChain, WgpuOutputView};
#[cfg(not(target_arch = "wasm32"))]
use librashader::runtime::{Size, Viewport};

use crate::window::GpuContext;

#[cfg(not(target_arch = "wasm32"))]
static REPO_ROOT: LazyLock<PathBuf> =
    LazyLock::new(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
#[cfg(not(target_arch = "wasm32"))]
static SLANG_SHADER_ROOT: LazyLock<PathBuf> =
    LazyLock::new(|| REPO_ROOT.join("third_party/slang-shaders"));

#[derive(Debug, Clone)]
pub struct RetroArchPresetInfo {
    pub id: String,
    pub label: String,
}

#[cfg(not(target_arch = "wasm32"))]
static RETROARCH_PRESETS: LazyLock<Vec<RetroArchPresetInfo>> =
    LazyLock::new(discover_retroarch_presets_uncached);

pub fn is_shader_preset_mode(mode: TextureScaleMode) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = mode;
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    matches!(mode, TextureScaleMode::RetroArch)
}

pub fn retroarch_presets() -> &'static [RetroArchPresetInfo] {
    #[cfg(target_arch = "wasm32")]
    {
        &[]
    }
    #[cfg(not(target_arch = "wasm32"))]
    &RETROARCH_PRESETS
}

#[cfg(not(target_arch = "wasm32"))]
pub struct ShaderPresetRenderer {
    gpu: GpuContext,
    chains: HashMap<String, FilterChain>,
    failed_keys: HashSet<String>,
    frame_count: usize,
}

#[cfg(target_arch = "wasm32")]
pub struct ShaderPresetRenderer;

#[cfg(not(target_arch = "wasm32"))]
impl ShaderPresetRenderer {
    pub fn new(gpu: GpuContext) -> Self {
        Self {
            gpu,
            chains: HashMap::new(),
            failed_keys: HashSet::new(),
            frame_count: 0,
        }
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
    ) -> Option<()> {
        if !is_shader_preset_mode(mode) {
            return None;
        }
        let key = preset_key(mode, retroarch_preset)?;
        if self.failed_keys.contains(&key) {
            return None;
        }
        if !self.chains.contains_key(&key) {
            match self.load_chain(&key) {
                Some(chain) => {
                    self.chains.insert(key.clone(), chain);
                }
                None => {
                    self.failed_keys.insert(key);
                    return None;
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
            return None;
        }
        if frame_count.is_none() {
            self.frame_count = self.frame_count.wrapping_add(1);
        }
        Some(())
    }

    fn load_chain(&self, key: &str) -> Option<FilterChain> {
        let path = preset_path(key);
        let preset = match ShaderPreset::try_parse(&path, ShaderFeatures::NONE) {
            Ok(preset) => preset,
            Err(e) => {
                tracing::error!("failed to parse shader preset {}: {e}", path.display());
                return None;
            }
        };
        match FilterChain::load_from_preset(preset, &self.gpu.device, &self.gpu.queue, None) {
            Ok(chain) => Some(chain),
            Err(e) => {
                tracing::error!("failed to compile shader preset {}: {e}", path.display());
                None
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl ShaderPresetRenderer {
    pub fn new(_gpu: GpuContext) -> Self {
        Self
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
    ) -> Option<()> {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn preset_key(mode: TextureScaleMode, retroarch_preset: Option<&str>) -> Option<String> {
    match mode {
        TextureScaleMode::RetroArch => retroarch_preset
            .filter(|preset| !preset.trim().is_empty())
            .or_else(|| retroarch_presets().first().map(|preset| preset.id.as_str()))
            .map(str::to_string)
            .or_else(|| {
                tracing::warn!("RetroArch shader mode selected but no .slangp presets were found");
                None
            }),
        _ => unreachable!("non-preset mode checked before preset_key"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn preset_path(key: &str) -> PathBuf {
    SLANG_SHADER_ROOT.join(key)
}

#[cfg(not(target_arch = "wasm32"))]
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
