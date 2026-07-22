# Authoritative RNG Audit

PA-036 retains one serialized, Engine-owned `fastrand` stream, plus one
separately typed deterministic auxiliary generator whose output enters
authoritative campaign state without advancing that stream. It does not
attempt to reproduce a particular libc `rand()` sequence. Parity requirements
are the number and order of draws, their range and branch semantics, and the
gameplay phase that owns them.

Every production draw takes a typed `RngSite`. The
`authoritative_rng_source_inventory_is_reviewed` test parses all Rust sources
in `robin_engine`, `robin_lua`, and the runtime `robin_rs` crate, rejects
unlabelled `sim_rng` calls and unreviewed ambient `rand`/`fastrand` use, and compares
structural site-use counts with the reviewed manifests in `sim_rng.rs`. It
also compares the module's public function surface with an entry-point
allowlist and requires one row below for every `RngSite` and
`AuxiliaryRngSite`. This is deliberately source-structural: it does not depend
on line numbers.

## Snapshot contract

`EngineInner::control.rng: SimulationRng` owns the stream at every snapshot boundary.
It serializes the complete `fastrand::Rng::get_seed()` state and participates
in `StateHash`; `Engine`, save-game, multiplayer, replay, and rollback clones
therefore all carry the exact next draw. `perform_hourglass`, post-initialize,
level ingestion, SCB startup, and Sherwood production receive an explicit
Engine-derived `SimulationContext`; no production RNG is installed in TLS.
Campaign mission selection occurs before the loaded Engine exists, so
`Engine::select_next_mission` temporarily owns the campaign in a bare
`EngineInner`, advances that same `SimulationRng`, and hands the complete next
seed and existing `SimConfig` to `EngineArgs`. A finished mission returns its
next RNG seed and `SimConfig` with the campaign for the following selection.
Save loads preflight the serialized mission-construction seed/config, while
the existing campaign restart snapshot carries its paired pre-selection
seed/config. Replays require campaign, seed, and config in their version-7
header; network version 13 announces mission identity with the same seed and
config before Engine construction. This preserves the original
single process-wide sequence (`launcher.cpp:763-765`, `RHCampaign.cpp`) rather
than creating identically seeded campaign and mission streams.
In the table, **E** means this complete Engine serialization/hash
coverage. Initial-load draws happen before the frame-zero Engine snapshot and
are consequently covered by that snapshot.

## Authoritative gameplay inventory

“Uses” is the guarded number of production source expressions naming the
site, not a runtime draw count. Loops and conditional sequences state their
runtime behavior explicitly. Ranges use Rust notation; `C-unit` means an
inclusive `0..=32767 / 32767` fraction matching Original's MSVC-era
`rand()/RAND_MAX` endpoint semantics.

