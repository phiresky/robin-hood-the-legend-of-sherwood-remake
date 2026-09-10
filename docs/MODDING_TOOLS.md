# Modding tools

Native GitHub release packages include these command-line tools alongside the
game executable. On Windows, open a terminal in the installed application's
binary directory (or the extracted portable package) and use the `.exe` names.
On Linux, extract the AppImage with `--appimage-extract` to access its bundled
executables. Each tool supports `--help`.

| Tool | Purpose |
| --- | --- |
| `cpf_to_json` | Export a binary CPF profile cache as JSON; preview profile JSON Patches. |
| `encode_mod_sprites` | Encode authored sprite directories and terrain PNGs. |
| `disasm_scb` | Disassemble or decompile compiled mission scripts. |
| `dump_res` | Dump resource archive metadata as JSON. |

Build all four from source:

```sh
cargo build -p robin_modding_tools --bins
target/debug/cpf_to_json --patch-view /path/to/Data/Configuration/profile.cpf profiles.json
target/debug/cpf_to_json --patch-view --patch profiles.patch.json /path/to/Data/Configuration/profile.cpf patched.json
target/debug/encode_mod_sprites SOURCE_MOD NEW_DESTINATION_MOD
target/debug/disasm_scb --decompile /path/to/mission.scb
target/debug/dump_res /path/to/archive.res > resources.json
```

CPF and resource JSON go to stdout when no output file is supplied; diagnostics
go to stderr. See [JSON Patch mods](JSON_PATCH_MODS.md) for profile patch authoring.

The sprite encoder also supports `--family OUTPUT INPUT_RHS_DIR...` and
`--map INPUT_PNG OUTPUT_MAP`. Terrain PNG conversion requires the external
JPEG XL `cjxl` executable on PATH; it is not bundled. Sprite encoding itself
does not require it. Use a new destination directory when encoding a mod.
