# Multiplayer runtime repair

Baseline: `ecfdfbb10`. Original live failure evidence and exact old executable remain under `/tmp/robin-multiplayer-validation-OUy6DR2y`. New acceptance artifacts use `/tmp/robin-multiplayer-repair-qyUh8ZdY`.

## Changes

- An explicitly initiated host snapshot resynchronization enters `HostWaitingForResyncBegin { snapshot_frame }`. Only a BeginSim for that exact adopted frame can open the replacement start gate. Unsolicited BeginSim while Running and mismatched snapshot frames still panic as invalid admission ordering. The manual-step owner calls the new hook at its final adopted boundary; the host late-input-horizon resynchronization emits the equivalent internal event and republishes host readiness, which the transport reset clears.
- A published host replacement remains frozen while peers rejoin. Queued old-generation inputs are logged and discarded rather than modifying it through late rollback or future commands; pending future inputs and inputs already accumulated before the graphical second drain are discarded at that explicit adoption boundary. Repeated obsolete ingress cannot restart an already pending barrier. A genuinely new resynchronization after BeginSim but before its future start time explicitly rearms the gate.
- Peers retain the first pre-tick state hash at each existing `STATE_HASH_INTERVAL` boundary (25 frames). Received host hashes are compared against that exact historical frame, even when the local simulation has advanced beyond it. No full-engine hash is computed on intervening frames, and repeated paused presentations reuse the first sample rather than changing its meaning or cost.
- Hash retention is bounded to 256 cadence entries (approximately 6,400 simulation frames). A late input at frame F invalidates local hashes strictly after F, preserving the unaffected pre-F state; whole-state replacement/disconnection clears the local generation. Invalidated/expired comparisons emit `multiplayer hash comparison missed` rather than silently disappearing. TODO: reconstruct missing historical hashes during rollback if stronger comparison coverage is required; a missed comparison is explicitly not an agreement.
- Native client transport identity remains intentionally process-local. Documentation now distinguishes same-process seat recovery from the durable ranking identity and nickname display label.
- The diagnostic harness records an explicit source revision and requires a positive periodic hash comparison after the replacement snapshot frame. It separately reports missed comparisons; no absence-of-DESYNC-only pass is accepted.

The hash work adds one full state hash per 25 peer simulation frames, not one per render/frame-loop iteration. Host cadence and wire/hash bytes are unchanged. Both peers still load the exact production engine snapshot and scripted demo; no fixtures are used for the live acceptance run.

## Verification

Production source tested: `762225be1`, including the coordinated stepping repairs `4f2d07b4e`, `2b026ff50`, and `d5aaed8f9` (local cherry-picks `fae498376`, `a33484c35`, `762225be1`). Later changes to this report/harness do not change that executable.

Passed:

- `CARGO_BUILD_JOBS=1 cargo build --locked -p robin_rs --bin robin --features multiplayer`.
- `CARGO_BUILD_JOBS=1 cargo test --locked -p robin_rs --features multiplayer --lib game_session::runtime::tests::`: **38 passed**.
- `CARGO_BUILD_JOBS=1 cargo test --locked -p robin_rs --features multiplayer --lib game_session::`: **185 passed**, including strict admission, obsolete ingress batches, published snapshot freezing, graphical second-drain input discard, and normal/manual complete-frame recording tests.
- Actual two graphical processes (`graphical-02`): **passed**, 88 seconds. Host steps 56→60, peer reconnects and adopts snapshot 60 from local frame 92, both retain the peer's four-PC selection and quick group, and peer SetLockAlt/StandUp commands are submitted after reconnect. Host confirms the new flag by frame 70; full-state hash agreement is logged at frame 75 afterward. Hash matches: 0, 50, 75; two real late-input rollbacks; no DESYNC. A rollback from frame 24 explicitly invalidates hash 25, producing one missed-comparison warning rather than a fabricated match (`peer.log:322`).
- Actual two headless processes (`headless-01`): **passed** the original repaired harness, 66 seconds. Host steps 44→48, peer adopts snapshot 48 from local frame 75, retained selection/group and new input are confirmed, with hash matches 0, 25, 50 and two actual rollbacks; no DESYNC or missed comparisons. Frame 50 proves post-reconnect agreement but precedes the follow-up input, so a stronger final headless check is recorded separately below.