| Site | Uses | Rust location / function | Range and draw order | Original provenance | Owner / phase | Snapshot |
| --- | ---: | --- | --- | --- | --- | --- |
| `AiPanic` | 7 | `ai/controller.rs`: `think_expected_event` panic substates | Conditional sequence: `[0,200)`, `[0,16)`, `[0,delta)`, `[0,16)`, `[-2,2]`, `[-3,3]`, `[0,delta)` | `RHArtificialIntelligence::ThinkExpectedEventCommonStuff`, `RHartificialintelligence.cpp:1902-1986` | NPC expected-event handling, creation-order tick | E |
| `AiRandomValueGauss` | 3 | `AiController::random_value` | Three ordered `[0,trunc(range*0.166))` draws when divisor is positive | `RHArtificialIntelligence::RandomValue`, `RHartificialintelligence.cpp:4687-4689` | AI decision evaluation | E |
| `AiRandomValueGaussHigh` | 3 | `AiController::random_value` | Three ordered `[0,trunc(range*0.333))` draws when divisor is positive | `RHArtificialIntelligence::RandomValue`, `RHartificialintelligence.cpp:4654-4656` | AI decision evaluation | E |
| `AiRandomValueRectangle` | 1 | `AiController::random_value` | `[min,max)`; no draw when `min == max` | `RHArtificialIntelligence::RandomValue`, `RHartificialintelligence.cpp:4644` | AI decision evaluation | E |
| `ArcherForestTarget` | 1 | `ai_enemy/archer_combat.rs`: target scoring | `[0,10000)` added only for distant forest targets | `RHArtificialMalignity::FindArcherTarget`, `RHartificialmalignity.cpp:18755` | Archer combat target scan | E |
| `ArrowFallingFrame` | 1 | `bow_shot.rs`: `apply_arrow_falling_sprite_visual` | `[3,6)` once per falling refresh | `RHElementArrow::Refresh`, `RHElementArrow.cpp:334` | Projectile refresh | E |
| `ArrowPiercingProtection` | 1 | `engine/combat.rs`: arrow victim filter | `[0,101)`, drawn only after the base hurtable gate | `RHElementActorPC::IsHurtableByArrow` and soldier twin, `Rhelementactorpc.cpp:10209`, `Rhelementactorsoldier.cpp:2794` | Projectile impact | E |
| `BattleCourage` | 1 | `ai_enemy/battle.rs`: predecision | `[0,100)`, only after the odds gate | `RHArtificialMalignity::GetPreDecision`, `RHartificialmalignity.cpp:8154` | Enemy battle decision | E |
| `BattlePanicRemark` | 3 | `ai_enemy/battle.rs`: panic/cassos branches | `[0,2)` only after each corresponding alert/run succeeds | `RHArtificialMalignity` battle branches, `RHartificialmalignity.cpp:7893,7923,7930` | Enemy battle reaction | E |
| `BattleProvoke` | 1 | `ai_enemy/battle.rs`: engage target | `[0,4) == 0` only for sword-ready actors | `RHArtificialMalignity`, `RHartificialmalignity.cpp:7602` | Enemy battle reaction | E |
| `BoredAnimationChoice` | 1 | `engine/animation.rs`: wait completion | `[0,10) == 0` when a bored WAIT cycles | `RHElementActor::Execute`, `Rhelementactor.cpp:1063` | Animation completion | E |
| `BowAccuracy` | 4 | `bow_shot.rs`: `roll_hit_and_compute_bias` | First `[1,100]`; on miss, exactly three `[-2,2]` bias draws | `RHElementActorHuman::ShootBow`, `Rhelementactorhuman.cpp:7222-7224` | Bow release | E |
| `BuildingExitGate` | 1 | `ai/macro_patrol.rs`: `pick_building_exit_gate` | `[0,candidate_count)` after collecting non-entry gates | `RHElementActorHuman` building-exit selection, `Rhelementactorhuman.cpp:12906` | AI movement planning | E |
| `CampaignAccessChance` | 2 | `campaign.rs`: both chance filters | `[0,101)` once per eligible candidate, in stable candidate order | `RHCampaign::HasBlazonMissionNoAccessChance` / `HasNonBlazonMissionNoAccessChance`, `RHCampaign.cpp:582,624` | Campaign mission selection | E |
| `CampaignForcedMission` | 1 | `Campaign::update_accessible_missions` fallback | `[0,fallback.len())` after ten failed selection passes | `RHCampaign::UpdateAccessibleMissions`, `RHCampaign.cpp:468` | Campaign mission selection tail | E |
| `CampaignNewPeasantType` | 1 | `Campaign::add_new_peasant_to_gang` | `[0,candidate_count)` only for missing/out-of-range requested type | `RHCampaign::AddNewPeasantToGang`, `RHCampaign.cpp:784` | Post-mission recruitment | E |
| `CampaignReinforcementPeasant` | 1 | `Campaign::get_random_peasant_from_gang` | `[0,pool.len())`, preferred pool before fallback pool | `RHCampaign::GetRandomPeasantFromGang`, `RHCampaign.cpp:706,715` | Reinforcement creation after door draw | E |
| `CampaignReservistReturn` | 2 | `Campaign::add_new_peasant_to_gang` | One 50% draw; on true, `[0,reservist_count)` | `RHCampaign::AddNewPeasantToGang`, `RHCampaign.cpp:758-761` | Post-mission recruitment | E |
| `CharlySorrow` | 2 | `ai_enemy/substate_handlers.rs`: looking for Charly | `[0,5000)`; conditional `[0,2)` look direction only when sorrow triggers | `RHArtificialMalignity::ThinkExpectedEvent`, `RHartificialmalignity.cpp:571-576` | Enemy timer event | E |
| `CheckForLookDirection` | 1 | `AiController::check_for` | `[0,2)` after synchronization bookkeeping | `RHArtificialIntelligence::CheckFor`, `RHartificialintelligence.cpp` | Patrol/check-for setup | E |
| `CivilianBeggarSpeechChoice` | 1 | `AiFriendly::random_speech` | `[0,5)` only after the speech gate succeeds | `RHArtificialBonhomie::RandomSpeech`, `RHartificialbonhomie.cpp:1128` | Creation-ordered civilian NPC tail at wrapped phase 0 | E |
| `CivilianBeggarSpeechGate` | 1 | `AiFriendly::random_speech` | `[0,3) == 0` after silence/cooldown gates | `RHArtificialBonhomie::RandomSpeech`, `RHartificialbonhomie.cpp:1125` | Creation-ordered civilian NPC tail at wrapped phase 0 | E |
| `CivilianFirstLookTimer` | 1 | `AiFriendly::init` | `AB_MIN + [0,AB_DELTA)` | `RHArtificialBonhomie::Init`, `RHartificialbonhomie.cpp:1360` | Civilian AI initialization | E |
| `CivilianPanicDirection` | 1 | `AiFriendly::panic_from_point` | `[0,5)-2`, wrapped to 16 sectors | `RHArtificialBonhomie::Panic`, `RHartificialbonhomie.cpp:1628` | Civilian panic start | E |
| `CombatObserveSideStep` | 1 | `ai_enemy/combat_positions.rs`: observer reposition | 50%, only after straight movement was rejected | `RHArtificialMalignity`, `RHartificialmalignity.cpp:15180` | Enemy combat positioning | E |
| `CombatReposition` | 1 | `ai_enemy/combat_positions.rs`: position reevaluation | `[0,3) == 0` after trainer/one-on-one gates | `RHArtificialMalignity`, `RHartificialmalignity.cpp:13404` | Enemy combat positioning | E |
| `DefaultPostLook` | 1 | `AiEnemy::return_to_duty` | `[0,4)` mapped to four look patterns | `RHArtificialMalignity::ReturnToDuty`, `RHartificialmalignity.cpp:9781` | Enemy return-to-duty | E |
| `DelayedSoundTimer` | 1 | `EngineInner::perform_hourglass` delayed sources | `[0,delay_stepping)`, then scaled into authored delay range | Original delayed-source timing uses `Random` in `RHsoundcache.h`; deterministic ownership is a Rust extension | Post-hourglass authoritative sound timer | E |
| `DoorFightDispersion` | 2 | `engine/soldier_helpers.rs`: door-fight placement | Per attempt: `[-3,3]` direction then `[30,93]` magnitude | `RHArtificialIntelligence` door-fight dispatch, `RHartificialintelligence.cpp:7617-7619` | Door battle setup | E |
| `DoorFightTarget` | 1 | `engine/soldier_helpers.rs`: extra pursuer assignment | `[0,fleeing_count)` for each excess pursuer | `RHArtificialIntelligence`, `RHartificialintelligence.cpp:7649` | Door battle setup | E |
| `DrunkCombatFreeze` | 2 | `ai_enemy/combat_positions.rs`: combat tick | First `[0,100)`; second only when first exceeds alcohol | `RHArtificialMalignity`, `RHartificialmalignity.cpp:13382` | Enemy combat work | E |
| `DrunkenPathDeviation` | 2 | `engine/tick.rs`: `apply_drunken_path_deviation` | Per attempt: `[0,16)` direction then `[0,16)` magnitude | `RHElementActorSoldier::Translate`, `Rhelementactorsoldier.cpp:1754-1755` | Movement path construction | E |
| `EnemySeekDirectionShuffle` | 1 | enemy seek-point setup | Insertion index `[0,current_len]` once per authored direction | `RHArtificialMalignity`, `RHartificialmalignity.cpp:2168` | Seek substate entry | E |
| `EnemySeekLook` | 2 | enemy seek/watch timer substates | `[0,2)` at the two distinct sideward-look transitions | `RHArtificialMalignity`, `RHartificialmalignity.cpp:2194` and related seek branch | Enemy expected-event handling | E |
| `EnemyWonderingLook` | 5 | enemy wondering substates | Ordered transition-specific `[0,2)`, `[0,8)`, `[0,2)`, `[0,8)`, and `[0,2)` draws | `RHArtificialMalignity`, `RHartificialmalignity.cpp:757,774,1757,1791` | Enemy expected-event handling | E |
| `HeroSpeech` | 1 | `engine/melee/speech.rs`: eventual hero expression | 50% after immediate speeches, before done speeches | `RHElementActorHuman`, `Rhelementactorhuman.cpp:8391` | Melee speech arbitration | E |
| `LevelBonusInitialFrame` | 1 | `engine/level_loading/entities.rs`: mission bonus spawn | `[0,row_frame_count)` once per successfully loaded bonus | `RHElementBonus::Initialize`, `RHElementBonus.cpp:683`; `RHSprite::ForceRandomSpriteFrame` | Mission entity ingestion | E |
| `MobileWaypointProbability` | 1 | `mobile.rs`: chariot waypoint macro selection | `[1,100]` once whenever a direction block is selected, including a single 100% block | `RHElementMobile::ExecuteWayPoint`, `RHelementmobile.cpp:529` | Mobile waypoint execution | E |
| `LuaMathRandom` | 3 | `robin_lua/state.rs`: `math.random` overloads | `[0,1)`, `[1,n]`, or `[a,b]`, exactly one draw per call | Post-Original Spellforge extension | Lua callback; deterministic scope required | E / Lua VM state remains PA-034 |
| `MacroRand` | 2 | `AiController::{calculate,forecast}_macro_rand` | `[1,100]`; forecast consumes once and calculate reuses it | `RHArtificialIntelligence::{Calculate,Forecast}MacroRand`, `RHartificialintelligence.cpp:6487,6507` | Patrol macro selection | E |
| `MeleeDegenerateDirection` | 1 | `engine/melee/evaluate.rs`: zero-distance move | `[0,16)` only for coincident actors | `RHElementActorHuman`, `Rhelementactorhuman.cpp:8537` | Melee distance maintenance | E |
| `MeleeInitiative` | 1 | `engine/melee/strikes.rs`: smalltalk initiative | `[0,100) <= relative ability`, before range fallback | `RHElementActorHuman`, `Rhelementactorhuman.cpp:8317` | Melee strike pass | E |
| `MeleeNonMutualGate` | 1 | `engine/melee/evaluate.rs`: non-mutual combat | `[0,100) < 10` fall-through | `RHElementActorHuman`, `Rhelementactorhuman.cpp:8342` | Melee evaluation | E |
| `MeleePrincipalReshuffle` | 1 | `engine/melee/evaluate.rs`: PC opponent update | `[0,3) == 0` only for PCs with at least two opponents | `RHElementActorHuman`, `Rhelementactorhuman.cpp:8294` | Melee evaluation | E |
| `MeleeProvoke` | 1 | `engine/melee/damage.rs`: hit reaction | `[0,100) < floor(0.2*fighting_ability)` after selected-PC exclusion | `RHElementActorHuman`, `Rhelementactorhuman.cpp:1508` | Damage handling | E |
| `MeleeStepBack` | 1 | `engine/melee/evaluate.rs`: step-back decision | `[0,100)` multiplied by opponents' ability | `RHElementActorHuman`, `Rhelementactorhuman.cpp:8833` | Melee evaluation | E |
| `NearSeekPoint` | 1 | `AiGlobalState::set_pos_on_near_seek_point` | `[0,candidate_count)` after stable seek-point scan | `RHArtificialIntelligence::SetPosOnNearSeekPoint`, `RHartificialintelligence.cpp:3360` | AI path planning | E |
| `NetWriggleGate` | 1 | `engine/animation.rs`: stuck-under-net cycle | `[0,31) == 0` on every motion-state visit | `RHElementActorHuman`, `Rhelementactorhuman.cpp:4763` | Animation execution | E |
| `OfficerSearchLook` | 1 | enemy officer-search substates | `[0,2)` at each authored look transition through one shared expression | `RHArtificialMalignity::ThinkExpectedEvent` officer search branches | Enemy expected-event handling | E |
| `PeasantReservistSurvival` | 1 | `EngineInner::convert_selected_peasants_to_blazons` | `[0,2*LIFEPOINTS_PC)` once per selected peasant in stable order | `RHGame::ConvertSelectedPeasantsToBlazons`, `RHgame.cpp:4220` | Campaign transition | E |
| `PhalanxAdvance` | 1 | enemy phalanx positioning | `[0,3) == 0` after enemy/front/protection gates | `RHArtificialMalignity`, `RHartificialmalignity.cpp:17674` | Enemy combat positioning | E |
| `PrincipalOpponent` | 1 | `engine/melee/swordfight.rs`: choose principal | `[0,candidate_count)` after stable opponent scan | `RHElementActorHuman`, `Rhelementactorhuman.cpp:8132` | Swordfight setup | E |
| `PurseCoinScatter` | 2 | `engine/purse.rs`: purse burst | Per each of seven attempts: direction `rand&15`, then magnitude `10+(rand&31)` | `RHElementPurse::Burst`, `RHElementPurse.cpp:152-153` | Live creation-ordered purse tick | E |
| `ReinforcementDoor` | 1 | `EngineInner::create_reinforcement` | `[0,door_count)` before peasant selection | `RHEngine::CreateReinforcement`, `RHengine.cpp:15933` | Deferred reinforcement tick | E |
| `ReinforcementJitter` | 2 | `EngineInner::create_reinforcement` | Per attempt: two ordered `-50 + 100*C-unit` floats, at most ten attempts | `RHEngine::CreateReinforcement`, `RHengine.cpp:15980-15988`; `Random` macro | Deferred reinforcement tick, after PC creation | E |
| `RuntimeBuildingExitWait` | 4 | `engine/movement.rs`: both movement sequence builders | At each authored exit: two ordered `[0,16)` draws summed to `[0,30]` | `RHSequence::AppendMoveToSequence`, `RHsequence.cpp:484,905` | Sequence construction | E |
| `ScriptRand` | 1 | `natives/mod.rs`: SCB `Rand` | Positive `max`: exactly one `[0,max)` draw; `max <= 0` panics and consumes none | `RHScript::Rand`, `RHScript.cpp:6502-6505`, is `rand()%iMaximum` with documented positive contract | Script native call order | E |
| `ScrollInitialFrame` | 1 | `EngineInner::initialize_all_scrolls` | `[0,row_frame_count)` once per scroll in entity order | `RHElementScroll::Initialize`, `RHElementScroll.cpp:153-171` | Engine initialize scope | E |
| `ScrollRevealFrame` | 1 | `engine/scroll_reveal.rs`: spawned amulet | `[0,row_frame_count)` after cached sprite load | `RHElementBonus::Initialize` / `RHSprite::ForceRandomSpriteFrame` | Deferred scroll reveal tick | E |
| `SeekPointAcceptance` | 1 | `ai_enemy/seek.rs`: next seek point | `[0,100)` only when point is not locked | `RHArtificialMalignity`, `RHartificialmalignity.cpp:12906` | Seek progression | E |
| `SeekPointDirectionPattern` | 1 | `SeekPoint::from_position` | `[0,4)` chooses one of four direction sets | Original seek-point construction uses process RNG; `RHartificialintelligence.cpp` | Level/AI initialization | E |
| `SeekPointSelection` | 3 | `ai_enemy/seek.rs`: seek list creation | Inclusive expected-count draw, then per point `[0,100)`; accepted points always draw insertion `[0,len]`, including empty | `RHArtificialMalignity`, `RHartificialmalignity.cpp:11924-11928`; expected-count `Random(min,max)` | Enemy seek planning | E |
| `SequenceRecordingBuildingExitWait` | 2 | `natives/mod.rs`: recorded movement path | Two ordered `[0,16)` draws summed to `[0,30]` | `RHSequence::AppendMoveToSequence`, `RHsequence.cpp:484` | Script/QA sequence recording | E |
| `SherwoodBeamMeShuffle` | 2 | `engine/level_loading.rs`: 100-swap loop | Exactly 100 pairs of `[0,beam_me_count)` after all returner placement rolls | `RHCampaign::CreateMissionCharacters`, `RHCampaign.cpp:1733-1749` | Sherwood mission ingestion | E |
| `SherwoodProductionBonusFrame` | 1 | `apply_production_sector_data` bonus spawn | `[0,row_frame_count)` per produced bonus | `RHElementBonus::Initialize` / `RHSprite::ForceRandomSpriteFrame` | Sherwood production scope | E |
| `SherwoodRelicFrame` | 1 | `apply_production_sector_data` relic spawn | `[0,row_frame_count)` per restored relic | `RHElementBonus::Initialize` / `RHSprite::ForceRandomSpriteFrame` | Sherwood production scope | E |
| `SherwoodReturningPcPlacement` | 2 | `roll_sherwood_placement` (axis expression is called twice) | Per returner: two ordered `-5 + 10*C-unit` floats, then `[0,16)` direction; all returners precede shuffle | `RHEngine::RandomizePosition`, `RHengine.cpp:16588-16604`, called before shuffle by `RHCampaign.cpp:1722` | Sherwood mission ingestion, team order | E |
| `ShieldAdvance` | 1 | enemy shield-danger substate | `[0,4) == 0` only while target still uses bow | `RHArtificialMalignity`, `RHartificialmalignity.cpp:4524` | Enemy expected-event handling | E |
| `SmalltalkStrikeSide` | 1 | `engine/melee/strikes.rs`: smalltalk strike | 50% after initiative/range gates | `RHElementActorHuman::TakeSmalltalkInitiative`, adjacent `rand&1` behavior | Melee strike pass | E |
| `SoldierBrawlCooldown` | 1 | enemy event handler after brawl | `300 + [0,32)` | `RHArtificialMalignity`, `RHartificialmalignity.cpp:5995` | Enemy stimulus handling | E |
| `SoldierFreedRotation` | 1 | `engine/soldier_helpers.rs`: free animation | `[-8,8]` | `RHElementActorSoldier::Translate`, `Rhelementactorsoldier.cpp:1338` | Soldier order translation | E |
| `SoldierNoiseCooldown` | 1 | enemy noise event handler | `70 + [0,60)` | `RHArtificialMalignity`, `RHartificialmalignity.cpp:8658` | Enemy stimulus handling | E |
| `SpecialActionRemark` | 1 | `AiEnemy::make_special_action_remark` | `[0,3) == 0` after shield/silence gates | `RHArtificialMalignity`, `RHartificialmalignity.cpp:20122` | Enemy periodic/special action | E |
| `SpriteBoredStart` | 1 | `Sprite::progress_frame` | Approximate 1/N start gate, only while frame and counter are zero | `RHSprite::Hourglass`, `RHsprite.cpp:787` | Entity animation tick | E |
| `SpriteSnakeStart` | 1 | `Sprite::progress_frame` | Approximate 1/N start gate, only while frame and counter are zero | `RHSprite::Hourglass`, `RHsprite.cpp:805` | Entity animation tick | E |
| `StonePiercingProtection` | 1 | `engine/combat.rs`: stone hit | `[0,100)`, soldier-only after VIP filter | `RHElementStone::Hit`, `RHElementStone.cpp:138` | Projectile impact | E |
| `SwordDamageProtection` | 2 | `combat.rs`: `receive_sword_damage` | When armed and not parrying: `[1,99]` cutting then `[1,99]` bludgeon, even if first succeeds/fails | `RHElementActorHuman::ReceiveSwordDamage`, `Rhelementactorhuman.cpp:8618,8652` | Damage handling | E |
| `SwordStrikeSelection` | 2 | `combat.rs`: `propose_good_sword_strike` | First `[0,100)` skill gate; conditional second PC parade gate, NPC short-circuits it | `RHElementActorHuman::ProposeGoodSwordStrike`, `Rhelementactorhuman.cpp:12406-12409` | Melee decision | E |
| `TitbitUpdate` | 1 | `EntityTitbitQuery::random_u32` | Full `u32`; consumers preserve `rand&3` and `rand%4` call sites | `RHTitbitManager::Hourglass`, `RHtitbit.cpp:1708-1724` | Tick tail, titbit creation order | E |
| `TooProudLook` | 1 | enemy too-proud overview | `[0,16) == 0` after state transition | `RHArtificialMalignity`, `RHartificialmalignity.cpp:4099` | Enemy expected-event handling | E |
| `VipIdleRemark` | 1 | enemy periodic work | `[0,12) == 0` after idle/default gates | `RHArtificialMalignity::The16thFrame`, `RHartificialmalignity.cpp:8988` | NPC periodic phase | E |
| `WaspDirectionTimer` | 1 | `engine/wasp_nest.rs`: wasp tick | `DIRECTION_CHANGE_TIMEOUT + [0,3)` after victim/direction changes | `RHElementWasp::Hourglass`, `RHElementWasp.cpp:447` | Live creation-ordered wasp tick | E |
| `WaspMovement` | 3 | `EngineInner::wasp_change_direction` | Per attempt: three ordered `[-6,4]` components | `RHElementWasp::ChangeDirection`, `RHElementWasp.cpp:193` | Wasp direction change | E |
| `WaspStingTimer` | 1 | `engine/wasp_nest.rs`: sting transition | `[1,STINGING_MAX_TIMEOUT]`; preserves Original's cancelling `MIN` expression | `RHElementWasp::Hourglass`, `RHElementWasp.cpp:498` | Live creation-ordered wasp tick | E |
| `WriggleDirection` | 1 | `engine/animation.rs`: wriggle start | `[0,3)` maps left/right/no turn | `RHElementActorHuman`, `Rhelementactorhuman.cpp:4801` | Animation start | E |

