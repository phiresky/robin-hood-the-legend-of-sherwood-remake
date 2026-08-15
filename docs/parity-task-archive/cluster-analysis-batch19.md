# Parity failure cluster analysis - 226 failing traces

Source data: `output/parity-audits/batch19-head07872e94-nestedd7792d55-preflight/traces.snapshot` (failing set) joined with `output/parity-audits/batch15-combined-validation-preflight/outcome-ledger.tsv` (frontiers), enriched from `batch15 .../logs/` and the in-flight `batch19 .../logs/` (fresh logs spot-checked identical to batch15 frontiers). All 226 traces assigned; 154 ordinary_mismatch + 72 rng_mismatch ("Rust consumed RNG draws N..M" = rust drew extra at the listed sites).

Summary of task sizes: T01=31, T02=21, T03=9, T04=22, T05=12, T06=13, T07=16, T08=15, T09=9, T10=6, T11=10, T12=13, T13=5, T14=4; leftovers/near-clusters=40.


## Task 1: direction_goal rotated by 4 sectors (90 deg) for stationary guards (31 traces)

**Hypothesis.** A facing-direction computation for stationary/guarding actors classifies its vector into the wrong 16-sector quadrant: in 19 of these traces `(rust - original) mod 16 == 12` (one trace is +4), i.e. an exact 90-degree rotation, always on soldiers/civilians standing at a post and choosing which way to face (not while walking). The most likely culprit is one call path that feeds a map-space vector into an iso-space sector classifier (or vice versa), or swaps/negates an axis before calling it. `crates/robin_engine/src/position_interface.rs` has both `vector_to_sector_0_to_15` (screen convention: (0,-1)->0, (1,0)->4) and `vector_to_sector_0_to_15_iso` (applies ASPECT_RATIO); mixing them yields exactly this class of error. The `Wait vs Turn` animation traces are the same bug seen one frame later: rust computes a rotated `direction_goal`, decides the actor is not facing its goal, and issues a `Turn` (anim 140->50, 0->2, 5->50) that the original never issues (and inversely Civilian 50->4 where the original turns but rust doesn't). Note repetition of single entities across replays (Soldier106 in linux3/P3 Savegame_000 x10, Soldier255 in Savegame_040 x4, Soldier240 x3, Civilian73 x3) - a per-entity post/guard orientation, deterministic per save.
**Likely source.** `crates/robin_engine/src/position_interface.rs` (`vector_to_sector_0_to_15{,_iso,_with_aspect}`), `crates/robin_engine/src/ai_enemy/substate_handlers.rs` + `crates/robin_engine/src/engine/movement.rs` `set_direction_goal` call sites (esp. the post/guard facing ones), `crates/robin_engine/src/ai/macro_patrol.rs`; original: `original-code/RHpositioninterface.cpp` (GetDirectionVector / sector-from-vector), `original-code/RHelementactorsoldier.cpp`, `original-code/RHartificialintelligence.cpp` (guard-post facing).

**Representative repros:**

- `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_067/replay-001-session-0001.jsonl.zst` - first divergence f483 - Soldier(SoldierId(51)).actor.animation: original=5 rust=50
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_067/replay-013-session-0001.jsonl.zst` - first divergence f925 - Soldier(SoldierId(51)).actor.animation: original=5 rust=50
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_071/replay-003-session-0001.jsonl.zst` - first divergence f5275 - Soldier(SoldierId(104)).actor.animation: original=140 rust=50
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_040/replay-015-session-0001.jsonl.zst` - first divergence f27765 - Soldier(SoldierId(255)).direction_goal: original=13 rust=9

**All members:**

- f483 `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_067/replay-001-session-0001.jsonl.zst` - Soldier(SoldierId(51)).actor.animation: original=5 rust=50
- f925 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_067/replay-013-session-0001.jsonl.zst` - Soldier(SoldierId(51)).actor.animation: original=5 rust=50
- f1413 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_022/replay-012-session-0001.jsonl.zst` - Civilian(CivilianId(60)).actor.animation: original=50 rust=4
- f5275 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_071/replay-003-session-0001.jsonl.zst` - Soldier(SoldierId(104)).actor.animation: original=140 rust=50
- f5392 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_071/replay-015-session-0001.jsonl.zst` - Pc(PcId(342)).direction_goal: original=3 rust=7
- f9239 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-015-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f9385 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-004-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f9436 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-010-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f9456 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-012-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f9457 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f9572 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_003/Savegame_000/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f9700 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f9917 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-005-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f10391 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=3 rust=15
- f10498 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_000/replay-001-session-0001.jsonl.zst` - Soldier(SoldierId(106)).direction_goal: original=11 rust=7
- f10511 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_032/replay-006-session-0001.jsonl.zst` - Soldier(SoldierId(87)).actor.animation: original=140 rust=50
- f14449 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_051/replay-004-session-0001.jsonl.zst` - Soldier(SoldierId(144)).direction_goal: original=8 rust=4
- f17593 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_008/replay-013-session-0001.jsonl.zst` - Soldier(SoldierId(110)).direction_goal: original=2 rust=14
- f27765 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_040/replay-015-session-0001.jsonl.zst` - Soldier(SoldierId(255)).direction_goal: original=13 rust=9
- f27966 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_040/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(255)).direction_goal: original=11 rust=7
- f28530 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_040/replay-010-session-0001.jsonl.zst` - Soldier(SoldierId(255)).direction_goal: original=12 rust=8
- f28702 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_040/replay-001-session-0001.jsonl.zst` - Soldier(SoldierId(255)).direction_goal: original=11 rust=7
- f31211 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_030/replay-006-session-0001.jsonl.zst` - Soldier(SoldierId(240)).actor.animation: original=140 rust=50
- f31391 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_030/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(240)).actor.animation: original=140 rust=50
- f31714 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_030/replay-005-session-0001.jsonl.zst` - Soldier(SoldierId(240)).actor.animation: original=140 rust=50
- f33699 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_024/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(65)).direction_goal: original=4 rust=0
- f37149 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_055/replay-012-session-0001.jsonl.zst` - Soldier(SoldierId(176)).actor.animation: original=0 rust=2
- f37163 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_055/replay-001-session-0001.jsonl.zst` - Soldier(SoldierId(182)).actor.animation: original=0 rust=2
- f54862 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_038/replay-011-session-0001.jsonl.zst` - Civilian(CivilianId(73)).direction_goal: original=2 rust=14
- f55147 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_038/replay-004-session-0001.jsonl.zst` - Civilian(CivilianId(73)).direction_goal: original=13 rust=9
- f55309 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_038/replay-015-session-0001.jsonl.zst` - Civilian(CivilianId(73)).direction_goal: original=13 rust=9

## Task 2: direction off-by-one sector at boundary while walking (21 traces)

**Hypothesis.** `direction` (and sometimes `direction_goal`) is off by exactly +-1 sector, always while the entity is moving (`direction,sprite_row` co-mismatch = walking sprite row derives from direction). This is an angle-to-sector rounding/tie-break at a sector boundary: original and rust classify a movement vector lying near a 22.5-degree boundary into adjacent sectors. Suspects: the aspect-ratio application order in `vector_to_sector_0_to_15_with_aspect`, a `+0.1` radian nudge like the one removed in task #295, float precision of atan2 vs the original's table/comparison-based classifier, or `TurnAntiVibration` hysteresis (cf. completed task #545) letting rust settle one sector early/late. Values cluster at boundaries (8 vs 7/9, 14 vs 15, 1 vs 2, 0 vs 15).
**Likely source.** `crates/robin_engine/src/position_interface.rs` sector classification; turn/anti-vibration logic in `crates/robin_engine/src/engine/movement.rs`; original: `original-code/RHpositioninterface.cpp`, `original-code/RHelementactor.cpp` (Turn / TurnAntiVibration).

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_071/replay-015-session-0001.jsonl.zst` - first divergence f564 - Soldier(SoldierId(114)).direction_goal: original=10 rust=11
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_010/replay-014-session-0001.jsonl.zst` - first divergence f1030 - Soldier(SoldierId(61)).direction: original=14 rust=15
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_046/replay-006-session-0001.jsonl.zst` - first divergence f1575 - Soldier(SoldierId(60)).direction: original=0 rust=15
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_030/replay-008-session-0001.jsonl.zst` - first divergence f6319 - Soldier(SoldierId(53)).direction: original=8 rust=7

**All members:**

- f564 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_071/replay-015-session-0001.jsonl.zst` - Soldier(SoldierId(114)).direction_goal: original=10 rust=11
- f1030 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_010/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(61)).direction: original=14 rust=15
- f1158 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_026/replay-006-session-0001.jsonl.zst` - Soldier(SoldierId(82)).direction: original=5 rust=4
- f1249 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_014/replay-007-session-0001.jsonl.zst` - Pc(PcId(80)).direction_goal: original=0 rust=2
- f1329 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_014/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(62)).direction: original=14 rust=13
- f1390 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_045/replay-008-session-0001.jsonl.zst` - Soldier(SoldierId(61)).direction: original=15 rust=14
- f1451 `parity-save-replays/60s-random-input/schema14/traces/Savegame_nicouzouf/Profile_001/Savegame_057/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(61)).direction: original=14 rust=15
- f1575 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_046/replay-006-session-0001.jsonl.zst` - Soldier(SoldierId(60)).direction: original=0 rust=15
- f1843 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_046/replay-012-session-0001.jsonl.zst` - Soldier(SoldierId(61)).direction: original=1 rust=2
- f6319 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_030/replay-008-session-0001.jsonl.zst` - Soldier(SoldierId(53)).direction: original=8 rust=7
- f7127 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_030/replay-007-session-0001.jsonl.zst` - Soldier(SoldierId(52)).direction: original=8 rust=9
- f10594 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_017/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(180)).direction: original=8 rust=9
- f10950 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_039/replay-003-session-0001.jsonl.zst` - Soldier(SoldierId(138)).direction_goal: original=7 rust=6
- f16899 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_032/replay-010-session-0001.jsonl.zst` - Soldier(SoldierId(52)).direction: original=2 rust=1
- f19540 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_034/replay-012-session-0001.jsonl.zst` - Pc(PcId(180)).direction: original=2 rust=1
- f24481 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_033/replay-009-session-0001.jsonl.zst` - Soldier(SoldierId(53)).direction: original=6 rust=5
- f32859 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_010/replay-007-session-0001.jsonl.zst` - Pc(PcId(172)).direction: original=1 rust=2
- f35320 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_034/replay-005-session-0001.jsonl.zst` - Soldier(SoldierId(65)).direction_goal: original=7 rust=8
- f35842 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_010/replay-015-session-0001.jsonl.zst` - Soldier(SoldierId(134)).direction: original=2 rust=1
- f36715 `parity-save-replays/60s-random-input/schema14/traces/Savegame_SuN1Sh1nE/Profile_004/ExQuickSave/replay-003-session-0001.jsonl.zst` - Soldier(SoldierId(97)).direction_goal: original=3 rust=1
- f39208 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_035/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(70)).direction: original=9 rust=8

## Task 3: PC sword-strike initiation gate (thrust vs move) (9 traces)

**Hypothesis.** In a swordfight, the original PC commits `SwordstrikeThrustA` (anim 67) while rust instead keeps closing distance (`MoveOk`, anim 304) - six identical (67 vs 304) cases across four save families - plus one inverse-flavored case (76 vs 67, ParrySword vs Thrust) and two rng-side twins where rust draws an extra `SwordStrikeSelection` the original never draws (rust strikes when the original doesn't). So the strike-eligibility test (opponent-in-range / reachability / angle window before rolling strike selection) disagrees in both directions, pointing at a boundary condition in the melee strike gate (range or facing tolerance), not a missing feature. The `15-no-input` Savegame_039 trace is a minimal input-free repro (frame 244, rng draw 1965).
**Likely source.** `crates/robin_engine/src/engine/melee/strikes.rs`, `evaluate.rs`, `dispatch.rs`; sim-rng site `SwordStrikeSelection` in `crates/robin_engine/src/sim_rng.rs`; original: `original-code/RHartificialmalignity.cpp` (strike selection/eligibility), `original-code/RHelementactorpc.cpp`.

**Representative repros:**

- `parity-save-replays/15-no-input/traces/Savegame_nicouzouf/Profile_001/Savegame_039-session-0001.jsonl.zst` - first divergence f244 - rng sites: SwordStrikeSelection
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_037/replay-008-session-0001.jsonl.zst` - first divergence f654 - Pc(PcId(78)).actor.animation: original=67 rust=304
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/ExQuickSave/replay-005-session-0001.jsonl.zst` - first divergence f1542 - Pc(PcId(282)).actor.animation: original=67 rust=304
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Continue/replay-010-session-0001.jsonl.zst` - first divergence f1749 - Pc(PcId(173)).actor.animation: original=67 rust=304

