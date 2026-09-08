# Post-refactor live multiplayer validation

Source snapshot: `3e6beaa17`, 2026-09-07. Status: **not passing**. Actual two-process gameplay and rollback work in the exercised demo; two production panics prevent a successful end-to-end reconnect result. No production code was changed for this validation.

## Scope and isolation

Built separately with `CARGO_BUILD_JOBS=1 cargo build --locked -p robin_rs --bin robin --features multiplayer` (passed, 9m04s). Default features do not enable multiplayer. Binary SHA-256: `ff355368ce7c8898674efbf2f72c333e4bde7306a366979819f975cf8df766f5`.

Both peers are actual `robin` processes, using production iroh QUIC, admission, complete engine snapshots, deterministic seat commands, script execution, and rollback. Graphical runs use two real X11 game windows under Xvfb; synthetic Return dismisses their actual briefing panels. Additional runs use the production `--headless` driver. These are not mocked transport/unit tests. Commands are submitted through the production HTTP command endpoint and enter the multiplayer command path.

Every run uses a new loopback-only user/network namespace. Explicit endpoint addresses connect the peers directly over localhost. The production endpoint's automatic public discovery attempts fail inside this network namespace; no deployed service, public relay, matchmaking mutation, or public replay upload is reachable. The harness refuses any namespace containing an interface other than `lo`. Per-peer working, save, cache, configuration, replay, and identity directories live under the dedicated evidence directory. Existing player saves and identities were not used or changed. Audio is disabled.

Game data: `datadirs/demo_leicester_ecoste`, actual scripted `Dem_Lei_MP` / Leicester. Both processes load the mission, 74 soldiers, 18 civilians, and four controllable PCs. This is a bounded smoke exercise, not a completed mission or manual UX/playability review.

## Reproduction and retained evidence

Evidence root: `/tmp/robin-multiplayer-validation-OUy6DR2y`. Each child directory contains unfiltered host/peer logs, event timestamps, summary JSON, actual engine dumps, runtime roots, and host replay artifacts. Private identity files stay in those runtime roots and must not be published. The final harness additionally preserves its exact source as `driver.py` for each subsequent run.

Before worktree cleanup, the exact tested executable was retained at `/tmp/robin-multiplayer-validation-OUy6DR2y/robin-3e6beaa17-multiplayer`. Its SHA-256 was verified after copying and matches `ff355368ce7c8898674efbf2f72c333e4bde7306a366979819f975cf8df766f5`. Use this absolute path for `--binary` when reproducing after the worktree is removed.

From this worktree, after the separate build:

```sh
timeout --signal=TERM --kill-after=15s 600s unshare --user --map-root-user --net \
  python3 scripts/validation/multiplayer_live.py \
  --binary target/debug/robin \
  --data /home/phire/robinhood/datadirs/demo_leicester_ecoste \
  --evidence /tmp/NEW-UNUSED-MULTIPLAYER-EVIDENCE-DIRECTORY
```

Add `--headless` for the production headless path. The evidence path must not already exist. Add `--restart-process` only to diagnose the separate process-identity limitation below. Add `--observe-hashes-before-reconnect` to require a post-bootstrap periodic comparison before resynchronization (the bounded `headless-03` observation). Dependencies are Python 3, `unshare`, `ip`, and (for graphical runs) Xvfb/libX11. A bounded alarm and child cleanup prevent orphan games. No Cargo output was filtered and the default worktree `target/` was retained.

## Established live behavior