Guarded total: **84 semantic sites and 127 reviewed source uses**. Runtime
draw counts are data-dependent because several sites are loops or conditional
sequences; the table records those conditions and their internal order.

## Authoritative auxiliary randomness

This category is deterministic and gameplay-visible, but is deliberately
separate from the serialized draw stream. A temporary generator is seeded
from the current Engine RNG state; it cannot advance `EngineInner::control.rng`.
Snapshot coverage is instead provided by the authoritative output written to
campaign state.

| Auxiliary site | Uses | Rust location / function | Range and draw order | Original provenance | Owner / phase | Snapshot coverage |
| --- | ---: | --- | --- | --- | --- | --- |
| `AuxiliaryRngSite::PeasantNames` | 1 | `robin_rs/ui_panel.rs`: `generate_peasant_names` | For `MerryManA`, `B`, then `C`, skip a kind with a localized name; otherwise draw firstname `[0,firstnames.len())` then surname `[0,surnames.len())` per attempt, stopping at the first unused full name or after ten pairs. | Rust campaign/UI integration; no corresponding gameplay `rand()` caller was found in the reviewed Original sources. | Level setup, after localized name tables are loaded | The ephemeral auxiliary RNG is not serialized. Each accepted result is applied as `RegisterPeasantName`, entering serialized and state-hashed `campaign.peasant_names`; its seed is derived deterministically from `EngineInner::control.rng` without consuming it. |

