# Achievement proposals for Robin Hood: The Legend of Sherwood

Research date: 9 September 2026. Scope: the original game's identity, the current Rust port, campaign achievements, and individual-mission challenges. This is a design report; nothing below adds achievements to the implementation. Story spoilers follow.

**Recommendation**

Expand around **clever outlawry, teamwork, generosity, and preparation**. The strongest additions would make players discover something enjoyable about this particular game: using a beggar's information, passing a guard through a four-person capture operation, getting a friend over a wall, ambushing a convoy without a massacre, or opening a castle for allied troops.

Keep three distinct experiences: accessible story milestones, replayable mission badges, and a smaller set of campaign accomplishments. Do not make every mission inherit every challenge. A forest ambush, a diplomatic visit, and a castle assault should have different achievement menus.

This report proposes **34 candidates: 10 campaign achievements, 14 reusable mission badges, and 10 achievements tied to particular missions or mission families**. Start with the 12-candidate shortlist near the end. Names and numerical thresholds are proposals, not historical achievement names or tested balance values.

**What already exists**

The authoritative [achievement definitions and evaluator](../crates/robin_engine/src/achievement.rs), [engine hooks](../crates/robin_engine/src/engine/achievements.rs), and [feature documentation](NEW_FEATURES.md) establish four existing achievements:

| Existing achievement | Actual mission condition | Campaign/lifetime behavior |
| --- | --- | --- |
| Clean Hands | No NPC deaths attributed to player-controlled units. A setting can additionally invalidate it for NPC-on-NPC deaths; environment/script deaths are classified separately. | Every required mission in one completed campaign path must have eligible evidence. |
| Ghost | No living hostile NPC actually observes a player character. This is independent of killing and is not simply a count of alarms. | Same campaign-path aggregation as Clean Hands. |
| Pile-o-Bones | At some point, at least ten out-of-order NPCs occupy one building. The success latches; these need not be dead or all hostile. | One eligible mission suffices. |
| All Enemies Stashed | Every encountered hostile is out of order in the same building when the final result is evaluated. | One eligible mission suffices. |

Consequently, ordinary “finish without killing,” “finish unseen,” and “hide ten bodies” are already covered. The largest gaps are positive actions, character identity, story progress, campaign management, and objectives specific to a location.

The [campaign history design](CAMPAIGN_HISTORY.md) already supports eligible practice attempts that earn missed mission badges without awarding the campaign's money or progression twice. A completed campaign freezes its own required mission set; practice can fill missing badge evidence for that set. This is a forgiving **campaign mastery collection**, not proof that the original playthrough never broke the condition. New descriptions must preserve that distinction.

**What the research suggests**

The local interviews are primary evidence of design intentions, sometimes recorded before release. Reviews describe particular reviewers' experiences with the released original. Neither proves that a mechanic or mission variant works identically in every current datadir. Implementation observations below come from the repository; proposal rules are my recommendations.