- `graphical-03` and `graphical-04`: both actual windows finish initialization, admit two authenticated seats, dismiss the briefing, and advance simulation. The initial peer adopts the host frame-zero snapshot; logged snapshot and adopted hashes match exactly.
- Both seats successfully send `SetLockAlt`; the peer sends `SelectAllPcs`, `AssignQuickGroup { index: 0 }`, and `CrouchDown`. Host engine dumps confirm both seat flags and peer selection/group membership containing real PC IDs 198, 199, 201, 200.
- Late inputs cause actual rollback/replay in the peer. For example, `headless-02/peer.log` reports replay of frames 30 through 50 (21 frames) after a late input, with the production recent-timeline-history path. Brief host process suspension adds scheduling disturbance; some rollback already occurs naturally because the peer runs ahead, so it is not attributed exclusively to that injection.
- `headless-02`: synchronized host stepping advances from frame 31 to 35, closes the existing peer connection, and triggers automatic reconnection. The same process/transport identity reclaims `PlayerId(1)`, receives the real host snapshot at frame 35, adopts it, and receives a new BeginSim. The host then panics; continued post-reconnect input and seat-state preservation are therefore **not established**.

## Blocking findings

### P1: graphical synchronized stepping overwrites an uncommitted normal frame

`graphical-04/host.log` records a fatal panic after `POST /step-forward` with `{"n":4,"synchronized_multiplayer":true}`: `timeline commands must be recorded contiguously: frame 24, expected 23`. The HTTP caller receives a disconnected response; this is not a diagnostic timeout.

The stack is `TimelineHistory::commit_frame_input` (`crates/robin_engine/src/sim_timeline.rs:468`) → `RewindBuffer::end_frame_input` → `run_forward_ticks` (`crates/robin_rs/src/game_session/tick.rs:653`) → `drain_steps` → `InteractiveFrameSimulation::drive_manual_steps`.

Read-only diagnosis: `frame_simulate.rs:585` marks normal history commit pending, but calls `drive_manual_steps` at line 587 before the later `commit_simulation_history` in `flow.rs:435-436` (or the terminal paths at `frame_simulate.rs:1243/1297`). The step starts another rewind transaction and overwrites the pending pre-tick frame before the ordinary frame is journaled. The contiguity invariant correctly detects the missing frame. The method comment still says manual stepping occurs after the normal history commit. This ordering also deserves non-multiplayer interactive stepping coverage; the observed reproduction is multiplayer.

History attribution: **pre-existing at refactor baseline `a413e4c80`**, by source comparison, not a rebuilt historical binary. Baseline `frame_simulate.rs:624/626` and the later `flow.rs` commit already have this order; baseline `robin_rs/src/sim_timeline.rs:472-474` already has the same contiguous validation. The refactor moved that implementation into `robin_engine`; it did not introduce the assertion. The deferred commit/manual-step ordering predates the current pass (`71e289e06`, then complete-frame adaptation `f6a42b7079`).

Minimal fix direction: defer manual steps until the current normal frame, including its modal/post-frame contributions, is finalized and committed; keep the pre-tick checkpoint paired with its own complete frame input. Simply committing early before modal contributions, clearing history, or suppressing validation would weaken replay correctness. Add the transaction-order test below before moving the call.

Required regression: run an unpaused interactive normal tick plus queued synchronized/manual forward step in the same host frame; assert all complete frame inputs are journaled once, in order, before comparing replay reconstruction with the resulting live state. Do not remove the contiguity assertion.

### P1: host rejects BeginSim after its own successful resynchronization

`headless-02/host.log:114` records `invalid multiplayer admission ordering: state Running, event BeginSim { frame: 35, ... }`, panicking at `crates/robin_rs/src/game_session/runtime.rs:1095`. This follows an accepted synchronized host step, actual reconnect, snapshot transfer, peer readiness, and a new ready-barrier completion. The runtime is still `Running` when the legitimate replacement BeginSim arrives.

The peer log confirms `client reconnected new_seat=PlayerId(1)` and adoption of snapshot 35 from local timeline frame 52. Thus transport and snapshot adoption succeed, but the host admission lifecycle does not survive the cycle. This is distinct from the graphical pending-history crash: the headless manual step itself succeeds.

