# Mandatory-death audit for preserved lives

Reviewed all 39 full-game mission SCBs through the mission decompiler, including
victory checks, their phase/message dependencies, death predicates and scripted
damage. The decompiler's unrecovered jumps require following the phase logic;
a displayed `return 1` on its own is not evidence of an unconditional victory.

## Exclusions

| Mission | Actor script slot | Target | Required path |
| --- | ---: | --- | --- |
| `S05_Yrk_EC` | 143 | Guisbourne | `StartUp.Hourglass` gates Marian's rescue on his incapacitation. Only Robin can duel this VIP; town Robin's sword has zero stunning effect on every strike, so normal play requires a lethal duel. |
| `H10_Yor_VL` | 106 | Longchamps | `StartUp.Hourglass` requires `IsActorDead` during phase 2 before setting global 0 to 3. The disguise/body extraction sequence advances through phases 4 and 5; `CheckVictoryCondition` succeeds at phase 5. |
| `H12_Not_MP` | 97 | Sheriff | `StartUp.Hourglass` requires `IsActorDead` before launching the ending sequence. Its messages eventually set `bVictory2`, which `CheckVictoryCondition` requires. |

These are script-element slots, not soldier-array indices. Resolve them through
the existing loaded entity store, including after save restoration. Only the
named mission's target is omitted, from both living and dead totals. This avoids
crediting a required kill as a life spared. No additional entity list is saved.

H10 also has separate Longchamps disguise and corpse actors (slots 74 and 73).
They are not the combat target. The starting-corpse option handles baseline
hostile corpses independently.

## Cases kept in the ordinary tally

- `Str02_Der_MP`: Scathlock (24) and his replacement (25) have death checks that
  award the general's blazon. They are optional blazon objectives, not a
  mandatory victory gate. Neither receives the required-kill exemption.
- `H07_Not_MK`: the earlier Sheriff encounter does not require his death.
- `H04_Lei_VL`: the special guard group is exempt from the mission's no-death
  failure condition. That permission does not require their deaths.
- `IsActorHS` accepts death, tying, or unconsciousness. Objectives using it or
  `AreAllEnemiesInsideHS` do not by themselves require a kill. Also check combat
  restrictions and weapon profiles: Guisbourne is an exception to a simple
  death-predicate-only audit.
- `KillActorsInZone` is a misleading helper name: the inspected forest helpers
  apply `InflictPain` with stun enabled. They are not mandatory lethal targets.

## Coverage

| Scripts | Reviewed success route | Mandatory death additions |
| --- | --- | --- |
| `H01_Lin_VL`, `H02_Not_EC`, `H03_Der_MK`, `H04_Lei_VL`, `H05_Lin_EC`, `H07_Not_MK`, `H09_Not_VL` | Escape, rescue, meetings, phase flags, or incapacitating guards | None |
| `H10_Yor_VL`, `H12_Not_MP` | Death-gated story phases and ending sequence | Listed above |
| `S01_Not_VL`, `S02_Lei_MP`, `S03_FoB_MP`, `S04_Der_EC` | Rescue and extraction; dead rescuees are sometimes tolerated, not required | None |
| `S05_Yrk_EC` | Guisbourne duel, Marian rescue and extraction | Guisbourne, listed above |
| `Str01_Lin_EC`, `Str02_Der_MP`, `Str03_Yor_MK` | Blazon objectives, including nonlethal guard/general conditions | None |
| `Emb01_FoA_EC`, `Emb02_FoC_MK`, `Emb03_FoC_MP`, `Emb04_FoA_MP`, `Emb05_FoB_MP`, `Emb06_FoC_EC`, `Emb07_FoB_JMS`, `Emb08_FoA_JMS`, `Emb09_FoB_JMS`, `EmbTut_FoC_EC` | Seize treasure or search collectors | None |
| `Tac01_FoA_MP`, `Tac02_FoB_EC`, `Tac03_FoC_MP`, `Tac04_FoA_EC`, `Tac05_FoC_MP`, `Tac06_FoB_EC`, `Tac17_FoC_EC`, `Tac18_FoA_EC`, `Tac19_FoB_EC`, `Tac21_FoB_EC` | Blazons, incapacitation, supplies, or the messenger objective | None |
| `sherwood`, `SherwoodOutro` | Camp and ending presentation | None |

## Configuration and scope

“Exclude Required Kills” defaults to enabled, independently of “Exclude Starting
Corpses”. Both settings are deterministic session rules, host-controlled in
multiplayer, and disabled in parity sessions. The adjusted living/dead pair feeds
campaign preserved-life totals, debriefing counts, and post-mission recruitment.
Mission results and the main menu show the percentage plus “saved N of M”.
Historical campaign totals are retained. Achievement death rules are unchanged.

The registry covers these shipped mission identities. It does not infer
exemptions from VIP status, actor names, or optional objectives in custom missions.
TODO: provide authored required-death metadata if custom missions need their own
exemptions rather than extending the reviewed registry.

## Relic ending

The ending transition requires campaign stage 10 and at least seven collected
relics, then queues `SherwoodOutro` with the gang's VIP characters and stage 11.
It does not test Scathlock's campaign flag or the Silver Arrow's separate bit.

- Sceptre (`H03_Der_MK`): opened after the meeting;
  nearby guards need not die.
- Domesday Book (`H04_Lei_VL`): activated by the opened tower; guards can be subdued.
- Coronation Spoon (`S04_Der_EC`): `IsActorHS` on `Soldier_B03`
  reveals it; the knight is not a VIP and can be knocked out.
- Sword of State (`H07_Not_MK`): collectible pickup; the Sheriff encounter has a
  scripted health threshold escape rather than a required death.
- Ampulla (`S05_Yrk_EC`): `IsActorHS` on `Officer04` reveals it. The officer is not
  a VIP and can be knocked out by Little John. Guisbourne's required duel remains
  covered separately.
- Royal Seal (`H09_Not_VL`): revealed by a scroll; guards need not die.
- Crown (`H10_Yor_VL`): revealed by `CrownParch.IsTaken`; Longchamps' required duel
  remains covered separately.

These collection paths add no further unavoidable deaths. The saved-life
exemptions therefore cover the entire relic route without exempting ordinary
collectible guards or optional general-blazon kills.
