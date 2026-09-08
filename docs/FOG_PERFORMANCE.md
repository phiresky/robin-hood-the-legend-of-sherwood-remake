# Fog performance investigation (2026-09-07)

The fog history uses polygon unions, not a cell bitmap. Boolean operations are
provided by `geo` / `i_overlay`. Repeated floating overlays followed by f32
storage produced microscopic holes along shared edges. On the Lincoln doorway
route, the original accumulator reached 29,709 projected-history vertices after
66 live frames, with 7,103 holes (7,063 smaller than 0.01 square map pixels).
History accumulation could take over 100 ms, versus about 22 ms for sight itself.

Accumulation now calls `i_overlay` on a fixed 1/256-map-pixel coordinate scale.
Its output is exactly representable in the existing f32 storage for maps up to
65,536 pixels. No polygon area filter, raster representation, or change to sight
distance / camera occlusion is used. There is no save or replay format change.
Unchanged current regions are already explored, so stationary scans skip history
unions and preserve the mask generation (and therefore the uploaded texture).

## Reproduction

Build separately, then run an isolated fullgame Lincoln instance:

```sh
cargo build --profile parity --bin robin -p robin_rs
RUST_LOG=info,fog_perf=trace,present_perf=trace ROBINHOOD_DATA_DIR=datadirs/fullgame_gog timeout 600s target/parity/robin --mission H01_Lin_VL --proto Lincoln --start-paused --http-server 17648 --no-sound
```

From another terminal, starting with this fresh paused mission:

```sh
uv run --no-project scripts/benchmark_fog_history.py
```

The script drives four 60-frame segments using the HTTP step endpoint. This
measures simulation throughput, **not displayed FPS**; it does not render each
stepped frame. Default rollback checking is not disabled.

During development, a double-precision floating-overlay trial still accumulated
52,722 projected-history vertices at frame 61 and 37,765 at frame 241. The
fixed-scale version had 1,373 and 3,366 respectively on the same fixed-frame route.
Its four segments took 1.98, 2.51, 2.75, and 2.32 seconds. Stationary scans dropped
below 1 ms; moving scans generally spent 2–4 ms after source computation.
These are local optimized `parity`-profile observations, not release FPS claims.

Linux `perf record -e cycles:u -F 99 -g -p <game-pid> -o <file> -- sleep 20`
confirmed that polygon splitting/intersection was the main CPU consumer. Inspect
with `perf report -i <file> --stdio --no-children`.

## Remaining presentation bottleneck

Live measurements in this session were additionally limited by swapchain image
acquisition, including while paused with no fog uploads. Phase traces showed
178–193 ms in `get_current_texture`, with submission around 0.5 ms and presentation
around 0.1 ms. This is separate from fog-history accumulation; the cause of that
swapchain wait is not established. Do not interpret step throughput as fixing it.

The `fog_perf` and `present_perf` trace targets are opt-in and do not flood the
normal DEBUG replay log. Geometry tests cover history stability, subpixel holes,
bridge-top camera occlusion, wall facades, entity visibility, and serialization.