History attribution: **pre-existing at `a413e4c80`**, by source comparison. `git blame` attributes the strict admission transition table to `473beb6dc` ("Implement headless multiplayer admission barrier"); the baseline contains the identical host-only initial BeginSim case and panic. No historical binary run was needed or performed. This observation is specifically the synchronized-automation full-snapshot cycle; it is not evidence that an unrelated natural reconnect has identical host behavior.

Minimal fix direction: the host-side authoritative resynchronization operation should reset its runtime admission state to an explicit waiting-for-Begin state with the adopted frame/epoch boundary, alongside resetting the transport barrier. Do not indiscriminately permit `Running + BeginSim`, and do not misuse the peer-only waiting-for-snapshot state for a host that already owns the snapshot.

Required regression: running host → authoritative manual timeline adoption → full peer resynchronization → readiness barrier → new BeginSim → both peers advance and accept new input. Preserve strict admission checks; explicitly model the host's resynchronization transition instead of accepting arbitrary duplicate BeginSim events.

### P2/documentation: native process restart is not a durable seat reconnect

`headless-01` and `graphical-03` terminate the peer and launch a replacement with the same isolated save directory. The new process gets a different transport endpoint identity and is rejected because the session already has its configured two players. It cannot reclaim the parked seat merely by retaining the durable ranking identity or nickname.

This matches the explicit current implementation in `crates/robin_rs/src/multiplayer/native.rs:99`: the transport key is a process-held `OnceLock<SecretKey>`, never persisted; the durable game seed is a separate ranking identity. It is therefore a documented implementation boundary, not proof that the supported same-process reconnect is broken. `docs/MULTIPLAYER.md:155` still describes a durable per-install iroh owner key, and line 236 describes nickname as reconnect identity; those claims should be reconciled. Decide whether cross-process seat recovery is a product requirement before changing identity ownership.

## Run ledger and remaining boundaries

| Run | Result |
| --- | --- |
| graphical-01 | Real admission succeeded; initial diagnostic sampled before scheduled start/ConnectSeat and incorrectly expected two fully applied seats. Harness corrected. |
| graphical-02 | Real admission succeeded; mission briefing kept simulation at frame zero. Harness corrected to dismiss actual X11 briefing. |
| graphical-03 | Real graphical gameplay, both-seat commands, and rollback; process restart rejected as a new identity. |
| graphical-04 | Real graphical gameplay and rollback; synchronized stepping crashes on missing journal frame. |
| headless-01 | Real headless gameplay and rollback; process restart rejected as a new identity. |
| headless-02 | Real headless gameplay, rollback, automatic transport reconnect and snapshot admission; host crashes on new BeginSim. |
| headless-03 | Further 90-second pre-reconnect observation; host hash frames reach 400, peer schedule samples pass frame 400, but only frame-zero hash comparison is logged. No resynchronization request is made in this run. |

The hash observation is a coverage gap, not proof of divergence. `process_pre_tick_state_hash` (`frame_prepare.rs:320`) only compares a received host hash when its frame exactly equals the current local frame, then drops older received hashes. The observed peer stays ahead of host schedules, so late host hashes can go uncompared. A follow-up should compare an appropriately retained exact historical peer hash at the same authoritative frame (including rollback invalidation), or explicitly report missed comparisons; absence of DESYNC alone is not validation. This function also predates the refactor baseline.

No passing end-to-end claim is made. Initial frame-zero hash agreement is not sufficient evidence of sustained post-bootstrap determinism. TODO: re-run both graphical and headless complete scenarios after the two production defects are fixed. Successful post-reconnect gameplay, retained selection/groups after snapshot, longer network loss/reordering, WAN/public-relay behavior, browser/native mixed peers, and mission completion remain unverified. WAN/public-relay validation needs a separately authorized environment; this diagnostic deliberately cannot contact one. The existing relay script would require a reachable relay and is not substituted for the local actual-client evidence. Recorded host artifacts are retained for replay inspection, but this report does not claim offline replay verification or that replay alone reproduces the host-only HTTP transaction panic.
