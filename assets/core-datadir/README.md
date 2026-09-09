# Core overlay datadir

Always-on engine overlay, loaded before `mods/` and taking precedence over the
primary datadir.

- `Data/AudioDurations.json`: required English sample durations and speech-group
  identities for simulation, replay, and ranked verification. Local audio lengths
  are used only by playback; the mixer plays the selected language naturally.
- `Data/Interface/Fonts/`: original Latin bitmap fonts and role configuration
  from the demo, restoring fonts missing from Steam installs; `arial.ttf`
  supplies the TrueType list-widget font.
- `Data/Interface/UI/`: portrait, pin, stance, patrol, and formation PNGs.

Native packages validate `core-overlay-manifest.json` against the complete
`Data/` tree. When changing assets, update the sorted manifest (paths, sizes,
and SHA-256 hashes) and the required-path list in
[`core_overlay.rs`](../../crates/robin_rs/src/core_overlay.rs). Missing, extra,
symlinked, or corrupt entries fail validation.

Browser builds preload the audio timing table, `arial.ttf`, and the UI PNGs
they use, listed in their generated `preload-assets.json`.

## Regenerating audio timing

Build the Rust generator separately, then pass the core output directory and
English `Sounds` directories in precedence order. The first occurrence of a
sample or speech group wins; later roots supply additional demo-only samples.
Include both the base sounds (actor definitions, effects) and English voice
directory. The GOG English release uses `2047`, despite that folder number.

```sh
cargo build -p robin_rs --example generate_audio_durations
target/debug/examples/generate_audio_durations assets/core-datadir \
  /path/to/fullgame_gog/DATA/Sounds \
  /path/to/fullgame_gog/2047/data/Sounds \
  /path/to/demo_leicester_ecoste/DATA/Sounds \
  /path/to/demo_leicester_ecoste/1033/data/Sounds
```

The generator reads original WAV/OGG headers, resolves every English actor
variant, rejects unresolved durations, and updates the core inventory hash.
It never reads the retired per-user cache. Supply English inputs explicitly;
folder numbers alone do not prove a recording's language.

Durations are stored in milliseconds under lowercase, relative sample paths.
Simulation rounds up to 40 ms frames, with a one-frame minimum. Explicit
speech variants use their own English duration; random variants retain the
longest English variant duration regardless of presentation RNG. Empty
authored speech groups remain empty, not fabricated zero-length recordings.

Changing timing requires regenerating official content projections and using
the matching engine build for replays. Ranked manifests identify this authority
as `core_audio_durations_v1`; their sound-duration component binds the mission's
canonical durations and variant identities. The isolated verifier compiles this
same JSON file into its build because its runtime filesystem contains only
retail data; it never derives timing from that retail audio. Missing files, invalid schemas,
missing required profiles, or missing required sample entries fail explicitly.
The old `~/.local/share/robin_hood/cache/audio_durations.json` is ignored and may
be removed by the user.
