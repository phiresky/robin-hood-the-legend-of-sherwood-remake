# Core overlay datadir

Always-on engine overlay, loaded before `mods/` and taking precedence over the
primary datadir.

- `Data/Interface/Fonts/`: original Latin bitmap fonts and role configuration
  from the demo, restoring fonts missing from Steam installs; `arial.ttf`
  supplies the TrueType list-widget font.
- `Data/Interface/UI/`: portrait, pin, stance, patrol, and formation PNGs.

Native packages validate `core-overlay-manifest.json` against the complete
`Data/` tree. When changing assets, update the sorted manifest (paths, sizes,
and SHA-256 hashes) and the required-path list in
[`core_overlay.rs`](../../crates/robin_rs/src/core_overlay.rs). Missing, extra,
symlinked, or corrupt entries fail validation.

Browser builds preload only `arial.ttf` and the UI PNGs they use, listed in
their generated `preload-assets.json`.
