# Gameplay frame profiling

The release game has an opt-in, low-overhead host-frame profiler. It is off by
default; the disabled path does not read clocks or mutate counters.

Enable periodic 120-frame aggregates with:

```sh
ROBIN_GAMEPLAY_PROFILE=1 \
RUST_LOG='warn,robin_rs::game_session::frame_perf=info,robin_engine::engine::tick::perf=info,robin_engine::engine::tick::phase_perf=info' \
ROBINHOOD_DATA_DIR=datadirs/fullgame_linux \
target/release/robin [mission arguments]
```

The host aggregate separates preparation, simulation/modal handling,
recording, audio/effect dispatch, rendering/presentation, PostInitialize, and
frame pacing. `total` includes pacing, so compare `total` with the individual
phases before treating a normal frame-rate sleep as a regression. The engine
targets provide the nested `perform_hourglass` total and its deterministic
simulation phases, including `entity_systems`.

Add `robin_engine::engine::tick::entity_system_perf=info` to `RUST_LOG` for
the nested owner-walk breakdown and call counts. This is intentionally a
separate target because timing every owner callback is more intrusive than the
coarse phase profiler.

For CPU sampling, first build separately and then run `perf` against that exact
binary. Avoid profiling asset-loading startup by delaying event collection:

```sh
cargo build --release --bin robin
perf record -D 6000 -e cycles:u -F 999 -o target/york-perf.data -- \
  env ROBIN_GAMEPLAY_PROFILE=1 ROBINHOOD_DATA_DIR=datadirs/fullgame_linux \
  target/release/robin --custom-mission \
  datadirs/mods/defending-the-york-city/2025-01-07.zip \
  --mission Str03_Yor_MK
```

Use `--fast-forward` when measuring simulation throughput. Without it, the
`pacing` bucket intentionally contains the sleep used to maintain game speed.
