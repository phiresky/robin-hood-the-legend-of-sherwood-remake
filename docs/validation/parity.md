# Broader authentic parity validation

## Snapshot and scope

Source snapshot: `3e6beaa17ace3a8d90b47ed7416f00bf0e6a29ef`. A fresh headless runner was built using `CARGO_BUILD_JOBS=1 cargo build -p robin_parity --bin original_parity_replay` (passed). The older frozen runner was not reused because relevant protocol sources differ.

Read-only inventory found 41,845 native traces (25,118,257,467 bytes), all with readable native-v68 extent footers, across 14 capture groups. Footer readability is not an EOF/gameplay verdict.

The reproducible subset contains 256 traces, 91 distinct save/session-family identities, and 329,519 recorded frames. It includes all 18 interactive traces, every trace from the four tiny replacement/recapture groups, and 25–26 representatives per remaining campaign. Selection alternates heavy/median compressed-size representatives across stable-hash-ordered save identities; dispatch is round-robin across capture groups. The longest interactive trace has 6,020 frames.

Coverage caveat: campaign/save diversity and compressed size are proxies for mission/event richness. This diagnostic inventory does not decode distinct mission IDs or enumerate actual event types. The sample is not exhaustive corpus coverage.

## Campaign

- Evidence root: `/tmp/robin-validation-parity-3e6beaa17-v2`.
- Two concurrent workers, 1,800 seconds per case, eight-hour campaign ceiling.
- Expected duration before measurements: roughly four to eight hours; early results will refine this estimate.
- Each case uses a copied trace and one frozen runner, requires the structured exact-EOF protocol and matching trace/runner digests and full extents, and writes an atomic JSON result plus raw log.
- New isolated SQLite ledger and per-trace SHA256 locks; no existing campaigns, databases, watcher processes, or source corpus artifacts are changed.
- The controller owns dispatch independently of conversational polling. `launch.json` records command, namespace PID, script/validator/manifest/runner digests, datadir and launch time. `progress.json` is refreshed every wait cycle. `campaign-result.json` is written at completion or budget exhaustion.
- Classifications distinguish exact EOF, recorded divergence, runner/setup/evidence errors, timeouts, and cases not started before the budget. Error subcategories and raw logs remain available; errors are not relabeled gameplay divergences.

Diagnostic script: `scripts/validation/parity_sweep.py`. Its six fixture-free tests passed with `python3 scripts/validation/parity_sweep_test.py`. The additional controller test verifies the two-worker bound, final timeout/unrun-budget accounting and absence of pending/running ledger states after budget exhaustion; the live frozen harness was not changed.

Current-snapshot `CARGO_BUILD_JOBS=1 cargo test -p robin_parity --lib` also passed all 151 tests while the campaign used its independent frozen executable.

## Status

Launched at **2026-09-07 10:36:29 UTC**, using the committed `4522c118f` diagnostic harness. Execution session `61896`; controller PID `2` is the sandbox namespace PID, not a globally addressable host PID. The controller is a `nohup` process with persistent output, owns both worker dispatch loops, and does not require conversational polling. Its copied harness and all runtime inputs are outside the worktree.

- Runner SHA256: `1ba15c4eb47767780687c6eccfe8e36f400f281f06e949c214121a8663679bad`.
- Manifest SHA256: `002b623ee223396cfdc0001a8ca3c6b039e22b91a8bd3016632f761f526826b2`.
- Status: `jq . /tmp/robin-validation-parity-3e6beaa17-v2/progress.json`.
- Controller output: `/tmp/robin-validation-parity-3e6beaa17-v2/controller.log`.
- Exact launch command and additional digests: `launch.json` in the evidence root.

This is running, not yet a passing campaign result. Final counts, duration, identities, and findings will be appended when available.

First checkpoint (~6.6 minutes): three cases reached exact EOF (375-frame no-input, 750-frame random input, 1,500-frame schema12), with zero divergences/errors/timeouts. These heavy representatives measured roughly five recorded frames/second/worker; naive extrapolation exceeds eight hours, so the budget may leave explicitly counted unrun cases. Later lighter representatives may change that projection.

Second checkpoint (~30 minutes): **16/256 exact EOF, zero divergences, zero errors, zero timeouts**, with 24,770 matched frames. All 14 capture groups have at least one completed case, and completed results report actual trace schemas 12, 14, 15 and 16. The longest 6,020-frame interactive recording passed. Observed worker time now extrapolates to roughly 6.4 hours total, but this is not a guarantee and the eight-hour ceiling is unchanged.

Main advanced independently during execution (including `db83e5e9b`). This campaign remains pinned to `3e6beaa17`; its results do not validate that later main snapshot.

