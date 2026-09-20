# Modding tools

Native GitHub release packages include these command-line tools alongside the
game executable. On Windows, open a terminal in the installed application's
binary directory (or the extracted portable package) and use the `.exe` names.
On Linux, extract the AppImage with `--appimage-extract` to access its bundled
executables. Each tool supports `--help`.

| Tool | Purpose |
| --- | --- |
| `cpf_to_json` | Export binary CPF profiles to canonical JSON; validate JSON and preview patches. |
| `encode_mod_sprites` | Encode authored sprite directories and terrain PNGs. |
| `disasm_scb` | Disassemble or decompile compiled mission scripts. |
| `dump_res` | Dump resource archive metadata as JSON. |

Build all four from source:

```sh
cargo build -p robin_modding_tools --bins
target/debug/cpf_to_json /path/to/Data/Configuration/profile.cpf profiles.json
target/debug/cpf_to_json --patch profiles.patch.json /path/to/Data/Configuration/profile.cpf patched.json
target/debug/encode_mod_sprites SOURCE_MOD NEW_DESTINATION_MOD
target/debug/disasm_scb --decompile /path/to/mission.scb
target/debug/dump_res /path/to/archive.res > resources.json
```

CPF and resource JSON go to stdout when no output file is supplied; diagnostics
go to stderr. See [JSON Patch mods](JSON_PATCH_MODS.md) for profile patch authoring.

The sprite encoder also supports `--family OUTPUT INPUT_RHS_DIR...` and
`--map INPUT_PNG OUTPUT_MAP`. Terrain PNG conversion encodes AVIF with the web
datadir recipe's terrain settings and requires the external `avifenc` on PATH
(the pinned libavif 1.4.2 / libaom 3.15.0 build from
`scripts/install_pinned_avif_tools.sh`); it is not bundled. AVIF terrain loads
on every platform — the web build decodes it with the browser, native builds
with rav1d. Legacy JPEG XL terrain still loads natively, but the web build
rejects mods that contain JPEG XL assets at admission. Sprite encoding itself
does not require `avifenc`. Use a new destination directory when encoding a mod.

Authored sprite bundles use bounded bitcode inside Zstd: `RHMODVF5` for
families and `RHMODVQ4` for standalone bundles. Both retain the current VQ
sprite codec. Older binary and JSON bundle versions require regeneration
from the source `.rhs.d` directories; renaming their headers is insufficient.
Document decoding limits scratch/collection allocation before expansion,
with separate compressed-byte, Zstd-window, and decoded-frame budgets.

## Hackable scenery-occlusion masks

Normal hackable datadir conversion also writes
`Data/Levels/<name>.rhp.d/masks/000000.png` and `manifest.json` for every level.
PNGs are unscaled, 8-bit grayscale crops: white (255) means scenery covers an
actor; black (0) means uncovered. Add `box_top_left` to a PNG pixel coordinate
to locate it in the map artwork. These masks constrain source-view silhouettes;
they are not complete building segmentation or geometry for hidden surfaces.

The manifest's `index` is the original global `masks` array index, also used in
the six-digit PNG filename. `layer` and `layer_index` resolve patch `old_masks`
and `new_masks` references without renumbering. Bounds, mask type, polylines and
obstacle references are retained. Zero-sized masks have a manifest entry with
`png: null` because PNG cannot store zero dimensions. Paths in each entry are
relative to the manifest. The manifest's `source` points to the level JSON.
When exporting sidecars to a separate output root, `source` is the absolute input
JSON path; in-place and normal conversions use a portable relative path.

Sidecars are derived inspection/projection inputs. Level JSON keeps its original
RLE data and remains the runtime authority; editing a sidecar does not change
game occlusion. To backfill PNGs from an existing hackable datadir in place:

```sh
cargo build -p robin_rs --features tools --bin convert_datadir
target/debug/convert_datadir --input datadirs/fullgame_gog_hackable --output datadirs/fullgame_gog_hackable --mask-pngs-only
```

The incremental command reads all `.rhp.json` files under `Data/Levels` and
overwrites only the derived PNGs and manifests, leaving source JSON and other
datadir assets untouched. Regenerate the sidecars after changing JSON masks.