Guarded auxiliary total: **1 semantic site and 1 reviewed source use**. Its
draw count is data-dependent on localized-name availability, prior registered
names, and collision attempts. These inputs and the fixed character order
must remain deterministic.

## Host-only randomness

These streams must not feed simulation state and are intentionally absent from
the Engine snapshot.

| Host location | Purpose | Isolation |
| --- | --- | --- |
| `robin_rs/game_session/tick.rs` and `mod.rs` | Sound-cache random group selection / audio jitter | Dedicated `sound_rng`; outputs are host playback choices only. |
| `robin_rs/ingame_menu/dialogue.rs` | Dialogue mouth-frame animation | Menu-owned RNG; rendering only. |
| `robin_rs/multiplayer/lobby.rs` | Lobby host token | Networking identity only. |
| `robin_engine/sbfile.rs` | Unique temporary test/overlay path | Filesystem plumbing in test-only code, excluded structurally with other `#[cfg(test)]` sources. |
| `robin_rs/examples/sprite_size_bench.rs` | Benchmark shuffling | Developer example, not game runtime. |

## Verified fixes and remainder

- Mission ingestion, SCB initialization, and Sherwood production now receive
  explicit Engine-derived contexts; direct dereferencing or TLS installation
  of `SimulationRng` was removed.
- Returning Sherwood PCs now consume two position draws and one direction draw
  in team order before the 100 two-draw beam-me swaps, matching
  `RHCampaign::CreateMissionCharacters`.
- Original floating `Random(-50,50)` reinforcement jitter and
  `RandomizePosition` use inclusive C-unit endpoint semantics rather than
  integer-only or half-open samples.
- Script `Rand(max)` rejects `max <= 0` loudly and without consuming a draw;
  valid calls remain one draw in `[0,max)`.
- PA-013 owner-tail views are prepared without forecast draws. Only the exact
  ambush, The16thFrame, macro, Charly, panic, or seek behavior that consumes a
  destination resolves its `BuildingExitGate` choice. Empty common drains,
  quiet/off-phase owners, and unrelated macro/FIFO work therefore remain
  silent with a live door-passing actor; draw-trace regressions cover those
  negative paths and the Original ordered all-gates rejection loop.
- Spellforge Lua still has the broader PA-034 state-policy gap: its RNG calls
  are inventoried and typed here, but Lua VM state is not yet part of Engine
  snapshots and deterministic modes must continue to reject unsupported Lua
  sessions until that contract exists.
