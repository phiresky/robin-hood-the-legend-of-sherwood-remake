# Mission achievement research

Research date: 9 September 2026. Story spoilers throughout.

This supplements [Achievement proposals](ACHIEVEMENT_PROPOSALS.md). The S-series remains **proposed, not accepted**. Its purpose is to identify interesting decisions in actual missions before approving or implementing their achievements. The accepted 24-achievement table is unchanged.

The strongest direction is to reward a rescue, a diversion, a well-timed assault, or an optional encounter with a particular person. Do not turn every character ability into a quota for every map. Mission badges should offer a new tactical problem on replay; a memorable discovery or a single boss duel usually belongs in the campaign achievement list once.

## What the new articles change

The new [third-party collection](third-party/README.md) includes both original summaries and fuller source material. I reviewed its index and research limitations, the mission walkthrough material, scoring and collectible guides, and reviews discussing repetition, stealth, character abilities, and camp management. A record marked “indexed text” remains weaker evidence than a complete walkthrough or script. Review opinions inform the design choices below; they do not establish engine behavior.

| Finding in the articles | Consequence for achievement design |
| --- | --- |
| Brad criticizes the repeated knockout–bind–hide routine; Clubic and Games.cz find forest raids repetitive. | Do not add more universal cleanup quotas. A forest achievement should demonstrate an alternative solution once or be tied to a distinctly different authored encounter. [Brad](third-party/reviews/brads-tech-talk.md), [Clubic](third-party/reviews/clubic.md), [Games.cz](third-party/reviews/tiscali-leon.md). |
| Gameindustry specifically enjoys forest traps, while other reviewers dislike repeated ambushes. | Preserve the fun of a well-executed trap without requiring the same setup in every convoy variant. This favors a one-time S6. [Gameindustry](third-party/reviews/gameindustry.md). |
| PC Games observes that revisiting a town can remain interesting because objectives, blocked routes, and guards change. | Bind a badge to the actual mission variant, not “any mission in York” or a background name. This supports distinct rescue, diplomatic, and siege challenges. [PC Games](third-party/reviews/pcgames.md). |
| Gamekult and Jeuxvideo criticize the ease of solving encounters through direct combat; Gamers’ Temple values the ability to recover from detection. | Offer stealth and selective intervention as optional challenges, while preserving room for improvisation. Do not combine every authored feat with Ghost. [Gamekult](third-party/reviews/gamekult.md), [Jeuxvideo](third-party/reviews/jeuxvideo-com.md), [Gamers’ Temple](third-party/reviews/gamers-temple.md). |
| The Russian retrospective finds some abilities underused and generic recruits overabundant; GRYOnline values the tradeoff between deployment and camp work. | The accepted one-time ability feats and Many Hands already address these issues. S10 should not add another three-target ability checklist. [Old-Games.ru](third-party/reviews/old-games-redwings.md), [GRYOnline](third-party/reviews/gry-online.md). |
| Carter distinguishes tax-collector robberies from ground-money convoys and describes victory at the money target without extraction. | Remove the invented escape stage from S7. Evaluate escort preservation when the mission succeeds. [Carter, section 05](third-party/guides/gamefaqs-swcarter.md#full-text). |
| Carter describes banner captures advancing Derby’s allied army, including allies killing incapacitated enemies. | Do not invent an optional “call reinforcements” button for Derby. S8 needs the actual signals in York, and any nonlethal siege challenge must specify how allied deaths count. [Carter, section 07.14](third-party/guides/gamefaqs-swcarter.md#full-text), [Chinese walkthrough record](third-party/guides/ali213-chinese.md). |

The [Steam scoring guide](https://steamcommunity.com/sharedfiles/filedetails/?id=2829972690) also reports unreachable soldiers, pre-existing corpses, and a scripted friendly conversion in several missions. These are leads for an eligibility audit, not proof that every soldier can be cleared. They particularly affect the accepted **Ruthless** badge: retain its condition, but do not offer it on a mission until its complete required enemy population is demonstrably killable. The same guide distinguishes Ranulph’s soldiers from the Sheriff’s men in The Evening Visitor. The script audit below resolves that restriction more precisely.

## Revised S1–S10 assessment

“Keep” means **keep as a candidate**, not user approval or verified solvability. Every condition below includes successful mission completion. The mission-badge choices change the route or the order of objectives; the campaign-only choices reward one distinct feat or discovery.

| ID | Revised recommendation | Scope | Evidence and remaining gate |
| --- | --- | --- | --- |
| **S1 — Not Just Stutely** | Rescue Stutely and all three named-by-script generic prisoners; each must be freed, alive, and extracted. | Mission badge | `S01_Not_VL`, Nottingham. Ordinary victory accepts dead generic prisoners, so this is already a meaningful extra challenge. No added damage restriction. Raw victory branches verified. |
| **S2 — A Courtesy Call** | Complete Ranulph’s meeting and departure without player-caused harm, incapacitation, or restraint of his soldiers. | Mission badge | `H04_Lei_VL`, Leicester. Its protection group is explicit; five Sheriff soldiers are excluded. Ordinary success already forbids protected deaths but permits knockouts. **TODO:** prove a route without incapacitation. |
| **S3 — An Awkward Interruption** (replace *The Guests May Stay*) | Follow the Sergeant Buttler/Mary Wonara opportunity, retrieve the Ampulla, and win. | Campaign achievement, once | `S05_Yrk_EC`, York. This is an actual optional encounter. The original untouched-everyone-except-Guy restriction would prohibit this side quest and has no demonstrated route. |
| **S4 — The Sheriff Gets His Due** | Defeat the Sheriff in the final authored duel without Robin losing health during its combat phase. | Campaign achievement, once | `H12_Not_MP`. There is a precise dialogue-to-combat transition and a Sheriff-death event. This is the same boss encounter on every replay, so use one permanent award. **TODO:** verify damage tracking through retreat, healing, and re-entry. |
| **S5 — A Taxing Misunderstanding** | **Drop the original condition.** | None recommended | Clean Hands plus a surviving collector adds little to Clean Hands on a collector robbery. The collector does not need to die for the money objective. A trap-specific robbery could replace it, but overlaps S6 and lacks a proof route. |
| **S6 — The Forest Does the Work** | Use two distinct authored trap mechanisms, each successfully disabling or removing a distinct escort, and win without any PC directly striking an NPC. | Campaign achievement, once | Start playtesting with `Emb07_FoB_JMS`, which has multiple nets and logs; `Emb02_FoC_MK` also has nets/logs. Empty activations do not count. Arrows at mechanism targets are allowed. This showcases a solution once, without repeated trap quotas. |
| **S7 — No Need to Clear the Road** | Win a robbery with at least half its original escort alive and never incapacitated or removed by a player-triggered trap. | Mission badge on individually validated variants | Keep the alternative objective-first route; remove “escape.” Freeze the actual active escort, not every loaded soldier. **TODO:** publish each roster and prove each qualifying variant can end with the required survivors. |
| **S8 — Open for Business** | Open both eastern gates before the first raising of the keep’s signal flag, then win. | Mission badge | `Str03_Yor_MK`, York only. Actual gate and flag events exist. The bell is a separate signal; no extra bell-order restriction. **TODO:** prove the ordering under supported preparation states. |
| **S9 — Open Both Ways** (replace *A Castle, Not a Graveyard*) | Personally open the garden secret passage and lower the drawbridge before victory. | Mission badge | `Str01_Lin_EC`, the conditional Lincoln assault. Two concrete routes for allied forces replace the arbitrary two-objective quota. **TODO:** verify preparation and automatic victory cannot complete the mission before both actions remain possible. |
| **S10 — I’ll Take My Prize Anyway** (replace *Marian Knows Best*) | Physically collect the Silver Arrow and escape its mission successfully. | Campaign achievement, once | `H07_Not_MK`. Actual prize pickup sets a distinct campaign bit. The previous three-target spy quota is another repeated ability checklist; this optional story detour is more specific and memorable. |

**Recommended proof-run order:** S1, S8, S10, then S2 and S9. S1 has the clearest verified difference from ordinary victory; S8 changes an actual siege plan; S10 is a well-supported optional discovery. S6 and S7 need the most careful trap/escort definition. S3 and S4 have clear story hooks but still need effect/window validation.

### Rescue and diplomacy: what ordinary victory actually allows

**S1:** `S01_Not_VL.scb`, `StartUp.CheckVictoryCondition`, starts at quad `1105`. Stutely is actor `99`, the three generics are actors `100–102`, and all use extraction locations `13–16`. The generics have explicit dead-instead-of-extracted branches at `1210–1215`, `1285–1290`, and `1360–1365`. Require successful release/recruitment, survival, and extraction: Stutely’s release uses mission message `0`, and the generic releases use message `1`. They need not all use the same exit. This corrects the original proposal’s Leicester location and removes its unnecessary suggestion to add zero damage. [Carter, section 07.03](third-party/guides/gamefaqs-swcarter.md#full-text).

**S2:** `H04_Lei_VL.scb` marks `Crossbowman02`, `Guard_B02`, `Guard_B02_2`, `Guard_B02_3`, and `Officier_B02` with custom NPC slot `4 = 1`. These are the Sheriff’s exception group. `StartUp.CheckVictoryCondition` starts at `795`; protected soldiers are filtered by slot `4 == 0` at `817–827`, deaths tested at `829–837`, and **any civilian death** checked at `844–848`. Successful departure also needs the completed meeting and the living party at the exit. Preserve the authored group even if attitudes change during the mission. The proposed badge adds no harm/incapacitation to that existing no-death rule; it must not simply duplicate the mission’s success flag. [Carter, section 07.10](third-party/guides/gamefaqs-swcarter.md#full-text).

### Wedding and duel: separate the optional adventure from required progression

**S3:** `S05_Yrk_EC.scb` has the flag diversion in `target_mat_declenchement_800003d5.ActivatedByHand`: an animation sequence sends mission message `10`, completes the diversion briefing, and adds the duel objective. This is intended main progression, so merely raising the flag is weak as an achievement. Guy’s rescue transition checks `IsActorHS(Guisbourne)`, not specifically death. Do not impose “kill Guy” based only on a walkthrough’s language.

The optional affair is a better candidate: the script places the Ampulla at `Officer04` when he becomes out of order and separately observes its collection. The opportunity involves getting Mary’s husband away so Buttler emerges. Track completion of that encounter and the actual pickup, rather than paying any beggar or merely disabling an officer. **TODO:** verify the exact reveal/emergence/pickup sequence and whether bypasses should qualify; do not require harming the husband when distracting him works. [Carter, section 07.15](third-party/guides/gamefaqs-swcarter.md#full-text).

**S4:** `H12_Not_MP.scb`, `Duel_80000633.EnterZone`, admits Robin and stages the conversation before unlocking combat. In raw bytecode, `RecordPlayDialog(0)` occurs at `141–142`, then Sheriff actor `97` is unlocked at `158–162` and the user at `164`. The achievement’s health-loss window begins when that queued transition **executes**, not when the script schedules it. End the window at the Sheriff’s actual death; award only after final victory. Earlier mission injuries are permitted. Healing or leaving/re-entering the chamber cannot restart a failed window. [Carter, section 07.19](third-party/guides/gamefaqs-swcarter.md#full-text).

### Forest robberies: money, trap effects, and the real escort

**S5/S7:** `Emb02_FoC_MK` completes when the designated collector’s money is exhausted. `Emb07_FoB_JMS` and `Emb08_FoA_JMS` combine their treasure condition with the collector’s empty money property; `Emb04_FoA_MP` checks four ransom pickups. These are different objective predicates, not a shared “all escorts defeated, then extract” system. The victory predicate, rather than a later signpost, is the comparison point. [Carter, section 05](third-party/guides/gamefaqs-swcarter.md#full-text).

**S6/S7:** the nets in `Emb07_FoB_JMS` and `Emb02_FoC_MK` lock victim AI and move victims to location `-1`. Emb07’s net helper also confiscates the collector’s money; Emb02 has money confiscation in other trap paths, not in that net helper. These removals need not inflict damage or set ordinary unconsciousness. Logs can call `InflictPain` and then relocate victims. Therefore:

- A netted-and-removed escort counts as a successful trap victim for S6 and a removed escort for S7.
- An empty net activation proves neither success nor incapacitation.
- The same escort cannot satisfy both of S6’s distinct-victim requirements.
- A scripted hidden ally is different from a PC directly striking someone. S6 permits that authored assistance; it is not advertised as nonlethal.
- A soldier loaded for an unused variant or an off-map allied force is not part of S7’s initial escort. Late reinforcements need an explicit rule for each allowed scenario.

**TODO:** record initial escort identities, trigger-to-victim links, and a proof run before enabling either candidate. “At least half” means rounding upward. Restricting S6 to a one-time campaign achievement also matches the conflicting review reception: traps are enjoyable, but repeating similar ambush setups becomes work.

### Two sieges with actual objectives

**S8, York:** `Str03_Yor_MK.scb`, `StartUp.Hourglass` starts at `3595`. The two eastern gate patches are `pixel_vert_pixel_vert_2` and `pixel_vert_pixel_vert`; the script awards `Blazon_10`/`Blazon_11` and sets `mbInnerPortalOpen`/`mbOuterPortalOpen` at `3617`/`3642`. The keep flag sets `mbBannerSignalSent` at `3906` in `ProcessMessage`; the cathedral bell has its own signal. Read the actual patch state when the **first flag event executes**, so polling order cannot reject gates opened earlier in the same simulation frame. The script explicitly warns when the flag is raised before eastern access is open. Opening a gate can itself advance allies, so “before any allies arrive” would be the wrong condition.

**S9, Lincoln:** `Str01_Lin_EC.scb` names the garden passage and drawbridge as an allied pincer plan. `Jardin_800002af.IsTaken` activates the associated allied force, awards `Blazon_2`, sets global `4`, and completes briefing `2`. `mecanisme_pont_levis_8000024c.ActivatedBySword` applies `Lincoln_Pont_levis`, awards `Blazon_4`, sets global `10`, and completes briefing `3`. Require both actual player-triggered objectives in the attempt; an inherited banner total is insufficient. This mission is conditional, so its badge must not silently become a requirement for every completed campaign. [Scoring guide’s conditional Free Lincoln entry](https://steamcommunity.com/sharedfiles/filedetails/?id=2829972690).

## Additional story discoveries worth considering

These are alternatives to weak S candidates, **not additions to the accepted list**. All require a successful relevant mission; the cross-mission loan requires both events in the same campaign’s retained progression.

| Working proposal | Scope | Actual gameplay and proposed award |
| --- | --- | --- |
| **A Friend on the Inside** | Campaign achievement, once | In the final Nottingham mission, obtain Applegoad the Old’s help, receive his son’s safe-passage response, and win with the son alive. This rewards finding an ally where the obvious solution is another takedown. A further alternative to S10’s former generic spy quota. |
| **Back on His Feet** | Campaign achievement, once | Help the bankrupt tradesman during the wedding mission, then meet him again and collect his repayment during The Letter. This is a small story spanning two visits to York. It is distinct from Nothing in Return: that accepted achievement is an unrewarded donation; this one follows a specific person’s recovery. |
| **I’ll Take My Prize Anyway** — revised S10 above | Campaign achievement, once | Escape The Silver Arrow with the actual Silver Arrow collected. The tournament is a trap, and recovering its prize is an optional detour. Do not substitute a guessed “perfect tournament score” or require every royal collectible. |

### Applegoad: an actual alternative to removing a guard

**Mission:** `H12_Not_MP`, the final Nottingham confrontation, called *Last Challenge* in Carter’s walkthrough.

The script starts with the friendly soldier off-map. `AppleGoadVieux_8000080d.IsTaken` checks that the existing soldier is not out of order, moves a replacement soldier into his position, removes and locks the original, attaches `AppleGoadJeune`, and sets mission global `1000` to `1`. The son’s scroll then supplies the safe-passage response. This is an authored identity substitution, not a generic diplomacy conversion event.

**Raw evidence:** in `H12_Not_MP.scb`, class `AppleGoadVieux_8000080d`, function `IsTaken` starts at quad `6`; the actor-98 `IsActorHS` gate is at `11–21`, replacement of actor `98` by actor `71` at `28–88`, and global `1000 = 1` at `90–93`. Class `AppleGoadJeune_8000080e.IsTaken` starts at `5`, detaches the interaction at `6–12`, and displays response `14` at `15–16`. These are class-local quad addresses, not file byte offsets.

**Award evidence:** record the successful father branch and the son’s completed response, then check survival of the *replacement* actor at victory. Merely seeing an actor disappear would incorrectly treat the helpful branch as a death. **TODO:** verify a playable route through the newly permitted passage and the replacement actor’s full faction/control state. Do not describe him as a recruited Merry Man. The walkthrough corroborates the father-and-son side quest. [Carter, section 07.19](third-party/guides/gamefaqs-swcarter.md#full-text).

### The tradesman: two missions and a retained campaign flag

**Missions:** `S05_Yrk_EC` (*A Wedding and a Funeral*) → `H10_Yor_VL` (*The Letter* / *Lackland’s Plan*).

The wedding script’s loan response sets custom campaign slot `4` to `1`. The later mission tests this exact slot, substitutes the recovered tradesman for a beggar, and attaches `Good01`. Taking that response activates `Bonuses.Ransom_2` and completes briefing objective `7`. Carter describes the repayment as £2,500; the script evidence here establishes activation of the reward, not its amount.

**Raw evidence:** `S05_Yrk_EC.scb`, `StartUp.ProcessMessage`, quads `1096–1129`: loan response, ransom deduction, `SetCustomCampaignValue(4, 1)`, amulet activation, and local conversation state advance. `H10_Yor_VL.scb`, `StartUp.Initialize`, `427–462`: the slot-4 branch, beggar actor `82` deactivation, and `Good01` actor `226` attached to tradesman actor `83`. Class `Good01_800002ad.IsTaken`, `5–28`: response, activation of reward actor `202`, objective completion, and detachment.

**Important edge case:** the wedding script deducts £2,000 when available but otherwise takes the remaining ransom money. Therefore the condition should be **complete the authored loan**, not “spend exactly £2,000.” Require the later reward’s actual collection, not just its activation. Save/load is allowed; practice on an unrelated campaign must not invent the prior loan. The dialogue’s reference to a later visit to Nottingham conflicts with the actual York mission pairing; use mission IDs. [Carter, sections 07.15 and 07.17](third-party/guides/gamefaqs-swcarter.md#full-text).

### The Silver Arrow: a prize pickup, not an invented minigame score

**Mission:** `H07_Not_MK`.

Carter’s route treats this as escaping the tournament trap, with retrieving the guarded arrow as an optional diversion. The [collectible guide](https://steamcommunity.com/sharedfiles/filedetails/?id=792309796) independently identifies it as a physical pickup distinct from the Sword of State. The script’s `Fleche_d_argent_80000771.ActivatedByHand` deactivates the arrow target and sets bit `8192` (`0x2000`) in custom campaign slot `0`.

Record the actual pickup transition and successful escape. An imported bit alone is not evidence of a new eligible attempt. A full regalia/bonus-ending achievement remains a separate, unapproved idea: the [Russian collectible record](third-party/guides/square-faction-relics.md) distinguishes the Silver Arrow from the seven royal regalia while including it in the reported ending requirement. **TODO:** verify the ending script before proposing an exact collection predicate.

## Evidence, reproduction, and remaining work

The script subagent inspected the full Linux mission scripts and returned the findings summarized above. The parent independently reviewed article evidence, the named siege/duel hooks, and raw bytecode for the Applegoad and tradesman branches. There were **no gameplay proof runs** and no assertion that the other full-game packs, demos, or custom missions share these predicates. Script findings establish authored opportunities and conditions; they do not establish that a restrictive route is fun or feasible on every difficulty.

Inputs are the `.scb` files in [the Linux level directory](../datadirs/fullgame_linux/Data/Levels). The existing [disassembler/decompiler](../crates/robin_rs/examples/disasm_scb.rs) was run without rebuilding or modifying game code:

```sh
target/debug/examples/disasm_scb --decompile \
  --datadir datadirs/fullgame_linux \
  --out-dir target/achievement-mission-research \
  datadirs/fullgame_linux/Data/Levels/*.scb

# Raw disassembly for a critical condition; omit --decompile.
target/debug/examples/disasm_scb \
  datadirs/fullgame_linux/Data/Levels/S01_Not_VL.scb
```

Generated files in `target/` are disposable inspection aids, not committed sources. The pseudo-source omitted both S1’s fourth extraction location and S2’s civilian-death check; raw disassembly exposed them. Other output includes commented-out jumps. **Never implement an achievement predicate by blindly translating the decompiled TypeScript.** The class/function names, literal IDs, and raw addresses in this report allow the consequential claims to be checked against the original binary. All addresses are class-local quad indices.

| Inspected script | SHA-256 |
| --- | --- |
| `S01_Not_VL.scb` | `866301b5283400a1f54147488458c43499044519a0b62a06ae5e591847906c41` |
| `H04_Lei_VL.scb` | `42b6ad56afaae38e3e2dc71544e8b92e2f24ab032e2ae91f489f8970b40b9129` |
| `S05_Yrk_EC.scb` | `a6216abb95ae875915944c3c941e65c13ffc4829d53467c983f892a6d70f02f6` |
| `H12_Not_MP.scb` | `289f232c2c3cadd8dd7e8903427d4529170a619c6c286c926c5183292f3d8829` |
| `H07_Not_MK.scb` | `a7c8f996fc1395e2fa5fa349807f77040769cecef1890125ab3c91dd141c9868` |
| `H10_Yor_VL.scb` | `2b5e2d82e7fd6a0313094a01153a1893fbaffbad250c3acc34954bc8f324cf95` |
| `Str01_Lin_EC.scb` | `16ea754ccc01d493100a3f260777cc02f4dd71084bd850b69b15a13ed9f816d6` |
| `Str03_Yor_MK.scb` | `938e85a0d665790e336bcab4d34bb148d7d395af253d64dd71a80cefe226bb36` |
| `Emb07_FoB_JMS.scb` | `81b4061e4caf91e0d5184d58155b67d150ddfeff83fbef54ccfb7977bcd13f01` |
| `Emb02_FoC_MK.scb` | `19950db667dbaa3371c2ac063f98705c3e6f52646c33950f4fa9e584d99d512b` |

Before approval for implementation, finish the TODOs attached to each candidate: validate raw control flow, publish exact mission/variant eligibility, and record a successful proof run with an ordinary roster and preparation state. Then test failed interactions, healing after damage, prisoners dying after release, scripted actor substitution, trap removal, duplicate pickup/trigger events, and victory occurring in the same tick as a qualifying event.

The newly added [pre-existing corpse discussion](third-party/guides/steam-spared-lives-report.md) reinforces why old displayed spared-life percentages cannot substitute for causal achievement evidence. The [ambush dialog-loop report](third-party/technical/steam-ambush-dialog-loop.md) is an unresolved reproduction lead, not proof of a particular script defect; qualify the achievement on canonical successful completion rather than a popup. The [camp-production discussion](third-party/guides/steam-camp-production-report.md) likewise supports testing mission-type boundaries for the already accepted Many Hands, without changing its scope. None of these findings approve extra achievements or alter the accepted table.