**All members:**

- f244 `parity-save-replays/15-no-input/traces/Savegame_nicouzouf/Profile_001/Savegame_039-session-0001.jsonl.zst` - rng:SwordStrikeSelection
- f654 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_037/replay-008-session-0001.jsonl.zst` - Pc(PcId(78)).actor.animation: original=67 rust=304
- f1230 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_010/replay-003-session-0001.jsonl.zst` - Pc(PcId(103)).actor.animation: original=67 rust=304
- f1454 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_055/replay-003-session-0001.jsonl.zst` - rng:SwordStrikeSelection,MeleeStepBack,SmalltalkStrikeSide
- f1542 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/ExQuickSave/replay-005-session-0001.jsonl.zst` - Pc(PcId(282)).actor.animation: original=67 rust=304
- f1749 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Continue/replay-010-session-0001.jsonl.zst` - Pc(PcId(173)).actor.animation: original=67 rust=304
- f6289 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_000/replay-006-session-0001.jsonl.zst` - Pc(PcId(126)).actor.animation: original=67 rust=304
- f16553 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_004/replay-003-session-0001.jsonl.zst` - Pc(PcId(344)).actor.animation: original=76 rust=67
- f24487 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_033/replay-006-session-0001.jsonl.zst` - Pc(PcId(168)).actor.animation: original=67 rust=304

## Task 4: early-battle melee micro-gate frontier (SuN1Sh1nE Savegame_024 + nicouzouf Savegame_039) (22 traces)