Third checkpoint (~62 minutes): **32/256 exact EOF, zero divergences, zero errors, zero timeouts**. The second long interactive case (4,446 frames) remains in progress and is significantly slower than the first; it has not yet exceeded its 30-minute case timeout. The campaign remains active with the same frozen runner and budget.

## External interruption and verified resume

Session `61896` unexpectedly exited 143 (SIGTERM) after **63 exact EOF results, zero divergences/errors/timeouts**. The final heartbeat was at about 116 minutes. The cause is unknown: neither the controller nor either coordinating agent requested termination. A separate build also ended with 143. The campaign lock was released, and there was no final campaign report or Python traceback. This is an external controller interruption, not evidence of a gameplay divergence or case timeout.

Resume began at **2026-09-07 12:47:19 UTC**, session `57098`, namespace PID `2`. The original deadline remains **2026-09-07 18:36:29 UTC**, including downtime. All 63 completed results were verified and retained without rerunning; the two interrupted cases (indices 62 and 64) were queued again before the 191 pending cases. Their partial logs were moved into `interruptions/<timestamp>/` with separate resume metadata. The 4,446-frame interactive trace mentioned above completed successfully in 1,514.93 seconds before interruption.

The resumed controller verifies the original manifest, frozen executable, validator, configuration, copied traces, completed result/log digests and EOF identities before changing any ledger state. It can recover an atomic result written before its ledger update. It does not replace `launch.json` or reset the campaign budget. Nine diagnostic tests pass, including retained completed results, archived interrupted attempts, expired-budget resume, and rejection of modified evidence. `cargo fmt --check` passes.

The original launch harness remains untouched. The separately copied resume harness has SHA256 `383227891359ddb70f557c2734f2b34ce7bd0ee2eea9b0804832fbba9703c1e7`; the frozen runner and manifest hashes above are unchanged. Current controller output is `controller-resume.log`; status remains `progress.json`. Reproducible resume command:

```sh
nohup python3 /tmp/robin-validation-parity-3e6beaa17-v2/harness-resume/validation/parity_sweep.py run \
  --resume --output /tmp/robin-validation-parity-3e6beaa17-v2 \
  --runner /tmp/robin-validation-parity-3e6beaa17-v2/original_parity_replay \
  --datadir /home/phire/robinhood/datadirs/fullgame_linux \
  --workers 2 --timeout 1800 --hours 8 \
  > /tmp/robin-validation-parity-3e6beaa17-v2/controller-resume.log 2>&1
```

Do not launch another controller while the campaign lock is held, or reuse this log redirection for a second resume: retain each attempt's controller output separately. `nohup` does not protect against SIGTERM; persistent files and verified resume provide recovery, not immunity to external process termination. The campaign remains in progress, not yet a passing 256-case result.

Post-resume checkpoint (~143 minutes since original launch, including downtime): **72/256 exact EOF**, with **103,943 matched frames** and zero divergences/errors/timeouts. Both interrupted cases passed their retried attempts. Completed cases span all 14 capture groups and schemas 12, 14, 15 and 16. This remains a partial result against the pinned `3e6beaa17` snapshot.

## Final status: interrupted, incomplete, deadline elapsed

The read-only closing inspection at **2026-09-07 20:38 UTC** found **199/256 exact EOF results**, representing **256,284 matched frames**, with **zero recorded divergences, errors, or timeouts**. All 18 selected interactive traces passed. The ledger retains **55 pending cases and two stale `running` cases**, indices **199 and 200**, whose attempts were interrupted without producing final result records. These two attempts are not classified as gameplay failures or timeouts.

The last controller heartbeat was **2026-09-07 15:13:14 UTC**, about 4 hours 37 minutes after the original launch. Execution session `57098` is no longer available; the campaign lock and both interrupted-case locks are released. No `campaign-result.json` was produced. The second interruption's cause and exit signal are unavailable; unlike the earlier observed exit 143, no particular signal is established for this interruption. Namespace-scoped process inspection cannot independently exclude host-wide orphan processes, but there is no remaining owned execution session or evidence of active controller/worker jobs.

The original **18:36:29 UTC** deadline has elapsed. The campaign was **not resumed or extended**, and it did **not complete all 256 selected cases**. Prior running-status checkpoints above are historical, not the current status. This partial validation applies only to **`3e6beaa17ace3a8d90b47ed7416f00bf0e6a29ef`**, not subsequent main-branch or architecture changes.

Closing verification confirmed all **199 completed log digests** and the unchanged runner and manifest SHA256 values recorded above. Original manifests, copied traces, frozen executable, launch/resume metadata, interrupted-attempt logs, atomic results, SQLite ledger and controller output remain at **`/tmp/robin-validation-parity-3e6beaa17-v2`**, outside the worktree. The closing inspection and this documentation update made no campaign writes, resumed no jobs, and performed no builds. The validation task can be closed; retain that external evidence when removing the completed worktree later.
