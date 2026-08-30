# Item rebalance research

Research date: 2026-08-29. This note records the evidence used for Feature 16,
including the accepted apple, wasp, ground-stone, and preview package plus the
later approved stone-range, net, and ale rules. It deliberately excludes
difficulty, diplomacy, achievements, and Sherwood trading.

## Source baseline

The shipped behavior was checked against `original-code`, not inferred from the
Rust port:

- `RHProjectileSettings.h` defines the shared `1.153` projectile range factor,
  base ranges of 300 for apple/purse/net/wasp and only 200 for stone, and the
  projectile masses/apex factors.
- `RHElementStone.cpp` applies the direct-hit path and falls back to the apple AI
  event for protected targets. The live direct-hit implementation uses damage
  10 and concussion 100; the unrelated `STONE_DAMAGE 30` in `RHsettings.h` is
  not the executed item path.
- `RHElementNet.cpp` uses a strict squared-radius test `< 1600` (radius 40),
  catches active humans including allies, and lets a VIP, rider, or Stuteley
  crumple/stop the net. Its terrain slope and eight-point reach checks remain a
  separate crumple source.
- `RHartificialmalignity.cpp::AnswerQuestion` accepts ale outdoors when the
  soldier profile's beer value is positive. `RHelementactorsoldier.cpp` adds
  that authored value to blood alcohol on completion, clamped to 100.
- `RHElementWasp.cpp` uses 50 for initial victim acquisition, with distinct
  chase/charge/sting/forget distances and apple-scent preference. The accepted
  change touches acquisition only.
- `RHElementPurse.cpp` emits five recoverable coins. That strong, flexible item
  is retained as a benchmark rather than nerfed.

## Internet bibliography and evidence

The evidence is useful at different strengths. Contemporary professional
reviews and the detailed walkthrough establish recurring play patterns. Forum
and user reviews identify friction, but individual complaints are anecdotes
and are not treated as measurements.

### Repeated or comparatively strong evidence

