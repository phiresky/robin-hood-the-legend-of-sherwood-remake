# Parity campaign — full session state dump (2026-08-06, ~14:10 Berlin)

Written when the session ran out of credits (limit resets 18:30 Berlin; earlier reset was 13:30 so it may be a rolling window). This file is the complete resume point — a fresh session should be able to continue the campaign from here with NO other context.

## Standing orders (user directives, all still in force)

- **"keep going until ALL replays are 100% parity. until then do not stop."** Proper general fixes aligned with `original-code/` C++ — no per-trace hacks. See memory `project_parity_campaign`.
- Release builds ONLY on the remote (`ssh atlasbio-robin-cpu4`, repo copy at `~/robinhood/rust-src`); fix agents use DEBUG builds locally.
- Never `/tmp` — use repo-local `tmp/`. Never `git stash`. Never pipe cargo output through filters. No clippy in worktree agents.
- Agents validate ONLY 2-4 key repros + 2-3 short controls in debug; the remote release re-sweep is the wide regression check.
- Consult `docs/SNAPSHOT_INPUT_AUDIT.md` for the determinism/ownership contract.

## Score

- Universe: **5,164 traces** (corpora: `parity-random-save-replays-60s-15x/`, `-schema14/` [1,350 regen], `parity-save-replays-schema12/`, `parity-random-save-replays/`; `parity-save-replays/` = schema-11, permanently retired, 445 traces; retired regen saves listed in `tmp/regen14-saves.txt`).
- Last complete sweep (re-sweep 12, runner db67cc978): **2,390 passing (46.3%)**, 2,418 failing. Ledger entry committed in `docs/ORIGINAL_PARITY_REPLAY.md`.
- Failing-trace depth profile (sampled n=99): traces are ~1,384 frames; failures diverge at mean 630 frames = **mean 46.4% / median 41.4% progress**; 15% of failures get past 80%.
- **Re-sweep 13 DONE** (finished after the dump; results fetched + classified + ledger-committed): 31 passes, 1,948 state-div, 431 rng-panic, 8 timeouts → **cumulative 2,421/5,164 (46.9%)**. Classification: `output/parity-audits/resweep-69df41aeb/classification.json` (top: actor.animation 113, rng:VipIdleRemark 80 [mislabeled swordfight — task #38], direction_goal 79, position_goal_map.x 73, movement_map.x 71, motion_state 70, detectables.length 69).
  - Sweep 13 measured main @69df41aeb (has #12,#14,#18,#26,#29,#32). Merges AFTER it: #33,#27,#25,#30,#19 → need re-sweep 14.
  - **Re-sweep 14 MUST be full-universe (all 5,164)** — failure-only manifests are blind to regressions on passing traces, and we have a confirmed regression (task #36).

## Main branch state

main @ HEAD includes today's merged fixes (all suite-green, `cargo test -p robin_engine --lib` = 2629/0):
- #12 corpse-walk arrival predicate (committed-step + rebuilt-increment dot-product vs `dist<=speed` prediction) — merge of wt-fix-corpsewalk cc3737b28. Six traces to exact EOF.
- #13 (earlier) + #29 seek/stop-transition arms + #32 FaceOpponent determinant-form Angle + #33 seek-refresh queued-effect drop — branch wt-fix-orderlag (5cdc8d639, bccb27806, 94e32d620, 708abb56e).
- #14 ladder-transition pick + #27 ladder-fall flight kinematics — branch worktree-agent-a43843b306107ea87 (91be78873, 349502f46).
- #18 battle_decisions (us-list PCs, per-iteration swap commits, avenger fallback, detection.rs camp bug) — branch worktree-agent-a7bd1cc12fe038a73 (37e5149b1..b50a7b200).
- #25 hero forbidden-expression timer aging moved post-hourglass + set_focus(0)→unfocus — branch worktree-agent-acf2bcdcdeebac468 (ba78cf865).
- #26 test suite fully repaired (2629/0) — branch worktree-agent-a4ead08f9f8668510 (2495ac7a6, 18c810f10, 097eda962). ~176 tests fixed; production fixes: WorldState creation-order map `any_key_map_sized` serde adapter; "command level must stay at 2" was a BORN-BROKEN fixture, not a regression.
- #19 reach-point half: five Think-dispatch sites live-read animation instead of latched installed_order (mpOrder mirror) — deletion fix, branch wt-fix-noisetiming (da65a5097).
- #30 TakeCorpse: invented face-the-corpse snap deleted from begin_carry + FarmerCarry wrongly selecting LittleJohn carried-animation set — branch worktree-agent-ac0fca79adc9b7209 (497eb4ad0, 62ea32191). Merge had one trivial comment conflict in abilities.rs, resolved.

## CRITICAL open item: task #36 regression (URGENT)

Four traces that PASSED at db67cc978 now fail on HEAD with seek-RNG draw-count storms:
- Cyrdach_001 replay-009 (f1653, SeekPointSelection storm, draws 5251..5813)
- Nescafe_001 replay-009 (f1589, same)
- linux2/Profile_002/Savegame_000/replay-014 (f5379, SeekPointAcceptance)
- linux2/Profile_002/Continue/replay-013 (f7581, SeekPointAcceptance)
All confirmed pre-existing on HEAD by two independent A/B tests (not caused by #25 or #30). Introduced somewhere in db67cc978..78a792ed6. Prime suspects: #29 PerformSeek else-arm refresh, #33 seek-refresh effect drop, #12 arrival predicate. Agent fix-seekregress was bisecting (worktree agent-ae66cda91de4ba9d5) — status unknown at dump time, may have died to the credit limit before reporting.

## Interrupted agents (all hit the session credit limit; resume via SendMessage or fresh spawn)

All are harness-created worktrees `.claude/worktrees/agent-<id>/` with symlinks (original-code, datadirs, corpora) already set up, on branches `worktree-agent-<id>`:

1. **fix-jump (task #20)** — worktree agent-a2b48d6a0f1b3ebb0. Jump-up spurious step entry (f872 family) + flight-increment rounding. Task description contains fix-orderlag's detailed scoping: C++ ReadyForTakeOff computes increment ONCE = (goal3D−pos3D)/uwFramesOfFlight; Rust re-normalises per frame at engine/jump.rs:1870-1899 (advance_airborne_flight); uwFramesOfFlight is per-RHflightstyle — FALL style uses (myZ−goalZ)/FALL_SPEED (Rust's JumpingDown 20.0 = FALL_SPEED), others use animation duration (CurrentStepState.total_frames at jump.rs:1498 is the analogue). Last seen: dumping frames 790-800 of its repro to see Rust sequencing (mid-diagnosis of the f872 step-entry half, no commit yet).
2. **fix-ladderpick2 (task #21)** — worktree agent-a43843b306107ea87 (same tree that landed #14+#27). Small elevation residue (~11 traces). Was just starting: had been told to `git merge main` then work the elevation groups from resweep-db67cc978 classification. Watch for: map.y = world.y − z, INVERSE_ASPECT_RATIO 1.743, f32/f64 boundaries.
3. **fix-seekregress (task #36)** — worktree agent-ae66cda91de4ba9d5. THE URGENT ONE (see above). Was bisecting db67cc978..78a792ed6 with the 4 repros; also received the 2 extra linux2 repros mid-flight.
4. **fix-corpsewalk (task #39)** — worktree `.claude/worktrees/fix-corpsewalk` (MANUALLY created, branch wt-fix-noisetiming; this agent is teammate-style, reachable by name). Heard-steps noise family (11 traces). Its own analysis: C++ RHElementActorNPC::Noise is fully synchronous — walks every NPC, inline Think(EVENT_HEAR) in the emitting element's slot; Rust posts noise through a deferred path. Officer CallAlert vs noise delivery ordering is the observable. Was about to merge main + re-derive membership from resweep-db67cc978.
- fix-endfacing (task #30) — DONE and merged, worktree agent-ac0fca79adc9b7209 disposable.
- fix-orderlag — retired cleanly; branch wt-fix-orderlag fully merged.

## Task board (open items)

- #9 RefreshArrowProtection Reserve residual (f794/f762)
- #20 jump-up step entry + flight increment (fix-jump, interrupted mid-diagnosis)
- #21 small elevation residue (fix-ladderpick2, interrupted at start)
- #23 SeekArea one-fewer-pair geometry (13+) — requeued, unowned
- #24 get_sector_screen associated-sector resolution (wide blast radius)
- #28 strike resolution one frame early (f19137) + CombatObserveSideStep misattribution (8)
- #31 360-gate LOS/detection us-list composition (2 repros @f33630/f13778, all-soldier membership, relates to detection.detectables.length cluster 68)
- #34 tolerance-arrival + StrangleCmd post-seek Moving stamp (nicouzouf/S069 r005 f457) — check if survives sweep first
- #35 LeaveSpy/LeaveTree priority NotYetSet latent hazard
- #36 URGENT seek-RNG regression bisect (fix-seekregress, interrupted)
- #37 exclude/re-record 11 nicouzouf_061 old-schema traces (lack hero_refused_action records — unfixable as recorded; add to retired list or re-record with schema-14 recorder, original-code commit d494273)
- #38 swordfight RNG divergence — the "rng:VipIdleRemark" cluster (80) is MISLABELED (classifier keys on first RNG site of frame); real mismatches are UpdateSwordfightDistance (offsets 1807002/1807253), ReceiveSwordDamage (1746896/1747024), EvaluateSwordfight (1814391), FindPositionForTableSwordfight (~1804390), SwordDamageProtection. Offsets symbolized via addr2line on original-code/build/native-full/robin.
- #39 heard-steps noise delivery (fix-corpsewalk, interrupted at start)
- #40 The16thFrame periodic path live-read animation audit (tick_periodic_ai_for_npc_with_animation, tick_npc_post_detection_tail_for_npc) — fix if sweep shows bored-reroll/16th-frame family

Standing clusters not yet tasked: actor.animation 133 (top cluster, resweep-db67cc978 classification), direction_goal 76, position_goal_map.x 72, motion_state 70, MoveOk→MoveWaiting 57, BoredAnimationChoice 56, AiRandomValueRectangle 55, path_events.length 54, RuntimeBuildingExitWait 51, visibility_queries.length 45 (+43 ai.list_them etc.). Full breakdown: `output/parity-audits/resweep-db67cc978/classification.json`.

## Wave loop (the process to continue)

1. Merge reported agent branches into main (verify suite 2629/0 + debug build).
2. When agents free up / finish: assign next task from board, or spawn fresh (`Agent` tool, `isolation: "worktree"`, prompt template: symlink setup + TaskGet + debug-only + narrow validation + faithful-C++ rules — copy any recent spawn prompt from this file's history).
3. After a batch of merges: rsync source to remote, release-build, freeze runner as `original_parity_replay-<commit>`, make manifest, launch 11-shard tmux sweep (`scripts/run_parity_release_sweep.sh CORPUS AUDIT RUNNER SHARD SHARDS`, wrapper `run-parity-*.sh` pattern), watcher on SWEEP-DONE, rsync back, classify, update ledger, dispatch.
4. **Next sweep (14) must be FULL-UNIVERSE** — build manifest from ALL 5,164 minus schema-11 (`parity-save-replays__`) minus `tmp/regen14-saves.txt` retirees minus the 11 nicouzouf_061 (#37, once retired). The passing set must be re-verified because of the #36 regression.

## Key mechanics reference (hard-won, do not relearn)

- Remote sweep: `ssh atlasbio-robin-cpu4`; corpora + datadirs + original-code + rust-src at `~/robinhood/`; build `cd ~/robinhood/rust-src && cargo build --release --example original_parity_replay`; sweep script reads `<audit>/traces.snapshot`, writes per-trace `status/<key>.status` (0 pass, 1 state-div, 101 rng-panic, 124 timeout) and `logs/<key>.log`; key = trace path with `/`→`__`.
- classify_parity_failures.py groups by first divergence boundary; output classification.json.
- Substates in dumps: C++ enum = Rust index + 1. Aim previews burn creation-order in Original, not in Rust (see memory project_creation_order_preview_gaps).
- RNG contract: Rust consumes recorded draw VALUES; "replay exhausted" = Rust drew MORE than Original; extra/missing draws → compare gate conditions at the named site.
- `--dump-jsonl` ~500KB/frame — narrow windows only, delete after.
- Entity mapping in dumps: original pc:N ↔ Rust Pc(PcId(N−1)) sometimes (creation-order shift) — verify entity identity before trusting a dump diff (bit fix-endfacing twice).
- Debug replays under multi-agent load: 25-85 min for 15k+ frame traces; timeout 1800-3600s.
- The Bash tool auto-spills long output; never filter cargo.

## Credit-limit protocol

When agents die with "You've hit your session limit": wait for reset time, then SendMessage each named agent ("Session limit has reset — resume <task> where you left off") — agents resume from transcripts. Unnamed/failed spawns: re-spawn fresh with the task-board context (task descriptions carry the accumulated analysis). Previous reset (13:30) revived the whole fleet successfully this way.