**Hypothesis.** Two saves that both open in an ongoing multi-combat battle; every replay of them diverges within the first ~1500 frames on melee bookkeeping RNG gates (DrunkCombatFreeze, MeleeNonMutualGate, MeleeInitiative, DefaultPostLook, MacroRand, SwordDamageProtection, BowAccuracy, SmalltalkStrikeSide) - rust consumes extra draws at these sites. These are the classic downstream signature of one combat participant making a slightly different micro-decision (who freezes, who repositions, who gets initiative) a few frames earlier. SuN1Sh1nE/Savegame_024 is the historically hottest save (tasks #295/#345/#360/#549 all advanced it); its current frontier is f207 (replay-011). Recommend attacking the two earliest frontiers (f207 Savegame_024, f231 Savegame_039) with the rng-draw diff and letting the rest of the family ride. Plausibly shares a root with Task 3 (both saves also produced Task 3 members).
**Likely source.** `crates/robin_engine/src/engine/melee/` (mod, dispatch, swordfight), `crates/robin_engine/src/ai_enemy/battle.rs`, `combat_positions.rs`; original: `original-code/RHartificialmalignity.cpp`.

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-011-session-0001.jsonl.zst` - first divergence f207 - rng sites: DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition,SwordStrikeSelection,SmalltalkStrikeSide,MeleeNonMutualGate
- `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-001-session-0001.jsonl.zst` - first divergence f231 - rng sites: DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition,ArrowPiercingProtection

**All members:**

- f207 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-011-session-0001.jsonl.zst` - rng:DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition
- f231 `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-001-session-0001.jsonl.zst` - rng:DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition
- f243 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(81)).ai.list_them: original=[Soldier(SoldierId(102)), Soldier(
- f383 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-001-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,MeleeNonMutualGate,DefaultPostLook
- f417 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-005-session-0001.jsonl.zst` - rng:MeleeNonMutualGate,MeleeNonMutualGate,SmalltalkStrikeSide
- f424 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-001-session-0001.jsonl.zst` - rng:BowAccuracy,BowAccuracy,BowAccuracy
- f431 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-013-session-0001.jsonl.zst` - rng:DefaultPostLook,DrunkCombatFreeze,DrunkCombatFreeze
- f477 `parity-save-replays/30s-random-input/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-003-session-0001.jsonl.zst` - rng:DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition
- f488 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-009-session-0001.jsonl.zst` - rng:DefaultPostLook
- f492 `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-003-session-0001.jsonl.zst` - rng:MeleeInitiative,SmalltalkStrikeSide
- f503 `parity-save-replays/30s-random-input/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-002-session-0001.jsonl.zst` - rng:DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition
- f503 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-006-session-0001.jsonl.zst` - rng:DefaultPostLook,DefaultPostLook,DrunkCombatFreeze
- f517 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-003-session-0001.jsonl.zst` - rng:DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition
- f517 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-013-session-0001.jsonl.zst` - rng:MeleeInitiative,SmalltalkStrikeSide
- f518 `parity-save-replays/30s-random-input/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-001-session-0001.jsonl.zst` - rng:MeleeNonMutualGate,DrunkCombatFreeze,DrunkCombatFreeze
- f616 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-006-session-0001.jsonl.zst` - rng:MeleeInitiative,SmalltalkStrikeSide,BoredAnimationChoice
- f792 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-004-session-0001.jsonl.zst` - Pc(PcId(173)).actor.animation: original=238 rust=283
- f912 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-004-session-0001.jsonl.zst` - Soldier(SoldierId(111)).ai.substate: original=160 rust=236
- f983 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-012-session-0001.jsonl.zst` - rng:MacroRand,MeleeNonMutualGate,SwordDamageProtection
- f999 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-014-session-0001.jsonl.zst` - rng:MacroRand,MacroRand,MacroRand
- f1314 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-015-session-0001.jsonl.zst` - rng:SwordDamageProtection,SwordDamageProtection
- f1492 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-005-session-0001.jsonl.zst` - rng:SwordStrikeSelection

## Task 5: linux2 Savegame_029 f4200 - PC parry gate (Wait vs ParrySword) (12 traces)

**Hypothesis.** All 12 traces are the same save (linux2/Profile_002/Savegame_029) diverging in f4192-5351. The sharpest symptom (5 traces incl. the input-free `15-no-input` trace) is Pc170: original Waits (anim 283, motion_state 3) while rust raises `ParrySword` (anim 76, motion_state 2). Rust's PC decides an incoming strike must be parried when the original's doesn't - a parry-trigger predicate (strike direction/timing window, or whether the striker is actually targeting this PC) firing spuriously. The remaining 7 members (Pc169 Wait vs MoveOk, Soldier63 opponents list, movement float, visibility count, direction_goal) are later frontiers of the same battle and should collapse once the parry gate matches. Repro is deterministic without input: `parity-save-replays/15-no-input/traces/Savegame_linux2/Profile_002/Savegame_029-session-0001.jsonl.zst` at f4249.
**Likely source.** parry decision in `crates/robin_engine/src/engine/melee/swordfight.rs` / `dispatch.rs` (ParrySword command authoring); original: `original-code/RHelementactorpc.cpp`, `original-code/RHartificialmalignity.cpp` (parry consideration).

**Representative repros:**

- `parity-save-replays/30s-random-input/traces/Savegame_linux2/Profile_002/Savegame_029/replay-001-session-0001.jsonl.zst` - first divergence f4192 - Pc(PcId(170)).actor.animation: original=283 rust=76
- `parity-save-replays/30s-random-input/traces/Savegame_linux2/Profile_002/Savegame_029/replay-002-session-0001.jsonl.zst` - first divergence f4223 - Pc(PcId(170)).actor.animation: original=283 rust=76

**All members:**

- f4192 `parity-save-replays/30s-random-input/traces/Savegame_linux2/Profile_002/Savegame_029/replay-001-session-0001.jsonl.zst` - Pc(PcId(170)).actor.animation: original=283 rust=76
- f4223 `parity-save-replays/30s-random-input/traces/Savegame_linux2/Profile_002/Savegame_029/replay-002-session-0001.jsonl.zst` - Pc(PcId(170)).actor.animation: original=283 rust=76
- f4249 `parity-save-replays/15-no-input/traces/Savegame_linux2/Profile_002/Savegame_029-session-0001.jsonl.zst` - Pc(PcId(170)).actor.animation: original=283 rust=76
- f4249 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-002-session-0001.jsonl.zst` - Pc(PcId(170)).actor.animation: original=283 rust=76
- f4283 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-006-session-0001.jsonl.zst` - Pc(PcId(170)).actor.animation: original=283 rust=76
- f4288 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-009-session-0001.jsonl.zst` - Pc(PcId(169)).actor.animation: original=283 rust=303
- f4389 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-005-session-0001.jsonl.zst` - Soldier(SoldierId(63)).human.opponents: original=[Pc(PcId(169)), Pc(PcId(170)), 
- f4560 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-013-session-0001.jsonl.zst` - Soldier(SoldierId(113)).direction_goal: original=13 rust=12
- f4791 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-015-session-0001.jsonl.zst` - frame.visibility_queries.length: original=41 rust=38
- f4866 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(63)).movement_map.x: original=0.6279297 (0x3f20c000) rust=0.62
- f5185 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-010-session-0001.jsonl.zst` - Pc(PcId(169)).movement_map.x: original=0.69662476 (0x3f325600) rust=0.69659424 (
- f5351 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_029/replay-011-session-0001.jsonl.zst` - frame.visibility_queries.length: original=27 rust=29

## Task 6: nicouzouf Savegame_076 f33884 mass-event divergence (13 traces)

**Hypothesis.** Thirteen traces of the same save all diverge at exactly f33884-33886 (one at f34555) regardless of input schema - a deterministic scripted/timed event at f33884 that triggers a crowd reaction: the rng tails show dozens of `SeekPointSelection` draws plus `RuntimeBuildingExitWait`, `BoredAnimationChoice`, `VipIdleRemark`, and the two ordinary-mismatch members show Soldier210 getting `direction_goal` 0 vs 9/11 at f33886. Likely a building-exit / crowd-dispersal or alarm event where rust either includes a different set of participants or processes them in a different order, causing an avalanche of seek-point draws the original doesn't make (or makes elsewhere). Debug one trace's rng-draw diff at the event frame and identify which entity draws first.
**Likely source.** `crates/robin_engine/src/ai_enemy/seek.rs`, `periodic.rs` (RuntimeBuildingExitWait), `crates/robin_engine/src/engine/refresh_seek.rs`; original: `original-code/RHartificialintelligence.cpp`.

**Representative repros:**

- `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-001-session-0001.jsonl.zst` - first divergence f33884 - rng sites: RuntimeBuildingExitWait,RuntimeBuildingExitWait,BoredAnimationChoice,SeekPointSelection,SeekPointSelection,SeekPointSelection
- `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-002-session-0001.jsonl.zst` - first divergence f33884 - rng sites: AiRandomValueRectangle,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection

**All members:**

- f33884 `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-001-session-0001.jsonl.zst` - rng:RuntimeBuildingExitWait,RuntimeBuildingExitWait,BoredAnimationChoice
- f33884 `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-002-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,SeekPointSelection,SeekPointSelection
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-003-session-0001.jsonl.zst` - rng:VipIdleRemark,SeekPointSelection,SeekPointSelection
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-004-session-0001.jsonl.zst` - rng:BoredAnimationChoice,BoredAnimationChoice,SeekPointSelection
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-005-session-0001.jsonl.zst` - rng:VipIdleRemark,BoredAnimationChoice,SeekPointSelection
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-006-session-0001.jsonl.zst` - rng:VipIdleRemark,SeekPointSelection,SeekPointSelection
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-011-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,VipIdleRemark,BoredAnimationChoice
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-012-session-0001.jsonl.zst` - rng:VipIdleRemark,DefaultPostLook,SeekPointSelection
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-013-session-0001.jsonl.zst` - rng:VipIdleRemark,BoredAnimationChoice,SeekPointSelection
- f33884 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-015-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection
- f33886 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(210)).direction_goal: original=0 rust=9
- f33886 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-008-session-0001.jsonl.zst` - Soldier(SoldierId(210)).direction_goal: original=0 rust=11
- f34555 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_076/replay-007-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection

## Task 7: SeekPointSelection flood - wander-destination search entered off-schedule (16 traces)

**Hypothesis.** Rust consumes a long run of `SeekPointSelection` draws (up to 211 in one frame) terminated by `SeekPointAcceptance` at a frame where the original consumed none - i.e. rust starts (or restarts) a seek-point search loop the original doesn't, or its rejection-sampling loop iterates a different number of times. The count histogram (9, 12, 46, 53, 67, 73, 90, 113, 211 draws) suggests the whole search runs spuriously rather than one extra iteration. Check the seek trigger condition (idle timer expiry, `RuntimeBuildingExitWait`, bored/wander schedule) and the acceptance predicate (visibility/zone validity of candidate points - the site draws 3 values per candidate per sim_rng.rs). Members led by VipIdleRemark/Bored with seek-flood tails are the same event one draw earlier.
**Likely source.** `crates/robin_engine/src/ai_enemy/seek.rs` (SeekPointSelection/SeekPointAcceptance), `crates/robin_engine/src/engine/refresh_seek.rs`, `crates/robin_engine/src/ai_enemy/periodic.rs`; original: `original-code/RHartificialintelligence.cpp` (seek point search).

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_073/replay-006-session-0001.jsonl.zst` - first divergence f807 - rng sites: VipIdleRemark,AiRandomValueRectangle,VipIdleRemark,DefaultPostLook,SeekPointSelection,SeekPointSelection
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_036/replay-007-session-0001.jsonl.zst` - first divergence f1332 - rng sites: VipIdleRemark,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_039/replay-010-session-0001.jsonl.zst` - first divergence f10310 - rng sites: SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_033/replay-006-session-0001.jsonl.zst` - first divergence f11565 - rng sites: VipIdleRemark,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection,SeekPointSelection

**All members:**

- f807 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_073/replay-006-session-0001.jsonl.zst` - rng:VipIdleRemark,AiRandomValueRectangle,VipIdleRemark
- f1332 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_036/replay-007-session-0001.jsonl.zst` - rng:VipIdleRemark,SeekPointSelection,SeekPointSelection
- f1465 `parity-save-replays/60s-random-input/schema14/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_028/replay-011-session-0001.jsonl.zst` - rng:VipIdleRemark,BoredAnimationChoice,VipIdleRemark
- f10310 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_039/replay-010-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection
- f11565 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_033/replay-006-session-0001.jsonl.zst` - rng:VipIdleRemark,SeekPointSelection,SeekPointSelection
- f11886 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_031/replay-011-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection
- f12185 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_031/replay-007-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,SeekPointSelection,SeekPointSelection
- f19576 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_025/replay-013-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection
- f23297 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_035/replay-002-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection
- f25690 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_029/replay-006-session-0001.jsonl.zst` - rng:VipIdleRemark,SeekPointSelection,SeekPointSelection
- f26449 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_029/replay-015-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection
- f26751 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_029/replay-010-session-0001.jsonl.zst` - rng:VipIdleRemark,SeekPointSelection,SeekPointSelection
- f32485 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_001/Savegame_010/replay-001-session-0001.jsonl.zst` - rng:SeekPointSelection,SeekPointSelection,SeekPointSelection
- f32486 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_010/replay-001-session-0001.jsonl.zst` - rng:VipIdleRemark,BoredAnimationChoice,SeekPointSelection
- f38707 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_066/replay-001-session-0001.jsonl.zst` - rng:BoredAnimationChoice,VipIdleRemark,BoredAnimationChoice
- f39360 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_066/replay-002-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,VipIdleRemark,BoredAnimationChoice

## Task 8: ambient idle-RNG extra draws (VipIdleRemark / BoredAnimationChoice / AiRandomValueRectangle), by save (15 traces)

**Hypothesis.** Bucket of rng_mismatch traces whose first extra draw is an ambient idle site (VipIdleRemark, BoredAnimationChoice, AiRandomValueRectangle, MacroRand, DefaultPostLook). Per campaign history these labels are downstream symptoms: the per-frame ambient rolls happen for one extra (or one fewer) entity because some entity is in a different macro-state (still idle when the original is busy, or vice versa). These need per-trace log-tail work: identify which entity made the extra draw and sub-group by its actual state mismatch. Save-level sub-groups to start with: nicouzouf Savegame_073 (f807/f1148, 2 members here + 1 in Task 7), nicouzouf Savegame_037 (f962/f1511, battle save - may belong to Task 4), linux3/P3 Savegame_074 (f74062/f74569), linux2 Savegame_031 (f12898; also Savegame_031 members in Tasks 7/12), single-site minimal repros `VipIdleRemark`-only ExQuickSave f36163 and Savegame_017 f10321, and `BoredAnimationChoice`-only Savegame_051 f1142.
**Likely source.** `crates/robin_engine/src/ai_enemy/periodic.rs` (site definitions), `crates/robin_engine/src/sim_rng.rs`; original: `original-code/RHartificialintelligence.cpp` periodic/idle handlers.

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_073/replay-013-session-0001.jsonl.zst` - first divergence f807 - rng sites: VipIdleRemark,AiRandomValueRectangle,VipIdleRemark,SwordStrikeSelection,SwordStrikeSelection,SwordStrikeSelection
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_069/replay-010-session-0001.jsonl.zst` - first divergence f886 - rng sites: BoredAnimationChoice,DefaultPostLook,VipIdleRemark,SwordDamageProtection,SwordDamageProtection,MeleeProvoke
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_030/replay-013-session-0001.jsonl.zst` - first divergence f6378 - rng sites: VipIdleRemark,VipIdleRemark,DoorFightDispersion,DoorFightDispersion,RuntimeBuildingExitWait,RuntimeBuildingExitWait
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_017/replay-007-session-0001.jsonl.zst` - first divergence f10321 - rng sites: VipIdleRemark

**All members:**

- f807 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_073/replay-013-session-0001.jsonl.zst` - rng:VipIdleRemark,AiRandomValueRectangle,VipIdleRemark
- f886 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_069/replay-010-session-0001.jsonl.zst` - rng:BoredAnimationChoice,DefaultPostLook,VipIdleRemark
- f962 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_037/replay-005-session-0001.jsonl.zst` - rng:VipIdleRemark
- f1142 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_051/replay-001-session-0001.jsonl.zst` - rng:BoredAnimationChoice
- f1148 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_073/replay-014-session-0001.jsonl.zst` - rng:VipIdleRemark,VipIdleRemark,AiRandomValueRectangle
- f1511 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_037/replay-014-session-0001.jsonl.zst` - rng:VipIdleRemark,DrunkCombatFreeze,DrunkCombatFreeze
- f6378 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_030/replay-013-session-0001.jsonl.zst` - rng:VipIdleRemark,VipIdleRemark,DoorFightDispersion
- f10321 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_017/replay-007-session-0001.jsonl.zst` - rng:VipIdleRemark
- f12898 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_031/replay-003-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,VipIdleRemark
- f15608 `parity-save-replays/30s-random-input/traces/Savegame_linux2/Profile_002/Savegame_015/replay-003-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,VipIdleRemark,VipIdleRemark
- f18479 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_003/Savegame_019/replay-001-session-0001.jsonl.zst` - rng:MacroRand,BoredAnimationChoice,BoredAnimationChoice
- f28765 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_018/replay-013-session-0001.jsonl.zst` - rng:BoredAnimationChoice,BoredAnimationChoice
- f36163 `parity-save-replays/60s-random-input/schema14/traces/Savegame_SuN1Sh1nE/Profile_004/ExQuickSave/replay-012-session-0001.jsonl.zst` - rng:VipIdleRemark
- f74062 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_003/Savegame_074/replay-001-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,MacroRand,AiRandomValueRectangle
- f74569 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_074/replay-002-session-0001.jsonl.zst` - rng:AiRandomValueRectangle,VipIdleRemark

## Task 9: MoveOk vs MoveWaiting - anti-collision / right-of-way arbitration (9 traces)

**Hypothesis.** Anim 292 = movement-wait. Both directions occur: rust waits when the original walks (51/11/12 vs 292) and walks when the original waits (292 vs 10/51/52). So the pedestrian anti-collision arbitration (who yields when two walkers' paths intersect, or when a blocker occupies the next cell) resolves differently - suspect the blocker-detection radius, the priority/tie-break between two moving elements, or the re-check cadence while in MoveWaiting. One member is door-flavored (Pc179 MoveWaiting vs PassDoor f33948) and one starts from EquipBow vs MoveWaiting (Soldier115 f995) - keep them; they exercise the same arbitration from adjacent states.
**Likely source.** `crates/robin_engine/src/engine/anti_collision.rs`, movement command authoring in `crates/robin_engine/src/engine/movement.rs`, `crates/robin_engine/src/repulsive.rs`; original: `original-code/RHelementactor.cpp` (anti-collision / move-waiting), `original-code/RHParity.cpp` notes.

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_071/replay-010-session-0001.jsonl.zst` - first divergence f995 - Soldier(SoldierId(115)).actor.animation: original=12 rust=292
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_008/replay-015-session-0001.jsonl.zst` - first divergence f1136 - Soldier(SoldierId(58)).actor.animation: original=51 rust=292
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_022/replay-010-session-0001.jsonl.zst` - first divergence f1403 - Soldier(SoldierId(67)).actor.animation: original=292 rust=51
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_Nescafe/Profile_001/Savegame_000/replay-002-session-0001.jsonl.zst` - first divergence f1421 - Soldier(SoldierId(76)).actor.animation: original=51 rust=292

**All members:**

- f995 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_071/replay-010-session-0001.jsonl.zst` - Soldier(SoldierId(115)).actor.animation: original=12 rust=292
- f1136 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_008/replay-015-session-0001.jsonl.zst` - Soldier(SoldierId(58)).actor.animation: original=51 rust=292
- f1403 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_022/replay-010-session-0001.jsonl.zst` - Soldier(SoldierId(67)).actor.animation: original=292 rust=51
- f1421 `parity-save-replays/60s-random-input/schema12/traces/Savegame_Nescafe/Profile_001/Savegame_000/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(76)).actor.animation: original=51 rust=292
- f14240 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_009/replay-004-session-0001.jsonl.zst` - Soldier(SoldierId(136)).actor.animation: original=11 rust=292
- f17627 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_032/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(110)).actor.animation: original=292 rust=52
- f29248 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_034/replay-013-session-0001.jsonl.zst` - Soldier(SoldierId(235)).actor.animation: original=292 rust=10
- f33948 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_024/replay-013-session-0001.jsonl.zst` - Pc(PcId(179)).actor.animation: original=292 rust=295
- f42830 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_025/replay-004-session-0001.jsonl.zst` - Pc(PcId(183)).actor.animation: original=11 rust=292

## Task 10: attentive-mode enter/leave timing (anim 142, ai.state + alert_status) (6 traces)

**Hypothesis.** Six traces where the first mismatch bundles `actor.animation` 142 (`LeaveAttentiveMode`) or 51 vs 140/142 with `ai.state`, `ai.substate`, `detection.alert_status` and a large `visibility_queries` diff: a soldier drops (or keeps) attentive/alert posture one frame off. Both directions occur (original leaves attentive while rust stays: 142 vs 283/51/52; rust enters/stays attentive when original moves on: 51 vs 142/140). Suspect the attentive-mode countdown/decay condition (alert_status threshold, last-seen timer) or the order of detection update vs. posture decision within the frame. The visibility_queries avalanche is downstream (attentive soldiers scan differently).
**Likely source.** `crates/robin_engine/src/ai_enemy/alert.rs`, `crates/robin_engine/src/engine/transitions.rs` / `posture_transitions.rs` (Attentive), `crates/robin_engine/src/ai.rs`; original: `original-code/RHelementactorsoldier.cpp` (attentive mode), `original-code/RHartificialintelligence.cpp`.

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_041/replay-001-session-0001.jsonl.zst` - first divergence f611 - Soldier(SoldierId(76)).actor.animation: original=51 rust=140
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_030/replay-011-session-0001.jsonl.zst` - first divergence f6592 - Soldier(SoldierId(57)).actor.animation: original=142 rust=283
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_051/replay-006-session-0001.jsonl.zst` - first divergence f14130 - Soldier(SoldierId(146)).actor.animation: original=142 rust=51

**All members:**

- f611 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_041/replay-001-session-0001.jsonl.zst` - Soldier(SoldierId(76)).actor.animation: original=51 rust=140
- f6592 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_030/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(57)).actor.animation: original=142 rust=283
- f14130 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_051/replay-006-session-0001.jsonl.zst` - Soldier(SoldierId(146)).actor.animation: original=142 rust=51
- f14579 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Continue/replay-009-session-0001.jsonl.zst` - Soldier(SoldierId(91)).actor.animation: original=51 rust=142
- f14579 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_025/replay-009-session-0001.jsonl.zst` - Soldier(SoldierId(91)).actor.animation: original=51 rust=142
- f38703 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_035/replay-006-session-0001.jsonl.zst` - Soldier(SoldierId(66)).actor.animation: original=142 rust=52

## Task 11: movement/increment vector low-order float drift (10 traces)

**Hypothesis.** First divergence is a tiny numeric difference in `movement_map` / `increment_map` (typically 1-2 quanta of the trace's quantized f32 encoding - original values end in 0x..00) with no state/command mismatch. One shared math path computes a walk step slightly differently: candidates are the speed*direction-vector multiply order, a normalization (sqrt/hypot precision), the iso aspect-ratio scaling, or degrees->radians constants differing from the original's x87/tabled math. Note Soldier51 in nicouzouf appears 4x across different saves (Savegame_037/047/055) - likely one code path (soldier walk on a slope or diagonal?). Worth diffing rust vs original step computation for one repro at the divergent frame with full-precision logging.
**Likely source.** `crates/robin_engine/src/engine/movement.rs` (step/increment computation), `crates/robin_engine/src/position_interface.rs` (`sector_to_vector_iso`, aspect scaling), `crates/robin_engine/src/interp.rs`; original: `original-code/RHelementactor.cpp`, `original-code/RHpositioninterface.cpp`.

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_047/replay-004-session-0001.jsonl.zst` - first divergence f564 - Soldier(SoldierId(51)).movement_map.y: original=2.2339478 (0x400ef900) rust=2.2339172 (0x400ef880)
- `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_055/replay-003-session-0001.jsonl.zst` - first divergence f609 - Soldier(SoldierId(51)).movement_map.x: original=-3.5924683 (0xc065eb00) rust=-3.5924072 (0xc065ea00)
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_Nescafe/Profile_002/Savegame_000/replay-015-session-0001.jsonl.zst` - first divergence f1531 - Soldier(SoldierId(73)).increment_map.y: original=-0.3700293 (0xbebd747b) rust=-0.37001354 (0xbebd726a)
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_042/replay-012-session-0001.jsonl.zst` - first divergence f6743 - Pc(PcId(123)).increment_map.x: original=-0.4936227 (0xbefcbc1d) rust=-0.4936109 (0xbefcba91)

**All members:**

- f564 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_047/replay-004-session-0001.jsonl.zst` - Soldier(SoldierId(51)).movement_map.y: original=2.2339478 (0x400ef900) rust=2.23
- f609 `parity-save-replays/30s-random-input/traces/Savegame_nicouzouf/Profile_001/Savegame_055/replay-003-session-0001.jsonl.zst` - Soldier(SoldierId(51)).movement_map.x: original=-3.5924683 (0xc065eb00) rust=-3.
- f964 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_037/replay-013-session-0001.jsonl.zst` - Soldier(SoldierId(51)).movement_map.y: original=-1.0779419 (0xbf89fa00) rust=-1.
- f1094 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_037/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(51)).movement_map.x: original=-1.6959839 (0xbfd91600) rust=-1.
- f1397 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_032/replay-002-session-0001.jsonl.zst` - Pc(PcId(107)).movement_map.x: original=1.21521 (0x3f9b8c00) rust=1.215332 (0x3f9
- f1531 `parity-save-replays/60s-random-input/schema12/traces/Savegame_Nescafe/Profile_002/Savegame_000/replay-015-session-0001.jsonl.zst` - Soldier(SoldierId(73)).increment_map.y: original=-0.3700293 (0xbebd747b) rust=-0
- f6743 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_042/replay-012-session-0001.jsonl.zst` - Pc(PcId(123)).increment_map.x: original=-0.4936227 (0xbefcbc1d) rust=-0.4936109 
- f10562 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_039/replay-009-session-0001.jsonl.zst` - Pc(PcId(172)).movement_map.x: original=1.4731445 (0x3fbc9000) rust=1.4730225 (0x
- f19467 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_009/replay-007-session-0001.jsonl.zst` - Soldier(SoldierId(83)).movement_map.y: original=-2.4262695 (0xc01b4800) rust=-2.
- f39347 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_066/replay-008-session-0001.jsonl.zst` - Soldier(SoldierId(255)).increment_map.x: original=-0.59993 (0xbf199503) rust=-0.

## Task 12: visibility-raycast census drift (visibility_queries.length +-1..3) (13 traces)

**Hypothesis.** The per-frame set of vision line-of-sight queries differs in count with no earlier state mismatch: rust issues one-to-three extra or missing raycasts. The minimal repro is stunning: linux3/P1 Savegame_023 replay-001 at f54711 has original=1 rust=0 - the original casts exactly one ray that rust skips (replay-002/-003 show 2-vs-1 and 3-vs-2 at nearby frames, and the same save's replay-015/replay-003(schema14) rng/anim members sit 1k frames later - almost certainly the same missing check maturing into a state divergence). Suspect the detectable-filter/scan-scheduling (which detectables get a fresh LOS test this frame - round-robin cadence, distance gate, or a stale `seen_last_frame` early-out) rather than the raycaster itself, since query origins/destinations match where counts match.
**Likely source.** `crates/robin_engine/src/ai_vision.rs`, `crates/robin_engine/src/ai_detectable_filter.rs`, detection update in `crates/robin_engine/src/ai.rs`; original: `original-code/RHartificialintelligence.cpp` (vision/detectable scan loop), `original-code/RHsightobstacle`-equivalent in `crates/robin_engine/src/sight_obstacle.rs`.

**Representative repros:**

- `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_011/replay-013-session-0001.jsonl.zst` - first divergence f975 - frame.visibility_queries.length: original=10 rust=11
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_047/replay-010-session-0001.jsonl.zst` - first divergence f990 - frame.visibility_queries[2].destination: original_bits=[1139215841, 1114801303, 1119879300] rust_bits=[1139215841, 1114801305, 1119879300]
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_031/replay-014-session-0001.jsonl.zst` - first divergence f12560 - frame.visibility_queries.length: original=3 rust=4
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_036/replay-010-session-0001.jsonl.zst` - first divergence f34551 - frame.visibility_queries.length: original=32 rust=29

**All members:**

- f975 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_011/replay-013-session-0001.jsonl.zst` - frame.visibility_queries.length: original=10 rust=11
- f990 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_047/replay-010-session-0001.jsonl.zst` - frame.visibility_queries[2].destination: original_bits=[1139215841, 1114801303, 
- f1063 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_018/replay-014-session-0001.jsonl.zst` - frame.visibility_queries.length: original=10 rust=11
- f1290 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_011/replay-007-session-0001.jsonl.zst` - frame.visibility_queries.length: original=16 rust=18
- f12560 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_031/replay-014-session-0001.jsonl.zst` - frame.visibility_queries.length: original=3 rust=4
- f24019 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_033/replay-007-session-0001.jsonl.zst` - frame.visibility_queries.length: original=13 rust=14
- f25434 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_023/replay-008-session-0001.jsonl.zst` - frame.visibility_queries.length: original=31 rust=34
- f34551 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_036/replay-010-session-0001.jsonl.zst` - frame.visibility_queries.length: original=32 rust=29
- f54212 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_001/Savegame_023/replay-003-session-0001.jsonl.zst` - frame.visibility_queries.length: original=3 rust=2
- f54350 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_001/Savegame_023/replay-002-session-0001.jsonl.zst` - frame.visibility_queries.length: original=2 rust=1
- f54711 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_001/Savegame_023/replay-001-session-0001.jsonl.zst` - frame.visibility_queries.length: original=1 rust=0
- f54778 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_023/replay-003-session-0001.jsonl.zst` - Soldier(SoldierId(158)).actor.animation: original=283 rust=52
- f55629 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_023/replay-015-session-0001.jsonl.zst` - rng:VipIdleRemark,AiRandomValueRectangle,VipIdleRemark

## Task 13: helping-climb interaction trigger (anim 328/180/185 vs 8) (5 traces)

**Hypothesis.** Five PC traces around `EnterHelpingClimb`: original plays climb-help-related anims (328/180/185) while rust runs/waits (8/283), and once the inverse (8 vs 328 - rust starts helping when the original runs). The two-PC cooperative climb (one PC boosts another over a wall) engages under different conditions - suspect the proximity/eligibility test for offering the boost, or the order in which the helper/climber pair commits (TakeCorpse vs EnterHelpingClimb at f52484 shows rust choosing climb-help over corpse pickup - a priority inversion between interaction offers). All are late-frame random-input traces, so the fix likely lives in the interaction-selection predicate rather than a specific save event.
**Likely source.** grep `HelpingClimb` -> `crates/robin_engine/src/order.rs`, `element_priority.rs`, `crates/robin_engine/src/engine/sequence_validity.rs`, `stealth.rs`; original: `original-code/RHelementactorpc.cpp` / `RHelementactor.cpp` (helping climb), `original-code/RHartificialbonhomie.cpp`.

**Representative repros:**

- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_035/replay-012-session-0001.jsonl.zst` - first divergence f39561 - Pc(PcId(170)).actor.animation: original=328 rust=8
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_067/replay-008-session-0001.jsonl.zst` - first divergence f51860 - Pc(PcId(316)).actor.animation: original=328 rust=8
- `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_038/replay-007-session-0001.jsonl.zst` - first divergence f55593 - Pc(PcId(243)).actor.animation: original=180 rust=283

**All members:**

- f39561 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux2/Profile_002/Savegame_035/replay-012-session-0001.jsonl.zst` - Pc(PcId(170)).actor.animation: original=328 rust=8
- f51860 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_067/replay-008-session-0001.jsonl.zst` - Pc(PcId(316)).actor.animation: original=328 rust=8
- f52484 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_060/replay-005-session-0001.jsonl.zst` - Pc(PcId(246)).actor.animation: original=185 rust=180
- f55593 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_038/replay-007-session-0001.jsonl.zst` - Pc(PcId(243)).actor.animation: original=180 rust=283
- f77329 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_075/replay-013-session-0001.jsonl.zst` - Pc(PcId(344)).actor.animation: original=8 rust=328

## Task 14: CivilianBeggarSpeechGate extra draw (beggar speech cooldown) (4 traces)

**Hypothesis.** Rust draws an extra `CivilianBeggarSpeechGate` roll the original skips - the beggar's periodic speech-chance gate runs when the original's cooldown suppresses it (or for a beggar whose state differs). Directly adjacent to the just-landed fix `3ea0b8fc9` "Match beggar cooldown after fast pay click": these four traces are likely the remaining cooldown paths (a different reset trigger - alms from AI, player proximity timeout, or save-adopted cooldown state). Three of four are linux3/Profile_003 30s-random-input traces; check whether the fresh batch19 sweep already moved them before starting.
**Likely source.** `crates/robin_engine/src/engine/beggar.rs`, site in `crates/robin_engine/src/sim_rng.rs`; original: `original-code/rhelementactorcivilian.cpp` (beggar speech/alms).

**Representative repros:**

- `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_003/Savegame_043/replay-002-session-0001.jsonl.zst` - first divergence f8108 - rng sites: CivilianBeggarSpeechGate,VipIdleRemark,AiRandomValueRectangle
- `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_009/replay-003-session-0001.jsonl.zst` - first divergence f23208 - rng sites: CivilianBeggarSpeechGate,VipIdleRemark,DefaultPostLook,VipIdleRemark

**All members:**

- f8108 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_003/Savegame_043/replay-002-session-0001.jsonl.zst` - rng:CivilianBeggarSpeechGate,VipIdleRemark,AiRandomValueRectangle
- f23208 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_009/replay-003-session-0001.jsonl.zst` - rng:CivilianBeggarSpeechGate,VipIdleRemark,DefaultPostLook
- f39378 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_003/Savegame_072/replay-003-session-0001.jsonl.zst` - rng:CivilianBeggarSpeechGate,BoredAnimationChoice,VipIdleRemark
- f48082 `parity-save-replays/30s-random-input/traces/Savegame_linux3/Profile_003/Savegame_073/replay-003-session-0001.jsonl.zst` - rng:CivilianBeggarSpeechGate,VipIdleRemark,AiRandomValueRectangle

## Leftovers and near-clusters (below task threshold)


### LEFT-sg047 (4) - near-cluster: linux3/P1 Savegame_047 f83107-84003 - one battle event (PC342 motion/anim + SwordStrikeSelection rng + Soldier230 substate); viable 4-trace task if capacity allows, else fold into Task 3/4 follow-up

- f83107 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_047/replay-003-session-0001.jsonl.zst` - Pc(PcId(342)).actor.motion_state: original=3 rust=4
- f84003 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_047/replay-004-session-0001.jsonl.zst` - Soldier(SoldierId(230)).ai.substate: original=163 rust=160
- f83758 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_047/replay-009-session-0001.jsonl.zst` - Pc(PcId(342)).actor.animation: original=283 rust=6
- f83711 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_047/replay-010-session-0001.jsonl.zst` - rng:SwordStrikeSelection,SwordStrikeSelection,VipIdleRemark

### LEFT-projectile (4) - near-cluster: projectile lifecycle - two traces where a despawned arrow keeps layer 1/2 vs original 65535 (layer not cleared on removal), one sprite_frame 4 vs 0, one elevation arc drift; likely 2 distinct small bugs in crates/robin_engine/src/../projectile handling vs original-code/RHelementprojectile.cpp

- f35731 `parity-save-replays/15-no-input/traces/Savegame_SuN1Sh1nE/Profile_004/QuickSave-session-0001.jsonl.zst` - Projectile(ProjectileId(651)).sprite_frame: original=4 rust=0
- f5022 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_008/replay-014-session-0001.jsonl.zst` - Projectile(ProjectileId(176)).layer: original=65535 rust=2
- f8630 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_015/replay-012-session-0001.jsonl.zst` - Projectile(ProjectileId(245)).layer: original=65535 rust=1
- f12292 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_029/replay-001-session-0001.jsonl.zst` - Projectile(ProjectileId(147)).elevation: original=66.359375 (0x4284b800) rust=75

### LEFT-substate155 (3) - near-cluster: Soldier ai.substate 155 vs 250 (twice, different saves) and 208 vs 155 - one substate-machine edge in ai_enemy/substate_handlers.rs

- f972 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_020/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(73)).ai.substate: original=155 rust=250
- f662 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_055/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(51)).ai.substate: original=208 rust=155
- f19639 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_034/replay-014-session-0001.jsonl.zst` - Soldier(SoldierId(74)).ai.substate: original=155 rust=250

### LEFT-equipbow (3) - near-cluster: PC auto-EquipBow (anim 85) when original moves/waits - bow-equip decision gate in abilities/bow_shot; 3 traces (+ 1 inverse Soldier case parked in Task 9)

- f25981 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_029/replay-012-session-0001.jsonl.zst` - Pc(PcId(137)).actor.animation: original=86 rust=85
- f1314 `parity-save-replays/60s-random-input/schema12/traces/Savegame_randomguy/Profile_004/Savegame_036/replay-015-session-0001.jsonl.zst` - Pc(PcId(107)).actor.animation: original=5 rust=85
- f553 `parity-save-replays/60s-random-input/schema14/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_001/replay-005-session-0001.jsonl.zst` - Pc(PcId(252)).actor.animation: original=3 rust=85

### LEFT-sg013 (2) - near-cluster: SuN1Sh1nE Savegame_013 f1963/f2165 - AiPanic draw flood + Soldier81 LowerShield vs Wait; single shield/panic event

- f2165 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_013/replay-005-session-0001.jsonl.zst` - Soldier(SoldierId(81)).actor.animation: original=283 rust=173
- f1963 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_013/replay-006-session-0001.jsonl.zst` - rng:AiPanic,AiPanic,AiPanic

### LEFT-rng (2) - unclustered rng: SwordDamageProtection lead (linux2 Savegame_018 f8873), DrunkCombatFreeze lead (linux3/P3 Savegame_010 f20363; note same save as leftover PassDoor f18927)

- f8873 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_018/replay-008-session-0001.jsonl.zst` - rng:SwordDamageProtection,SwordDamageProtection,MeleeProvoke
- f20363 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_010/replay-008-session-0001.jsonl.zst` - rng:DrunkCombatFreeze,DrunkCombatFreeze,CombatReposition

### LEFT (22) - singletons

- f738 `parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_036/replay-006-session-0001.jsonl.zst` - Pc(PcId(318)).actor.animation: original=50 rust=6
- f11979 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux2/Profile_002/Savegame_031/replay-015-session-0001.jsonl.zst` - Soldier(SoldierId(43)).actor.animation: original=12 rust=283
- f28290 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_018/replay-002-session-0001.jsonl.zst` - Soldier(SoldierId(131)).direction_goal: original=2 rust=12
- f28282 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_018/replay-006-session-0001.jsonl.zst` - Pc(PcId(183)).actor.animation: original=8 rust=283
- f29221 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_001/Savegame_034/replay-008-session-0001.jsonl.zst` - Pc(PcId(297)).actor.motion_state: original=2 rust=4
- f8775 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_043/replay-007-session-0001.jsonl.zst` - Soldier(SoldierId(92)).direction_goal: original=11 rust=3
- f36854 `parity-save-replays/60s-random-input/schema12/traces/Savegame_linux3/Profile_003/Savegame_055/replay-007-session-0001.jsonl.zst` - Pc(PcId(298)).actor.animation: original=11 rust=12
- f1920 `parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_000/replay-012-session-0001.jsonl.zst` - Pc(PcId(126)).actor.motion_state: original=3 rust=4
- f1199 `parity-save-replays/60s-random-input/schema12/traces/Savegame_randomguy/Profile_004/Restart/replay-006-session-0001.jsonl.zst` - Soldier(SoldierId(225)).actor.motion_state: original=3 rust=2
- f941 `parity-save-replays/60s-random-input/schema14/traces/Savegame_Nescafe/Profile_003/Savegame_001/replay-005-session-0001.jsonl.zst` - Pc(PcId(252)).actor.wait_time: original=22 rust=21
- f33618 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_036/replay-008-session-0001.jsonl.zst` - Pc(PcId(81)).actor.animation: original=12 rust=10
- f28893 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_044/replay-003-session-0001.jsonl.zst` - Soldier(SoldierId(223)).actor.animation: original=303 rust=283
- f44592 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_001/Savegame_045/replay-004-session-0001.jsonl.zst` - Soldier(SoldierId(140)).actor.animation: original=304 rust=303
- f55516 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_005/replay-008-session-0001.jsonl.zst` - Soldier(SoldierId(52)).ai.substate: original=73 rust=71
- f18927 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_010/replay-009-session-0001.jsonl.zst` - Pc(PcId(194)).actor.command: original=PassDoor rust=MoveOk
- f19091 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_019/replay-005-session-0001.jsonl.zst` - Pc(PcId(243)).actor.animation: original=303 rust=283
- f12003 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_028/replay-009-session-0001.jsonl.zst` - Soldier(SoldierId(87)).position_goal_map.x: original=0 (0x00000000) rust=1474 (0
- f12812 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_028/replay-010-session-0001.jsonl.zst` - Soldier(SoldierId(93)).position_goal_map.x: original=0 (0x00000000) rust=1477 (0
- f1332 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_046/replay-011-session-0001.jsonl.zst` - Soldier(SoldierId(68)).actor.motion_state: original=1 rust=2
- f14088 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_051/replay-008-session-0001.jsonl.zst` - Civilian(CivilianId(121)).ai.substate: original=24 rust=23
- f1066 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_054/replay-007-session-0001.jsonl.zst` - Soldier(SoldierId(201)).actor.action_state: original=7 rust=8 (original animatio
- f39679 `parity-save-replays/60s-random-input/schema14/traces/Savegame_linux3/Profile_003/Savegame_066/replay-012-session-0001.jsonl.zst` - Soldier(SoldierId(251)).detection.detectables[47].last_visibility: original=5.75

## Cross-cutting notes for dispatch

- Tasks 1 and 2 both live in the vector->16-sector classification; they can be dispatched in parallel (different call paths) but should coordinate on `position_interface.rs`.
- Tasks 3, 4, 5 are all melee-decision families and may partially collapse into one another; dispatch Task 5 first (it has an input-free `15-no-input` repro), then re-measure 3/4.
- Task 6, Task 7 and the Task 8 bucket all end in seek/idle rng floods; Task 7's mechanism (seek loop) is likely the shared engine, Tasks 6/8 the per-save triggers. If the Task 7 agent finds a systemic seek-loop bug, re-run Tasks 6/8 members before dispatching them.
- Savegame_066 members are split across Task 7 (f38707/f39360), Task 11 (f39347) and leftovers (f39679) - all in f38.7-39.7k; if the Task 7 agent touches that save, check the other two.
- The fresh sweep in `output/parity-audits/batch19-head07872e94-nestedd7792d55-preflight/` was ~60/226 complete during this analysis; spot-checks matched batch15 frontiers, but agents should re-verify their members' frontiers against its logs/ when they start.
- ~350 already-fixed families are catalogued in `docs/PARITY_CAMPAIGN_STATE.md`; grep it for your site/field before assuming a family is untouched (esp. tasks #295/#345/#360/#545/#549 which border Tasks 1/2/4/5).
