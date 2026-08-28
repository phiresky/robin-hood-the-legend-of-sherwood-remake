# HTTP automation server

The desktop-native `robin` binary exposes a loopback-only HTTP endpoint for
debug tools, test harnesses, replay drivers and screenshot pipelines. Android
does not start this HTTP transport. Browser builds use the in-process
`rh_rpc(...)` JavaScript bridge instead. The shared queue and desktop listener
implementation are in
[`http_server.rs`](../crates/robin_rs/src/http_server.rs).

This is a small JSON-over-HTTP API, not a JSON-RPC 2.0 server. Responses are
JSON except for `/screenshot`.

## Starting it

The server binds only to `127.0.0.1`; it has no authentication or TLS. The
default port is 17640.

```text
robin                         # listen on 127.0.0.1:17640
robin --http-server 9999      # choose another port
robin --http-server 0         # disable the server
robin --start-paused          # freeze simulation at frame zero
```

Build before launching a long-running game process:

```bash
cargo build --bin robin
RUST_LOG=debug ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste \
  target/debug/robin --start-paused
```

The listener runs in a dedicated `robin-http-server` thread. It places work on
a queue which the mission loop drains on the game thread. A request waiting on
that queue times out after 60 seconds; use a shorter client timeout when a
blocked or loading game should fail quickly.

## Inspection endpoints

| Method | Path | Result |
| --- | --- | --- |
| `GET` | `/` or `/info` | endpoint discovery document |
| `GET` | `/natives` | every script native with index and signature |
| `GET` | `/state` | compact frame, map and replay status |
| `GET` | `/engine-dump` | complete serialized deterministic Engine state |
| `GET` | `/level-assets` | static level data plus runtime fast-grid flags |
| `GET` | `/host-debug` | non-authoritative host/UI debug state |
| `GET` | `/script` | loaded script classes, functions and instance counts |
| `GET` | `/script/decompile?class=Foo` | TypeScript-like script decompilation; omit `class` for all classes |

`/engine-dump` and `/host-debug` are deliberately separate. The former is
authoritative simulation state; the latter contains local viewport, hover and
trajectory-preview details which do not belong in Engine snapshots.

## Command endpoints

### Script natives

`POST /native` invokes one native. `this` optionally supplies the transient
script receiver.

```json
{"op":"SetMissionWon","args":[],"this":null}
```

`POST /batch` invokes multiple natives back-to-back in one queue drain:

```json
{"calls":[
  {"op":"GetFrame","args":[]},
  {"op":"SetMissionWon","args":[]}
]}
```

Missing required Engine objects fail with a contextual error; the server does
not manufacture default values for malformed calls.

### Console and player commands

`POST /console` accepts the same command string as the in-game debug console:

```json
{"command":"give all"}
```

`POST /command` accepts the externally tagged JSON representation of
[`PlayerCommand`](../crates/robin_engine/src/player_command.rs):

```json
{"SelectPc":{"pc_id":{"Pc":1},"append":false}}
```

The command enters the same deterministic command path as interactive input.

## Screenshots

`GET /screenshot` returns raw `image/png` bytes. Without `frame`, it captures
the next rendered frame; `frame=N` waits until the simulation has reached at
least that absolute frame.

| Query | Meaning |
| --- | --- |
| `frame` | earliest absolute simulation frame to capture |
| `full_map` | capture the complete level at 1:1 map scale |
| `w`, `h` | aspect-preserving output bounds |
| `hide_ui` | omit screen-space HUD drawing; for a viewport capture, also crop the panel area |

Boolean debug-overlay overrides are `view_cones`, `pc_sight`, `motion_graph`,
`surface`, `all_obstacles`, `elevation`, `noise`, `sound_source`, `actor_info`,
`script_zones`, `door`, `projection_areas`, `railroad`, `probability`,
`company_number`, `combat_energy`, `light_zones`, `animation_lines`,
`seek_points`, `fps`, `sprite_masks` and `entity_ids`.

Values accept `1`/`0`, `true`/`false`, `yes`/`no` and `on`/`off`; a bare flag
means true. Overrides affect only the throwaway capture frame and do not mutate
the live `DevState`.

```bash
curl -o shot.png \
  'http://127.0.0.1:17640/screenshot?hide_ui=1&view_cones=1'

curl -o map.png \
  'http://127.0.0.1:17640/screenshot?full_map=1&hide_ui=1&frame=100'

curl -o thumb.png \
  'http://127.0.0.1:17640/screenshot?w=640&h=480'
```

## Timeline control

These endpoints are intended for deterministic automation, especially with
`--start-paused`.

| Method | Path | Body | Behavior |
| --- | --- | --- | --- |
| `POST` | `/step-forward` | `{"n":10}` | run full frame-equivalent ticks |
| `POST` | `/step-back` | `{"n":10}` | restore and replay through rewind history |
| `POST` | `/go-to-frame` | `{"frame":100}` | move forward or backward to an absolute frame |
| `POST` | `/set-paused` | `{"paused":true}` | change the manual-pause flag |

Forward stepping applies recorded commands when replay playback is active and
runs normal rollback/history bookkeeping. Automation steps silently dismiss
pending dialogs, popup scrolls, debriefings, Sherwood reports and pause-all
states so they cannot deadlock the driver. Successful replies include
`modals_dismissed` where applicable.

Backward movement fails if the requested frame predates retained rewind
history. Moving the timeline resets rollback-checker history because the live
Engine is now on a reconstructed timeline.

```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"n":1}' http://127.0.0.1:17640/step-forward

curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"frame":250}' http://127.0.0.1:17640/go-to-frame
```

## Replay handoff

- `GET /get-replay` returns the current recorder's JSONL byte stream from its
  in-memory mirror as `{"content":"..."}`.
- `POST /load-replay` accepts `{"data":"...","paused":true}`. It stages the
  replay for the next mission start; the caller must then trigger that restart.

The same handoff works for native and wasm drivers. Replay schema compatibility
is intentionally limited to the current format.

## Errors

Failures return a 4xx or 5xx response with `{"error":"..."}`:

- `400` for malformed input or a rejected operation;
- `404` for an unknown route;
- `500` if the response channel disappears;
- `504` if the game thread does not process the request within 60 seconds.

For endpoint discovery, prefer `GET /` over copying a route list into external
tools; the discovery document is maintained next to the server dispatcher.
