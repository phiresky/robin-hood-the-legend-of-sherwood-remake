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

## KILLED agents — session terminated by user; agents are NOT resumable. Respawn fresh.

The whole session (coordinator + subagents) was killed on 2026-08-06 ~14:20. The old session's agent transcripts and task store are gone for practical purposes, BUT everything needed is archived in-repo:

- **`docs/parity-task-archive/*.json`** — full copy of the task store (all 40 tasks with their accumulated per-task analysis in the descriptions). Read these instead of TaskGet; recreate open tasks in the new session's board from them.
- **`docs/parity-task-archive/wip-fix-jump-task20.patch`** (363 lines) — fix-jump's UNCOMMITTED mid-diagnosis work on task #20 (engine/jump.rs + sprite.rs, in worktree agent-a2b48d6a0f1b3ebb0 at stale base 3d42ce0ad). Mostly instrumentation + partial fix; treat as reference/head-start, re-derive on current main. It was last dumping frames 790-800 of its repro to compare Rust step-entry sequencing.
- **`docs/parity-task-archive/wip-fix-ladderpick2-task21.patch`** (47 lines) — fix-ladderpick2's uncommitted start on task #21 (movement.rs + tick.rs, worktree agent-a43843b306107ea87, base = merged main 9423d267a).
- **fix-seekregress (task #36, URGENT)**: worktree agent-ae66cda91de4ba9d5 is CLEAN at base 3d42ce0ad — the bisect produced no commits and no report. Nothing to salvage; restart #36 from the task description (it has all 4 repro traces + suspect list). This is the highest-priority item.
- **fix-corpsewalk (task #39)**: worktree .claude/worktrees/fix-corpsewalk is clean (its #19 work is merged; branch wt-fix-noisetiming). #39's full analysis is in its task JSON: C++ RHElementActorNPC::Noise is fully synchronous — walks every NPC with inline Think(EVENT_HEAR) in the emitting element's slot; Rust posts noise deferred. Observable: officer CallAlert vs noise delivery order. 11 traces.

Old worktrees under .claude/worktrees/ (agent-* and fix-*) whose branches are fully merged can be `git worktree remove --force`d to free disk; keep the two with WIP patches until their tasks land. All merged branch names appear in `git log --oneline --merges` on main.

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

## Recovery checklist for the NEW session (do these in order)

1. Read this file fully. Also load memories (MEMORY.md points here).
2. Recreate the open task board from `docs/parity-task-archive/*.json` (open items: #9, #20, #21, #23, #24, #28, #31, #34–#40; the JSON descriptions carry the accumulated analysis — copy them verbatim into new TaskCreate calls).
3. **First dispatch: task #36 (seek-RNG regression)** — spawn a fresh worktree agent with the task JSON's content; 4 shallow repros make it a fast bisect over `git log --oneline --merges db67cc978..78a792ed6`.
4. Spawn agents for #20 (attach wip patch as head-start), #21 (ditto), #39, and further board items as slots allow. Spawn template: Agent tool with isolation:"worktree"; prompt = symlink setup block (original-code, datadirs, corpora → absolute paths into this repo) + task text + rules (debug builds only; never --release/clippy/filtered cargo; faithful C++ only, no invented guards; validate 2-4 repros + 2-3 short controls; cargo fmt; commit on own branch; report worktree+branch+root cause+validation).
5. After merging a batch: remote release build + freeze runner + **FULL-UNIVERSE re-sweep 14** (all 5,164 minus `parity-save-replays__` schema-11 minus `tmp/regen14-saves.txt` minus the 11 nicouzouf_061 once #37 retires them). Failure-only sweeps are banned until #36 is resolved (regressions on passing traces are invisible to them).
6. Continue the wave loop (previous section) until 100% parity. Do not stop.
