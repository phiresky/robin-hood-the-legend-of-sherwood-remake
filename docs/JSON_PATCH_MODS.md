# JSON Patch mods

Mods can edit decoded game data with standard [JSON Patch (RFC 6902)](https://www.rfc-editor.org/rfc/rfc6902).
Each patch is an array of `add`, `remove`, `replace`, `copy`, `move`, or `test`
operations. This is not JSON Merge Patch. Place files beneath `mods/<your-mod>/`:

| File | Document being patched |
| --- | --- |
| `Data/Configuration/profiles.patch.json` | The canonical `Data/Configuration/profile.cpf.json` document; the binary CPF loader produces the identical schema |
| `Data/Levels/<mission>.level.patch.json` | Decoded `LoadedLevel`: `proto`, `mission`, and `diplomacy` |
| `Data/Levels/<mission>.descriptors.patch.json` | Decoded `LevelDescriptors`: resource references, pictures, dialogues, and custom text arrays |

These patch actual data before runtime construction, for both binary levels
and levels authored as `.level.json`. The level patch targets the expanded
data, not the smaller hackable authoring schema. Arbitrary JSON files, sprite
files, scripts, and already running sessions are not patch targets.

## Profiles

For a catalog with a unique `Knight03` filename:

```json
[
  { "op": "test", "path": "/soldiers/Knight03/filename", "value": "Knight03" },
  { "op": "replace", "path": "/soldiers/Knight03/life_point", "value": 150 },
  { "op": "copy", "from": "/soldiers/Knight03", "path": "/soldiers/Knight00" },
  { "op": "replace", "path": "/soldiers/Knight00/filename", "value": "Knight00" },
  { "op": "replace", "path": "/soldiers/Knight00/display_name", "value": "Blue Cavalier" }
]
```

Any serialized profile field can be edited: combat skills, hostility,
equipment, action arrays, voice-bank IDs, mission parameters, etc. Copying
includes the complete source profile. Required sprites/resources must exist.

The on-disk profile JSON is the patch target. There is no separate patch view.
The exporter creates exact, case-sensitive filename keys; missions use
`mission_filename`. Duplicate filenames become `filename#<original index>`.
Authored keys remain intact through all patch layers, even when a profile's
filename changes. Set the copied profile's filename to the intended asset name.
JSON Pointer escapes `/` as `~1` and `~` as `~0`. Weapon tables retain numeric
IDs: `/hth_weapons/<index>` and `/bows/<index>`.

The document also contains `character_order`, `soldier_order`, `civilian_order`,
and `mission_order`: arrays of keys preserving the original numeric profile IDs.
For example, `"soldier_order": ["Archer00", "Knight03"]` assigns slots 0 and 1
regardless of the order of properties in `soldiers`. A key may occur only once
in its order list and must exist in the matching map.

Patches cannot remove existing profiles or remove/reorder the existing order
lists, because compiled missions reference their numeric slots. New profiles
may be appended to an order list; any unlisted keys append in sorted key order
after all patch layers. Character `index` is absent from authored JSON and is
assigned from this ordering by the loader, including for copies.

Dump your base CPF's exact patch keys, fields and enum values:

```sh
cargo build -p robin_rs --example cpf_to_json
target/debug/examples/cpf_to_json /absolute/path/Data/Configuration/profile.cpf target/profile.cpf.json
```

The exporter and `convert_datadir` produce this canonical format by default.
Old array-based profile JSON is rejected with instructions to regenerate the
hackable datadir or re-export just its profile JSON from the original CPF.
No migrator or special export flag is needed. Runtime/save serialization of
`ProfileManager` is internal and is not the authored file format.

Ordinary JSON Patch tools can patch the exported file directly:

```sh
jsonpatch target/profile.cpf.json mods/my-mod/Data/Configuration/profiles.patch.json > target/patched-profile.cpf.json
target/debug/examples/cpf_to_json target/patched-profile.cpf.json target/validated-profile.cpf.json
```

Pass `--patch path/to/profiles.patch.json` to preview and validate an actual
patch with the Rust loader before exporting. Repeat `--patch` in mod load order
to preview their composition. JSON input is accepted as well as binary CPF.
Patches operate before mission-derived beam-me metadata is populated for loose
datadirs; those derived fields should be changed through mission data instead.

## Mission data

For a mission with an existing initial beam-me entry:

```json
[
  { "op": "replace", "path": "/mission/beam_mes/0/profile_override", "value": 1 },
  { "op": "replace", "path": "/mission/beam_mes/0/robin_role", "value": true }
]
```

The schema is `LoadedLevel` in `crates/robin_engine/src/level_data.rs`.
Mission actors, rescue slots, and other references still use actual numeric
IDs. Related prisoner targets and script references are not updated
automatically: patch each affected field. Use `test` to assert assumptions
about the base mission before changing it.

## Text and descriptors

```json
[
  { "op": "replace", "path": "/custom_popup_texts", "value": ["Find a way into the castle."] },
  { "op": "replace", "path": "/custom_short_briefings", "value": ["Rescue the prisoner."] }
]
```

These are zero-based arrays. `null` retains original resource text;
`/custom_popup_texts/-` appends. Indexed `replace` needs an existing element;
`add` cannot skip beyond the array's end. Replacing an entire array replaces
earlier mods' overrides, so prefer individual edits when indices are known.

`custom_dialogue_texts` is an array of nullable sentence arrays. Each non-null
entry must match an existing dialogue's `portrait_ids` count. Other serialized
`LevelDescriptors` fields are also patchable; see `crates/robin_assets/src/res_descr.rs`.

## Composition and limits

The base datadir/VFS patch is applied first, then each directory or ZIP overlay
in mount order. Desktop `mods/` directories mount in sorted order. Later
operations see earlier results. Patch files compose instead of shadowing.
Failed operations/tests, missing paths, unknown data fields and invalid types
abort loading with the path and layer number. No partially patched generic
document is installed. Missing optional patch files are normal.

The old `soldier-profiles.patch.json`, `.characters.patch.json` and
`.text.patch.json` formats have been removed. Their presence is a loading
error with migration guidance. Rename files and express their final data
changes as standard operations; template objects are not accepted.

The sprite importer emits this format too. After exporting the base catalog:

```sh
uv run scripts/import_fabri18_sprites.py --profiles-only --profile-catalog target/profile.cpf.json
uv run scripts/validate_sprite_mods.py --profile-catalog target/profile.cpf.json mods/fabri18-sprite-gallery mods/mounted-knight-colours
```

Both scripts declare dependencies inline; no requirements file or manual
environment setup is needed.

JSON Patch has no selectors, arithmetic, loops, or scripting. “Increase all
guards' health by 20%” and the old `progression_from` formula must be expanded
into concrete operations by an authoring tool. New AI systems or actions still
require scripting or engine changes.

Deserialization checks field types, enums and fixed array lengths. Additional
checks protect existing named profile slots, assign character indices, bound
soldier capacities to 0–100, and validate dialogue override lengths. Normal
engine construction checks still apply. TODO: unify comprehensive
cross-reference validation with the legacy loaders; type correctness alone
does not prove every weapon ID, resource, script reference, or geometry
relationship is valid.