The successful graphical frame-75 comparison is visible in `graphical-02/peer.log:354`; matching follows both replacement adoption and follow-up input. No result relies solely on absence of DESYNC. The deliberately missing frame-25 comparison remains an explicit coverage limit, not a pass.

The stronger `headless-02` check **passed** in 49 seconds on the same executable: synchronized step 45→49; exact replacement snapshot admission; retained selection/group and post-reconnect input; observed post-input host boundary 64; then matching full-state hash 75. Matches were 0, 25, 50, 75, with two real rollbacks, zero DESYNC, and zero missed comparisons. Its summary explicitly records `post_input_hash_agreement: true`. Together with the graphical frame-75 evidence, both production drivers continued deterministically beyond the repaired reconnect and follow-up commands.

## Reproduction and retained installation

The exact executable is retained read-only at `/tmp/robin-multiplayer-repair-qyUh8ZdY/robin-762225be1-multiplayer`, SHA-256 `0173cc5c57b6403f63139fbbb1294fc97d4b711d25d18a6f2db737496cb5163b`. Exact committed `assets/` and `mods/` directories were copied beside it. This matters: the initial `graphical-01` artifact launch stopped during startup because a standalone copied executable cannot resolve `assets/core-datadir` from a fresh working directory. Staging its required installation resources fixed that harness packaging error; no production workaround was introduced.

From the repository containing the committed harness:

```sh
timeout --signal=TERM --kill-after=15s 600s unshare --user --map-root-user --net \
  python3 scripts/validation/multiplayer_live.py \
  --binary /tmp/robin-multiplayer-repair-qyUh8ZdY/robin-762225be1-multiplayer \
  --snapshot 762225be1 \
  --data /home/phire/robinhood/datadirs/demo_leicester_ecoste \
  --evidence /tmp/NEW-UNUSED-MULTIPLAYER-REPAIR-RUN \
  --observe-hashes-before-reconnect
```

Add `--headless` for the second production driver. Build and run remain separate, `CARGO_BUILD_JOBS=1`, default worktree target, with no Cargo output filtering or target redirection. Each run has a fresh loopback-only namespace and isolated per-peer save, identity, cache, and working directories. Browser/public relay/WAN paths and cross-process native identity recovery are not claimed by this smoke test. Private identity files inside the evidence runtime roots must not be published. Logs, engine dumps, event transcripts, exact harness copies, summaries, and host replays are retained under the evidence root.

## Final combined repair acceptance

Root integration `83f87b1d1` merged cleanly into this branch as `185ea5783905d4ce2cb470a2525e64883275106e`. That committed combined source was rebuilt successfully with the same multiplayer build command (1m29s warm rebuild). Its retained read-only executable is `/tmp/robin-multiplayer-repair-qyUh8ZdY/robin-185ea5783-multiplayer`, SHA-256 `4a9df31e4ae89c56d6ba965369b514dd34a328d6dc64b9a31a15966a6a110814`, using the same staged sibling assets/mods. This exact executable was also supplied to the standalone stepping/replay validation owner.

Both final runs used the strongest harness, requiring a hash match **strictly after the observed post-input host frame**, and passed:

- `final-graphical` (64 seconds): host step 57→61; real peer reconnect and replacement snapshot admission; preserved four-PC selection/quick group; follow-up peer input committed. Post-input host boundary 72, then hash agreement at 75. Hash matches 0/50/75, two real rollbacks, zero DESYNC, one explicitly missed comparison after rollback.
- `final-headless` (56 seconds): host step 45→49; the same reconnect, retained-seat, and follow-up-input checks. Post-input host boundary 63, then hash agreement at 75. Hash matches 0/25/50/75, two real rollbacks, zero DESYNC, zero missed comparisons.

Use the reproduction command above with the final executable and `--snapshot 185ea5783905d4ce2cb470a2525e64883275106e` to repeat this combined-source check. Evidence lives in the corresponding `final-*` directories. All child processes were cleaned up when the bounded harnesses completed; no additional production change was required after integration.