- The contemporary [GameSpot review](https://www.gamespot.com/reviews/robin-hood-the-legend-of-sherwood-review/1900-2897317/)
  describes intricate maps built around distractions, calls out apples as a
  distraction, and also reports challenge and frustration. This supports
  making tactical consumables dependable without flattening encounters.
- Steven Carter's detailed [GameFAQs walkthrough](https://gamefaqs.gamespot.com/pc/562021-robin-hood-the-legend-of-sherwood/faqs/23124)
  explicitly characterizes Will Scarlet's slingshot as short-ranged, difficult
  to target, and less useful than it could be. The guide repeatedly recommends
  purses, uses wasp nests in selected encounters, and only occasionally uses
  apples. Its failure to recommend thrown nets or ale is suggestive rather than
  conclusive, but the explicit slingshot criticism is direct balance evidence.
- The official [Strategy First manual transcript](https://studylib.net/doc/8645865/robin-hood---strategy-first)
  presents purse, catapult stones, wasps, beer, nets, and apples together as
  core produced consumables. Bringing unreliable members of that set closer to
  the tactical relevance promised by the manual is preferable to weakening the
  purse.
- The contemporary [WorthPlaying preview](https://www.worthplaying.com/article/2002/11/1/previews/6630-pc-preview-robin-hood-legend-of-sherwood/)
  praises multiple solutions and specifically illustrates purse tactics. The
  [Game Over review](https://www.game-over.com/reviews/pc/Robin_Hood:_The_Legend_of_Sherwood.html)
  likewise emphasizes character-specific abilities and nonlethal play while
  noting campaign shortcomings.
- The current [Steam store/review aggregate](https://store.steampowered.com/app/46560/Robin_Hood__The_Legend_of_Sherwood/)
  and [Metacritic aggregate](https://www.metacritic.com/game/robin-hood-the-legend-of-sherwood/)
  show durable affection for the game alongside recurring challenge/control
  criticism. Aggregates are context, not item-level evidence.

### Player reports and counter-evidence

- A long-hours [Steam community review set](https://steamcommunity.com/app/46560/reviews/?browsefilter=toprated)
  praises mechanical depth but describes a try/fail/quickload loop and modern
  technical friction.
- A recent [patientgamers retrospective](https://www.reddit.com/r/patientgamers/comments/1ua0o78/robin_hood_the_legend_of_sherwood_the_good_the/)
  argues that most abilities become irrelevant beside knockout and hogtie and
  calls the controls touchy. Another [retrospective](https://www.reddit.com/r/patientgamers/comments/1qhvbvu/robin_hood_legend_of_sherwood/)
  reports rough AI loops, opaque combat, and later repetition. These support
  investigating reliability, but each remains one player's experience.
- An older [Larian forum discussion](https://forums.larian.com/ubbthreads.php?Number=171690&ubb=showflat)
  positively describes emergent purse-and-apple tactics, countering any claim
  that every distraction is broadly disliked.
- An [Ars Technica demo discussion](https://arstechnica.com/civis/threads/robin-hood-legend-of-sherwood-playable-demo.775713/)
  reports small icons and moving-target selection friction while recognizing
  the available bait tactics. This is anecdotal and demo-specific.
- Positive [GameFAQs player reviews](https://gamefaqs.gamespot.com/pc/562021-robin-hood-the-legend-of-sherwood/reviews/112463)
  and another [highly positive review](https://gamefaqs.gamespot.com/pc/562021-robin-hood-the-legend-of-sherwood/reviews/64735)
  are retained as counter-evidence: the redesign should be optional and must
  preserve shipped behavior, not assume universal dissatisfaction.

## Evidence-to-change matrix

| Rule | Evidence and derivation | Accepted change | Confidence / guardrail |
| --- | --- | --- | --- |
| Apple combat interrupt | GameSpot and the walkthrough validate apples as bait; active swordfights currently make a correct hit feel ineffective. | A direct hit may interrupt a swordfight; original daze and scent durations remain. | High. Independent switch; Classic/Original off. |
| Wasp acquisition | Walkthrough uses nests selectively; original has an unusually small 50-unit initial acquisition radius while later wasp distances are separate. | Initial valid-target acquisition 50 -> 75 only. | High. No chase, sting, forget, scent, VIP, or fighting change. |
| Ground stone | Review/guide evidence favors useful distraction choices; stone lacked a ground utility despite being a produced consumable. | Real stone projectile may target valid ground and emit one deterministic 240-unit noise stimulus. | Accepted prior scope. Ammo, range, layer, replay, and terrain validity remain authoritative. |
| Stone range | The walkthrough directly calls the slingshot short-ranged and hard to target. Original uses base 200 while every comparable throwable uses 300. | Independently use base 300 instead of 200: effective range `300 * 1.153 = 345.9` rather than `230.6`. | High. Reuses an original sibling value; exact 200 behavior off. |
| Selective net immunity | Nets are a manual-listed core item but effectively lose their whole area to one protected actor; modern player criticism says narrow optimal tools eclipse many abilities. | VIPs, riders, and Stuteley are skipped rather than crumpling the net; other active humans inside strict radius 40 are still caught. | Medium-high. Allies remain catchable; terrain crumple and radius are unchanged; exact resistant-target crumple off. |
| Reliable ale | Ale is manual-listed but absent from the detailed walkthrough's recommendations; original beer=0 makes a scarce produced item a total refusal. | Outdoor non-VIP soldiers with authored beer 0 accept and receive minimum potency 20; authored values above 0 remain unchanged. | Medium, owner-approved. Indoors and VIPs receive no new eligibility; Classic uses the exact authored value. |
| Purse | Walkthrough, preview, and forum evidence consistently show that purse tactics already work and enable creative solutions. | No gameplay nerf. | Strong negative decision: do not erase the useful benchmark. |
| Eight previews + cue | Contemporary and retrospective reports repeatedly mention selection/control friction. | Independent apple, direct-stone, stone-area, net-area, net-crumple, ale, purse, and wasp previews; separate stone-impact cue. | Presentation-only switches; never author outcomes. |

## Integration boundaries

- Feature 15 (planned quick actions): the ground-stone target field, point, and
  layer already round-trip in the restricted queued-command payload. The range
  rule is read by the same authoritative projectile check at execution; no
  separate queue-only balance exists.
- Feature 12 (data-driven allegiances): net eligibility intentionally remains
  allegiance-neutral, matching the shipped ability to catch allies. Selective
  immunity only changes VIP/rider/Stuteley classification and does not consult
  diplomacy, coalition, difficulty, achievements, or casualty ownership.
- Sherwood production/trading is not retuned. No costs, output rates, stocks,
  mission rewards, or purse economy values change.

## Remaining uncertainty and follow-up

- TODO: collect post-release item-use telemetry if available. Walkthrough
  omission is not proof of non-use, and no cited source provides controlled
  frequency data.
- TODO: perform a manual visual matrix at every supported UI scale/logical
  resolution. Automated checks can prove radius/text selection but not
  legibility over every map palette.
- TODO: revisit ale potency only with play evidence. Twenty is deliberately a
  conservative minimum below the 100 cap, not a claim that historical players
  converged on an optimal number.
