# Remaining-25 triage (batch25, runner ebf279a60, 2026-08-17)

25 traces remain non-EOF. Grouped by first-divergence root cause, NOT cluster label.

Substate numbering (Rust `ai::model::Substate` == C++ `RHsubstate`, 1:1, `disc = variant index`):
160=AttackingSwordfight, 162=AttackingSwordfightParade, 155=AttackingRunningToEnemy,
166=AttackingReserveOverview, 173=AttackingApproachingNewEnemy, 179=AttackingBowAiming,
181=AttackingBowObservingLoading, 183=AttackingArcherRetireFromCombatTurn,
23=DefaultPatrolEnroute, 24=DefaultPatrolEnrouteWaiting, 236=BeginAdditionalSubstates (SENTINEL).

## Group A — RNG divergence (6 traces, status 101 = Rust consumed MORE draws than Original)
| trace | frame | extra draw sites |
|---|---|---|
| 30s S019 r001 | 18479 | MacroRand, BoredAnimationChoice, BoredAnimationChoice, VipIdleRemark |
| 30s S074 r001 | 74062 | AiRandomValueRectangle, MacroRand, AiRandomValueRectangle |
| linux2 S031 r011 | 11886 | SeekPointSelection x8, SeekPointAcceptance, VipIdleRemark x3 |
| linux3 S074 r002 | 74569 | AiRandomValueRectangle, VipIdleRemark |
| nicouzouf S037 r005 | 962 | VipIdleRemark |
| SuN1Sh1nE S013 r006 | 1963 | AiPanic x4, VipIdleRemark, DrunkCombatFreeze x2, CombatReposition, SwordStrikeSelection, AiRandomValueRectangle, MeleePrincipalReshuffle, PrincipalOpponent, MeleeNonMutualGate |

## Group B — order-advance timing (actor.animation/command diverge)
| trace | frame | signature |
|---|---|---|
| nicouzouf S020 r014 | 1257 | S73 Wait->MoveOk, anim 54->303 |
| SuN1Sh1nE S024 r006 | 1337 | Pc174 Wait->MoveWaiting, anim 283->292 |
| SuN1Sh1nE S024 r014 | 1426 | S146 EnterAttentiveMode->Wait, anim 141->3, motion 2->1 |
| SuN1Sh1nE S024 r015 | 1397 | S95 ParryShield->RaiseShieldInstantly, anim 172->171 |
| SuN1Sh1nE S013 r005 | 2165 | S81 Wait->LowerShield, ai 183->166 |
| linux2 S031 r015 | 11979 | S43 WaitTimer->Wait, anim 12->283, wait_time 50->0 |
| linux3 P001 S018 r002 | 28416 | S138 Wait->MoveOk, anim 89->88, ai 179->181, view 6->1 |
| linux3 P001 S034 r013 | 29248 | S235 MoveWaiting->MoveOk, anim 292->10 |
| linux2 S024 r013 | 33948 | Pc179 MoveWaiting->PassDoor, anim 292->295, pos_goal 0->(1855,1337) |
| linux3 P001 S009 r004 | 14240 | S136 MoveOk->MoveWaiting, anim 11->292 |
| linux3 P001 S044 r003 | 28893 | S223 MoveOk->Wait, anim 303->283, motion 2->3, ai 173->160 (LINE-FORMATION, probe finding 4) |

## Group C — ai.substate sentinel/transition
| trace | frame | signature |
|---|---|---|
| SuN1Sh1nE S024 r004 | 912 | S111 ai 160->236 (SENTINEL BeginAdditionalSubstates) |
| linux3 P003 S051 r008 | 14088 | Civilian121 ai 24->23, pos_goal 0->(1322,2276) |
| linux2 S032 r002 | 17627 | S110 ai 155->160, opponents list grows (premature EnterSwordfight) |

## Group D — direction_goal
| trace | frame | signature |
|---|---|---|
| linux2 S039 r003 | 10950 | S138 direction_goal 7->6 (single field) |
| linux3 P003 S051 r004 | 14449 | S144 direction_goal 8->4 + full movement/pos_goal divergence |

## Group E — projectile physics
| trace | frame | signature |
|---|---|---|
| linux3 P003 S029 r001 | 12292 | Projectile147 elevation/movement/pos all diverge |
| 15-no-input QuickSave | 35731 | Projectile651 sprite_frame 4->0, sprite_row 6->0 |
