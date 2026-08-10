# External parity wave: 10 parallel lanes

You are the coordinator for a second machine working in parallel on the Robin Hood parity campaign. Start from commit `81970c018` (or a descendant that contains it). Read `paste`, `docs/PARITY_CAMPAIGN_STATE.md`, `docs/ORIGINAL_PARITY_REPLAY.md`, and the relevant JSON files under `docs/parity-task-archive/` before acting.

Spawn ten agents at once, each in its own Git worktree and branch (`external-parity-01` through `external-parity-10`). Never let two agents edit the same worktree. Never use `git stash`. Do not use `/tmp`; put small diagnostics under the lane worktree's `.codex-tmp/<lane>/`, and avoid multi-gigabyte dumps. Preserve all unrelated changes. Consult `./original-code` for every behavioral conclusion. Use debug or release/parity builds based on total turnaround time. Run `cargo fmt`, focused tests, `git diff --check`, and the representative replay before committing. Commit each completed fix independently and report the branch plus full commit hash. If a candidate is stale, already fixed, source-inconsistent, or not yet source-proven, make no speculative production change.

Each lane should inspect its assigned partition, choose the shortest currently failing trace whose first divergence is not excluded below, prove the first causal boundary, implement the narrow source-faithful fix, validate it, and commit it. After a representative reaches exact EOF or advances to an independent frontier, the lane may take another trace from its own partition.

## Ten lane assignments

1. `external-parity-01`: normal/60s `Savegame_nicouzouf`, save numbers 001–012.
2. `external-parity-02`: normal/60s `Savegame_nicouzouf`, save numbers 013–024.
3. `external-parity-03`: normal/60s `Savegame_nicouzouf`, save numbers 025–038.
4. `external-parity-04`: normal/60s `Savegame_nicouzouf`, save numbers 039–051.
5. `external-parity-05`: normal/60s `Savegame_nicouzouf`, save numbers 052 and above.
6. `external-parity-06`: normal/60s `Savegame_SuN1Sh1nE`, save numbers 001–012.
7. `external-parity-07`: normal/60s `Savegame_SuN1Sh1nE`, save numbers 013–024.
8. `external-parity-08`: normal/60s `Savegame_SuN1Sh1nE`, save numbers 025 and above.
9. `external-parity-09`: schema-14 Cyrdach and Nescafe corpora. Treat the two Task 36 controls as controls, not candidates.
10. `external-parity-10`: remaining linux2/linux3/base/schema-12 corpora, including Continue/Restart traces not covered above.

Prefer completed sweep evidence from the frozen `original_parity_replay-e1669842d`/`81970c018` lineage. Re-run a candidate on the lane's current build before changing code: an old sweep failure may already be cleared by the baseline.

## Exclusions: already owned by the primary machine

Do not work on these representatives or their exact behavioral families unless your trace independently demonstrates a different source call site:

- SuN `Savegame_032/replay-015`: stopped-short point seek incorrectly launches post-seek DropAle (Task 271).
- SuN `Savegame_034/replay-002`: EVENT_VIEW retarget needs a live table/jump line (Task 273).
- nicouzouf `Savegame_039/replay-001` 60s frontier: stretched `SquareDistance` in `AttackingApproachingNewEnemy` (Task 274).
- nicouzouf `Savegame_041/replay-015`: bow release execute-initialization validity (Task 277).
- nicouzouf `Savegame_020/replay-006`: retained shield obstacle geometry/per-projectile refresh (Task 278).
- SuN `Savegame_036/replay-005`: reactive counterstrike D/E boredom/save mapping (Task 282).
- linux3 `Savegame_046/replay-002`: active-shot equip must restore selected Bow action (Task 283).
- linux3 `Savegame_046/replay-008`: NPC movement request WalkingWithShield rewrite (Task 284).
- nicouzouf `Savegame_065/replay-015`: SetAlwaysAttentive must inspect view alert status (Task 285).
- SuN `Savegame_021/replay-005`: ladder/wall fall must not publish LayerGoal (Task 286).
- SuN `Savegame_016/replay-006`: repeated lethal piercing while already in coma (Task 287).
- nicouzouf `Savegame_020/replay-002`: terminal sword movement retains MovingSword (Task 288).
- SuN `Savegame_038/replay-001`: projectile hole classification must be scoped to terminal obstacle material (Task 289).
- nicouzouf `Savegame_022/replay-009` and `replay-015`: attentive-mode opposite requests/FIFO (Task 290).
- SuN `Savegame_036/replay-014`: Strangle input admission frontier (Task 291; still under source audit here).
- nicouzouf `Savegame_051/replay-015`: nested StopAll must precede WakeUp launch (Task 292).
- nicouzouf `Savegame_065/replay-008`: projectile layer/sector capture overlaps source-inconsistent Task 89 and is being retired (Task 293).
- SuN `Savegame_024/replay-010`: special-strike proposal rejects all candidates after synchronized RNG (Task 295).
- SuN `Savegame_036/replay-004`: newly constructed Bonus old-position is Original uninitialized memory; retired, never emulate it (Task 297).
- nicouzouf `Savegame_045/replay-003`: redundant RaiseShield must not overwrite an earlier Start motion latch (Task 299).
- linux2 `Profile_002/Continue/replay-008`: ordinary RaiseShield eagerly sets Upright (Task 302).

Also exclude all traces listed in `docs/PARITY_RETIRED_TRACES.txt` and all completed task families in `docs/parity-task-archive/`.

## Required evidence for each lane

Before editing, report:

1. Exact trace path, first divergent frame, entity, and all first-frame differing fields.
2. Whether the current baseline reproduces it.
3. Original source file/line and Rust owner boundary.
4. Why it is distinct from archived tasks, retired traces, merged wave commits, and the exclusions above.
5. The smallest faithful fix and at least one control that prevents overgeneralization.

After editing, report:

1. Files changed and focused regression names.
2. `cargo fmt` and `git diff --check` results.
3. Focused test results and representative replay outcome.
4. Any control replays run.
5. Branch name and full commit hash.
6. New independent frontier if the replay did not reach exact EOF.

Do not merge branches into one another. The primary machine will cherry-pick or merge each reviewed commit in dependency order.
