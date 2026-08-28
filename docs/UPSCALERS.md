# Upscalers and presentation effects

The Graphics menu applies a persisted scaling chain to the game or video
layer, then composites menus and the HUD as a separate sharp layer. All
bundled profiles use portable fragment-only WGSL: there are no compute
shaders, storage textures, subgroup operations, derivatives, or float render
targets. This keeps the curated suite available on native wgpu, Android,
browser WebGPU, and wgpu's WebGL2 backend.

## What each name promises

| UI choice | Passes | Conformance and provenance |
| --- | --- | --- |
| Nearest / Linear | one | Standard sampler definitions. |
| Pixel Art / Sharp Bilinear | one | Fractional-scale sharp-bilinear sampling. |
| Bicubic / Lanczos / CUT3 | one | Conventional bicubic, Lanczos-2, and triangulated interpolation kernels. |
| ScaleNX | ScaleNX + artifact removal | Its corner geometry derives from the published Scale2x rule; colour-distance matching, fractional placement, blend strength, and cleanup are project-specific. Published Scale2x rule vectors are tested separately. |
| HQx-style | one | Original colour-distance/corner interpolation informed by the public HQx algorithm description. It is not the LGPL hqnx implementation. |
| xBRZ-style | diagonal reconstruction + cleanup | Original free-scale diagonal analysis. It is not Zenju's GPL xBRZ implementation. |
| Super-xBR-style | diagonal reconstruction + cleanup + de-ringing | Original three-pass profile; it does not claim bit-identical Super-xBR output. |
| Anime line A (v4 layout) | restore + directional upscale + cleanup | Uses Anime4K v4's documented A pass ordering with small original kernels suited to this game's painted 2D input. |
| Anime line B (v4 layout) | soft restore + directional upscale + cleanup | Uses the documented B ordering, not Anime4K's trained CNN weights. |
| Anime line C (v4 layout) | edge-preserving denoise + directional upscale + cleanup | Uses the documented C ordering, not Anime4K's trained CNN weights. |
| CRT Guest-class | one post-effect | Original WGSL exposing scanline, aperture mask, bloom, curvature, and temporal controls. It is not CRT-Guest-Advanced. |
| CRT Royale-class | one post-effect | Original higher-cost slot-mask/beam profile. It is not a port of the GPL CRT-Royale shader. |

These deliberately qualified labels matter: upstream HQx/xBRZ/CRT shader
implementations have licenses that are not interchangeable with this project,
and the compact Anime profiles optimize for a different input domain. The
source and UI therefore never advertise those clean-room profiles as exact
copies.

## RetroArch presets

Standard desktop/native builds include the MPL-2.0 librashader runtime through
the `retroarch-shaders` Cargo feature (minimal native builds may opt out with
`--no-default-features`). That enables discovery of repository Libretro
`.slangp` presets when that collection is installed alongside the game, plus
the native file picker (`I` on the Graphics screen). An imported preset is
validated immediately and its absolute path is persisted; referenced shader
files remain beside the preset, so moving or deleting that directory causes
an explicit load error.

Individual Libretro presets retain their own licenses. In particular,
CRT-Royale is GPL-licensed and is available only as an external runtime preset;
none of its source is copied into the bundled Royale-class WGSL effect.
Librashader's browser path requires precompiled WGSL and WebGL2 cannot run a
`.slangp` chain, so the RetroArch choice is hidden on browser and Android
builds instead of pretending to support it. The curated WGSL suite remains
available on those platforms.

## Frame boundary and persistence

`TextureScaleMode`, `TextureEffect`, and their quantized 0–100 parameters are
stored in each profile. Effects are independently disableable with
`TextureEffect::None`. The temporal uniform comes from the presentation
counter and increments only after `queue.present`, not from the engine tick or
replay frame, so a 120/144 Hz display can animate presentation-only effects
without perturbing deterministic gameplay.

Gameplay draws mark the UI boundary before the HUD; modal menus mark it after
freezing their world backdrop. The renderer first executes scaling/effects on
the scene target and then alpha-composites the transparent UI target using
sharp-bilinear sampling. Top-level UI-only screens use that same sharp profile
for their entire logical frame (while retaining the ordinary target for modal
snapshots). Loading artwork remains in the effected layer, with progress and
version text split into the sharp UI layer. Cutscene video uses the same
scaling/effect config.

### Original presentation parity

The original `RHGame::Refresh` records the world (`RHEngine::Draw`), refreshes
the interface, adds `DrawOver`/messages/debug information, refreshes the
software cursor last, and then calls `SBDrawManager::Flip`. The SDL draw manager
uploads that already-composited opaque RGB565 logical surface and performs one
full-texture `SDL_RenderCopy`; `SDL_RenderSetLogicalSize` owns aspect-preserving
window scaling. There is no surviving alpha channel or separate UI surface in
that presentation path.

The new separation is therefore an intentional extension required by the
sharp-HUD setting, not a claim about an original hardware layer. It preserves
the relevant ordering: mission-space selection, sword-trail, patrol, and macro
feedback remain in the gameplay layer; panels, menus, messages, console, fade,
and software cursor remain ordered above it and are composited last. The
largest aspect-correct destination rectangle mirrors the original logical-size
fit. Because the new transparent UI target contains RGB that has already been
alpha-blended over transparent black, the final overlay uses premultiplied
alpha; multiplying by source alpha a second time would darken font edges and
cursor shadows. Binary-alpha opaque UI blits remain equivalent.

## Primary references

- Anime4K upstream and v4 profiles: <https://github.com/bloc97/Anime4K>
- Anime4K v4.0.1 release: <https://github.com/bloc97/Anime4K/releases/tag/v4.0.1>
- Scale2x algorithm: <https://www.scale2x.it/algorithm>
- librashader runtime and license: <https://github.com/SnowflakePowered/librashader>
- Libretro shader documentation: <https://docs.libretro.com/shader/introduction/>
- Libretro slang shader collection and per-preset licenses: <https://github.com/libretro/slang-shaders>
