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