| Evidence | Implication for new achievements |
| --- | --- |
| Martin Kuppe describes mercy affecting recruitment, nonlinear mission selection, Sherwood production, and a concrete apple → knockout → binding → carrying operation involving four heroes. [XGR interview, August 2002](interviews/08-martin-kuppe-xgr.md) | Reward the operation and the band it builds. Avoid treating kill totals as the principal measure of mastery. |
| Haessig and Devouassoux discuss distinct hero abilities, intelligence gathering, terrain assistance, three-stage quick actions, and infiltration, ambush, and siege objectives. [HomeLAN interview, August 2002](interviews/06-haessig-devouassoux-homelan.md) | Give support abilities and mission preparation their own achievements, alongside combat and stealth. |
| The visual-design discussion stresses colorful, playful adventure and selective adaptation of historical material. Staff favorites include Little John's staff tricks and Tuck's beer. [Visual-design Q&A](interviews/09-jean-marc-haessig-action-vault-qa.md), [character Q&A](interviews/10-action-vault-favorite-character-qa.md) | Use warm, mischievous names and a few comic feats. Environmental spectacle fits; cruelty to civilians does not. |
| Brett Todd praises varied objectives, squad combinations, traps, and the ransom/siege economy, while noting difficult navigation and civilian alarms. [GameSpot review, 12 November 2002](https://www.gamespot.com/reviews/robin-hood-the-legend-of-sherwood-review/1900-2897317/) | Include positive exploration and preparation challenges, with visible conditions that prevent mysterious failures. |
| Westlake criticizes repeated maps and similar rescues, and reports relying heavily on Little John rather than Robin. [Game Over review, 24 February 2003](https://www.game-over.com/reviews/pc/Robin_Hood:_The_Legend_of_Sherwood.html) | Change the tactical question on revisits; encourage alternatives to one dominant character without banning him throughout a campaign. |
| Tony Mitera enjoys convoy ambushes and describes the value of supporting skills, but finds objectives unclear at times. [WorthPlaying review, 16 December 2002](https://www.worthplaying.com/article/2002/12/16/reviews/7395-pc-review-robin-hood/) | Showcase ambush choreography and give players explicit challenge feedback. |
| Jarkendia's retrospective criticizes map reuse and underused abilities while enjoying newly discovered routes, including rooftops. [VidaExtra retrospective, 12 June 2020, Spanish](https://www.vidaextra.com/analisis/robin-hood-leyenda-sherwood-analisis-review-precio-experiencia-juego-para-pc) | Route and ability challenges are a better answer to repetition than requiring dozens more identical ambushes. |

These sources disagree about how varied the campaign feels. The useful design inference is that achievements should expose alternative solutions already present in the game. They cannot compensate for missing routes or weak objectives by simply demanding more repetitions.

**Lessons from other games**

| Game and verified precedent | Adaptation for Robin Hood | What to avoid copying |
| --- | --- | --- |
| **Shadow Tactics: Blades of the Shogun** has Five Shadows for executing a plan involving all five characters, location interactions such as hiding in a wagon, and Complete Mastery for collecting all badges. [Official Steam achievement list](https://steamcommunity.com/stats/418240/achievements) | Reward coordinated quick actions and interactions that teach a map's possibilities. Use mission badges beneath a smaller campaign/profile layer. | Its large kill/body-hiding totals would add little to our existing body achievements. Requiring every badge makes experimental or awkward challenges mandatory for completionists. |
| **Desperados III** includes Sheriff's Badge for 90 badges, a single-mission top-difficulty achievement, distinctive environmental feats, and achievements for the Baron's Challenges. [Official Steam achievement list](https://steamcommunity.com/stats/610370/achievements) | Offer a selection of mission challenges and an attainable collection milestone. Keep substantial remixed scenarios separate from ordinary achievement conditions. | Large repetitive counters and importing mechanics absent from Robin Hood, such as footprint-following or mind control. |
| **Dishonored** separates campaign nonlethal/stealth accomplishments from individual-mission feats and also recognizes escaping pursuers without killing them. [Official Steam achievement list](https://steamcommunity.com/stats/Dishonored/achievements) | Add a recovery achievement: a spotted attempt can still become an interesting success. Keep stealth and mercy conditions independently legible. | Another pair of badges that merely duplicates our Clean Hands and Ghost. Their names do not guarantee identical rules across games. |
| **HITMAN** uses explicit combinations of restrictions such as Silent Assassin and Suit Only. IOI's patch notes document both live HUD improvements and fixes to ambiguous objective wording. [February 2021 notes](https://ioi.dk/hitman/patch-notes/2021/february-patch-notes), [May 2023 notes](https://ioi.dk/hitman/patch-notes/2023/hitman-woa-may-patch-notes) | Compose a few clear restrictions and show exactly when they fail. A compound badge must be earned in the same attempt. | Hidden exceptions, ambiguous equipment categories, or conditions explained only after mission completion. |

Steam lists verify achievement examples, not the complete internal rules of each game. Their changing unlock percentages are not used as evidence that a proposed Robin Hood threshold will be fun or appropriately difficult.

**Rules shared by the proposals**

- **Mission:** earn on successful completion, even when the feat happened earlier. A failed or abandoned attempt does not award it. Count successful effects, not button presses.
- **Campaign:** evaluate within one campaign identity. Campaign progression and campaign challenges can require additional evidence beyond mission badge unions.
- **Collection:** allow eligible practice attempts, but do not combine incompatible conditions from different attempts. Replaying one mission is not visiting a new region or recruiting another person.
- **Difficulty:** ordinary achievements work on any supported difficulty. A dedicated hard-mode achievement records the qualifying difficulty throughout each relevant attempt.
- **Availability:** each badge has a mission/variant allowlist. Missing required allies, objects, routes, or targets makes it unavailable, not automatically earned.
- **Tracking:** distinguish in progress, failed, earned, unavailable, and unverifiable. Unknown historical evidence must not become zero deaths, zero damage, or success.

Effort labels below are preliminary: **S** means mainly existing completion/history data; **M** means new authoritative event tracking; **L** means mission-specific script/route definitions or more complex attribution. They are relative estimates, not implementation promises. **TODO** identifies an audit or design gate before shipping.

**Campaign candidates — 10**

| ID / working name | Proposed condition | Why it belongs / evidence and effort |
| --- | --- | --- |
| C1 — **A Legend Is Born** | Complete the full campaign once, using the canonical campaign-completion event. | Accessible recognition for finishing; does not demand perfection. **S.** No fixed mission count; demos cannot substitute for the full campaign. |
| C2 — **For King Richard** | In one campaign, reach the authored successful ransom-payment/rescue milestone. | Celebrates the central economic purpose. **M/L. TODO:** bind the actual script milestone; neither a guessed coin threshold nor a briefly high wallet balance proves payment. If this is indistinguishable from C1 in shipped data, merge them. |
| C3 — **The Whole Merry Company** | Finish a campaign after recruiting all five other named heroes and winning at least one mission with each under player control. | Gives Robin, Marian, John, Tuck, Scarlet, and Stutely a place in the campaign story. **M.** Use character identity, not localized names; merely escorting a prisoner does not count as deployment. |
| C4 — **No Empty Places at the Table** | Complete the campaign with no permanent loss of any recruited named hero or generic Merry Man in the retained progression history. | Values the supposedly replaceable members of the band. **M/L.** Includes losses on strategic assignments; capture alone is not death. A later practice win cannot erase an earlier campaign loss. Save/load is allowed. |
| C5 — **Friends in Every Town** | Complete the campaign having received a paid beggar-information response in each of Lincoln, Leicester, Derby, York, and Nottingham. | Connects the geography with helping and listening to ordinary people. **M/L. TODO:** verify a reachable paid interaction in every town on supported campaign paths; otherwise publish a smaller named set. Practice can fill interaction evidence. |
| C6 — **Many Hands Make Sherwood** | Complete the campaign after three distinct generic Merry Men have each contributed to a successful deployed mission and completed a productive or training assignment at headquarters. | Makes both halves of the band matter. **M.** Track real work completion and useful mission effects; exclude idle deployment. Three is a provisional modest threshold. |
| C7 — **A Well-Laid Campaign** | Before each castle assault actually undertaken in the completed campaign path, earn at least one preparation benefit from an associated optional mission. | Rewards connecting ambushes and strategic preparation to a later battle. **L. TODO:** enumerate valid benefit-to-assault links and require at least one qualifying assault. Purchased support alone does not meet this rule. |
| C8 — **Protector of the People** | Finish a campaign with no player-attributed civilian death in its retained progression history. | Allows fighting soldiers while protecting the people Robin serves; more approachable than campaign Clean Hands. **M/L.** Needs civilian identity and indirect-cause tracking. Later practice cannot repair the original history. |
| C9 — **Against All Odds** | Complete the campaign with a qualifying hard-difficulty success for every mission in its frozen required path. | One clear prestige goal for experienced players. **M.** Practice may fill it, like current campaign mastery badges; wording must say so. Difficulty changes within an attempt invalidate that attempt's hard evidence. |
| C10 — **Ballads of Sherwood** | In one completed campaign archive, earn one new mission badge on each of ten distinct canonical missions, covering infiltration/rescue, ambush, and siege. | A varied, forgiving collection goal. **M/L.** Practice allowed; no repeated farming of one mission. **TODO:** validate mission-family tags and ten eligible missions; freeze the eligible badge catalogue so future additions do not move the target. |

C4 and C8 deliberately represent the history the player carried forward. They need a different aggregation policy from the existing “fill missing badges later” campaign achievements. Neither should be advertised as a no-reload or ironman run.

**Reusable individual-mission candidates — 14**

These are templates applied only where the necessary opportunities exist. Most should remain mission badges; a few particularly expressive feats can additionally unlock a one-time profile achievement.

| ID / working name | Proposed condition in one successful attempt | Purpose / implementation gate |
| --- | --- | --- |
| M1 — **In and Out** | Complete an eligible infiltration with Ghost and with no player-caused damage, knockout, binding, net capture, or other incapacitation of an NPC. | A real extension beyond Ghost or Clean Hands: solve the route without removing guards. Distractions allowed. **M/L.** Exclude forced combat and audit indirect effects. |
| M2 — **Everyone Comes Home** | Every deployed character and every designated rescued companion survives and satisfies the mission's return condition. | An approachable team objective; injuries and revival allowed. **M/L.** Offer only where partial losses are compatible with ordinary victory, otherwise it merely duplicates completion. |
| M3 — **Not a Scratch** | No deployed or newly player-controlled companion loses any health after their controllable baseline. | Tactical precision distinct from nonlethal or unseen play. **M.** Healing cannot undo damage; exclude scripted damage before control is granted. |
| M4 — **A Penny Well Spent** | Pay a beggar and successfully receive one information response. | A gentle discovery achievement. **M.** Count the completed response, not a failed interaction or just money deducted. |
| M5 — **Pass the Parcel** | Four different characters successively distract, knock out, bind, and carry the same hostile into a building; that hostile is alive at mission end. | Directly realizes Kuppe's teamwork example. **L.** Link the distraction and later effects to the same target; actions need not happen within an arbitrary short timer. |
| M6 — **On My Mark** | In one quick-action execution, at least three characters successfully perform a non-movement tactical action against three distinct hostile targets. | Teaches coordination without requiring a five-person roster or kills. **M/L.** Track execution-group identity and action success; rejected commands do not count. |
| M7 — **A Leg Up** | Use an ally's assistance to complete a climb to another elevation. | Recognizes terrain cooperation rather than another takedown. **M.** Requires an actual completed assisted traversal, not invoking the command. |
| M8 — **Robin Takes the High Road** | Robin completes two distinct authored rooftop-jump links in an eligible town mission. | Encourages discovering the vertical map. **M/L.** Unique link IDs prevent repeating one jump. **TODO:** confirm two useful links in each allowed variant. |
| M9 — **You Never Saw Us Leave** | After at least three hostile soldiers concurrently pursue the party, break all those pursuits without killing any pursuer, then complete the mission. | Rewards recovering from detection. **L.** Pursuers must remain alive; their AI must return to a non-pursuit state while the party remains on-map. Exiting immediately cannot manufacture the escape. |
| M10 — **String Theory** | Finish with three distinct player-knocked-out hostiles alive, bound, and inside a building. | An accessible capture challenge, differentiated from Pile-o-Bones by survival and restraint. **M.** End-state check; no repeatedly counting one guard. |
| M11 — **A Round on the Friar** | Three distinct hostile soldiers successfully consume beer placed by Tuck. | Showcases comic mischief and a specialist tool. **M/L. TODO:** verify the effect and actor attribution in the current implementation; repeated drinks by one soldier count once. |
| M12 — **Something in the Air** | One player-thrown wasp nest successfully affects three distinct hostiles. | A memorable area-effect feat with a small natural threshold. **M/L.** Count actual effects from one nest, not nearby enemies. Do not promise it is nonlethal until tested. |
| M13 — **A Different Kind of Scarlet** | Deploy Will Scarlet; have him knock out three distinct hostiles with his sling; finish with Clean Hands. | Makes the aggressive hero useful in a merciful plan. **M.** Require actual knockouts, not hits; allowlist missions with enough accessible sling ammunition. |
| M14 — **The People Behind the Legend** | Win an eligible optional mission with a deployed squad made entirely of generic Merry Men. | Celebrates the supporting cast and changes squad-building. **M/L. TODO:** verify legal selection and solvable objectives; never override a mission's mandatory hero requirements. |

M1 and M13 require their combined conditions in the same attempt. Owning Ghost or Clean Hands from another playthrough is insufficient. M11 and M12 intentionally do not also require Ghost: a funny distraction should not become a contradictory stealth test.

**Authored mission and mission-family candidates — 10**

These give a particular expedition its own story. Locations, prisoners, groups, and optional objectives must be bound to canonical mission content, not inferred from a translated mission title or a reused background. The interview evidence establishes the relevant story beats and mission families, not their exact current script IDs. [HomeLAN interview](interviews/06-haessig-devouassoux-homelan.md), [XGR interview](interviews/08-martin-kuppe-xgr.md).

| ID / working name | Mission context and proposed condition | What makes this replay interesting / audit required |
| --- | --- | --- |
| S1 — **Not Just Stutely** | Opening Leicester rescue: free Stutely and every authored fellow prisoner; all reach their designated safe outcome alive. | Makes the other prisoners matter. **L. TODO:** verify whether rescuing all already gates ordinary success. If so, strengthen it with zero post-release prisoner damage or use only a story milestone. The opening rescue is also described in the [GameSpot preview](https://www.gamespot.com/articles/robin-hood-the-legend-of-sherwood-preview/1100-2895665/). |
| S2 — **A Courtesy Call** | First contact with a friendly royalist baron: reach the meeting and win without damaging, knocking out, or restraining any of the baron's soldiers. | An embassy rather than a raid; stricter than a mission that already forbids killing. **L.** Define the baron's faction/group and audit mandatory encounters. |
| S3 — **The Guests May Stay** | Mission preventing Marian's marriage: accomplish the rescue while leaving every non-objective hostile untouched. | Selective intervention in a busy location. **L. TODO:** list mandatory opponents and effects explicitly; do not silently assume Guy can be spared. Exclude innocent casualties as well. |
| S4 — **The Sheriff Gets His Due** | Final confrontation: win Robin's authored duel with the Sheriff without Robin taking damage during the duel phase. | A fitting combat showcase which permits an otherwise messy mission. **L.** Scripted duel start/end events define the window. No achievement for merely attacking the Sheriff first. |
| S5 — **A Taxing Misunderstanding** | Tax-collector convoy ambush: obtain the required money and finish with Clean Hands while the designated collector survives. | Connects the robbery to mercy, with a specific person to protect. **L. TODO:** select variants where the collector can survive victory; make indirect trap/brawl responsibility explicit. |
| S6 — **The Forest Does the Work** | Convoy ambush: activate two distinct authored ambush mechanisms, each successfully incapacitating at least one distinct escort, with no direct player melee or ranged hit on an NPC. | A terrain-and-preparation solution. **L.** Arrows at trigger targets are allowed; scripted allied attacks are allowed and are not claimed to be nonlethal. Verify two mechanisms exist. |
| S7 — **No Need to Clear the Road** | Robbery ambush: take the objective and escape while at least half the original escort remains alive and has never been incapacitated by the player. | Encourages stealing the objective rather than clearing the map. **L.** Freeze the authored escort set and round the half upward; allow only variants whose victory condition permits survivors. |
| S8 — **Open for Business** | Castle assault with a gate/drawbridge objective: personally complete that objective before issuing any optional call for allied reinforcements, then win. | Focuses the siege on opening access. **L.** Initial scripted allies are permitted; compare specific objective and reinforcement events. |
| S9 — **A Castle, Not a Graveyard** | Castle assault: complete at least two distinct optional strategic objectives and win. | Rewards dismantling the defense rather than a kill quota. **L. TODO:** tag optional objectives such as access or defensive-post tasks; require meaningful authored tasks, not two increments of one counter. |
| S10 — **Marian Knows Best** | A castle infiltration with Marian: identify three distinct previously unidentified enemies using her listening/spy ability, then complete the main objective with Ghost. | Makes intelligence part of a stealth plan. **M/L.** Record identification caused by her ability; already-visible guards and repeated uses do not count. |

For a first mission treatment, **Leicester's rescue could offer S1, M4, and M1**, subject to verifying its available interactions and non-incapacitation route. A convoy could offer S5, S6, and S7; these are alternative ways to play, not a demand to satisfy all three simultaneously. A suitable castle assault could offer S8, S9, and M6. This produces recognizable mission personalities with only three additions per selected mission.

The archery tournament is an appealing narrative candidate, but the interview's mention is not enough to specify an interactive accuracy challenge. **TODO:** inspect the tournament script before proposing a “perfect score” badge. Likewise, do not invent stealing from an inventory, disguises that work like HITMAN, or environmental interactions merely because another Robin Hood adaptation contains them.

**Recommended first release — 12 candidates**

| Group | Candidates | Reason to start here |
| --- | --- | --- |
| Campaign, 3 | C1 A Legend Is Born; C3 The Whole Merry Company; C8 Protector of the People | Completion, the cast, and the outlaw's ethics provide three distinct long-term goals. |
| Reusable mission badges, 6 | M1 In and Out; M4 A Penny Well Spent; M6 On My Mark; M7 A Leg Up; M9 You Never Saw Us Leave; M13 A Different Kind of Scarlet | Mixes approachable discovery with stealth, coordination, recovery, and character mastery. |
| Authored challenges, 3 | S1 Not Just Stutely; S5 A Taxing Misunderstanding; S8 Open for Business | One rescue, one ambush, and one siege anchor the system in actual missions. |

This is a design priority, not a claim that all twelve are equally cheap. M9 and C8 require especially careful attribution/state evidence. If they delay the first release, substitute M2 on a suitable rescue and defer the campaign ethics badge rather than implementing unreliable approximations. If S1 simply duplicates mandatory completion, use its stricter post-release damage version after a solvability check.

Next, add Pass the Parcel, the Tuck/wasp feats, Marian's intelligence challenge, and the preparation achievements. They offer strong identity but need more event linking or authored content review. Keep full-campaign no-loss and hard-mode goals for a later pass, after verifying every required mission can support them.

**Evidence and implementation implications**

The current [MissionStat](../crates/robin_engine/src/mission_stat.rs) includes money, recruitment, casualty, and faction statistics. [Campaign state](../crates/robin_engine/src/campaign.rs) includes ransom and blazon preparation values. Those are useful starting points, but totals alone cannot prove who drank Tuck's beer, whether a particular prisoner survived, which action identified a guard, or when a gate opened.

New trackers should consume authoritative simulation outcomes: completed information purchases; health-loss events; ability effects with source and target identities; recruitment and permanent losses; assisted traversal; quick-action execution groups; and authored mission milestones. Store deterministic evidence with the successful attempt. Do not award from UI selection, animation appearance, or an achievement screen's current counters.

Campaign achievements require more than adding enum variants. The current aggregation policies are only all-required-missions and any-mission-once. C4/C8 need retained-progression history, C5/C10 need set coverage, and C2 needs a campaign milestone. The [HUD metadata](../crates/robin_rs/src/achievement_hud.rs) also currently returns four presentations. New identifiers must remain append-only, with an explicit native schema/version policy; imported saves with insufficient evidence stay unverifiable.

Preserve the existing host eligibility policy: cheated, custom, headless, and replay-playback runs can calculate evidence without awarding normal badges. Eligible **history practice** is distinct from playing back a recorded replay. Do not silently relax this boundary to make a new badge easier to implement.

For attribution, pay particular attention to a purse-induced brawl, traps, allied soldiers, and a guard dying after incapacitation. The current Clean Hands option is relevant to several proposals. Display the rule used by an attempt; do not silently treat different causality settings as equivalent prestige runs. A strict civilian-protection badge needs its own documented causal rule and verified event coverage.

**Presentation and balance**

Offer opt-in tracking for a few pinned challenges, preserving the current uncluttered default. Explain failures concretely: “Marian took damage,” “One escort died,” or “Reinforcements called before the gate opened.” Reveal mechanical conditions before an attempt, with optional spoiler masking for story achievements. Debriefing should show progress and the first disqualifying event.

Award cosmetic recognition, illustrations, or Hall of Deeds entries. Do not attach combat advantages: that makes an optional challenge a power requirement and changes the campaign balance. Allow saving and loading. Do not require wall-clock waiting, repeated headquarters idling, or discovering the one undocumented input that counts.

Treat thresholds of three and ten as starting hypotheses. Before shipping, playtest on the lowest valid equipment/roster state as well as a developed campaign, and on each allowed difficulty and data variant. A mission badge should generally require an interesting change of plan, not a long cleanup after the objective is already secured.

**Ideas to defer or reject**

- **Kill hundreds of guards / hide hundreds of bodies:** little new learning, encourages repetition, and competes with this game's recruitment incentives.
- **All money on every map:** risks hunting inaccessible or script-dependent coins and confusing gross collections with money spent. Consider a few authored treasury objectives later.
- **Never use Little John for an entire campaign:** too broad a restriction on a signature character. A suitable optional mission can encourage a different squad more naturally.
- **Never save or load:** punishes experimentation and interruptions. It is unrelated to the tactical feats this report prioritizes.
- **Every mission below one universal time limit:** missions, ambiences, and starting rosters differ too much. If desired later, author individual par times using simulation time, with known start/end boundaries and difficulty-specific validation.
- **All possible campaign missions in one run:** branching, ageing, and repeatable content make this a poor assumed denominator. Use the actual completed path or a published finite collection.
- **Collect/use every amulet:** first audit the port's inventory, acquisition caps, and effects. A quantity counter alone is insufficient evidence of distinct discovery or meaningful use.
- **Campaign Ghost plus Clean Hands again under another name:** duplicates existing goals unless it explicitly requires both in the same attempts and the entire required path is proven feasible. Forced duels and story visibility need review first.

**Remaining validation work**

This research did not play through the campaign or audit every mission script. Before implementation, produce a canonical mission/variant eligibility table, bind the named story events, and record a successful proof run for each restrictive authored challenge. Resolve every TODO above against the supported data packs; disable unsupported variants explicitly. Then test the consequential edge cases: damage followed by healing, incapacitation followed by death, rejected quick actions, prisoner transfer, late-spawned escorts, practice replay, campaign reset, and incomplete imports.

The intended outcome is a collection of stories players can tell about their decisions: a rescue where nobody was abandoned, a robbery that left the escort wondering what happened, and a castle won by opening the right gate.
