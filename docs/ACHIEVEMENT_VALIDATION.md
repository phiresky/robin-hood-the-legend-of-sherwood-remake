# Achievement validation — 9 September 2026

This pass checks the accepted catalogue against the original full Linux profiles,
mission scripts, and a live `H01_Lin_VL` session. It does **not** establish a
successful full-mission route for every achievement. Script inspection, controlled
regression tests, and ordinary gameplay observations are distinguished below.

## Findings and fixes

| Finding | Evidence | Change |
|---|---|---|
| For King Richard could never unlock with the original profiles. | The implementation compared `mission_name` (a display title) with `H10_Yor_VL` (a filename). The real profile stores them separately. H10's dispatch message subtracts £100,000 and sets custom campaign slot 6. | Match `mission_filename` and require that dispatch flag, `CampaignValue::Custom7`. A large wallet alone is insufficient. |
| An exited hero could manufacture an escape while another hero remained on the map. | The old check required any active party member and did not check the engine's `in_honolulu` flag. | Require the party to remain on-map and clear an unfinished pursuit when it does not. Removed/off-map pursuers cannot manufacture a successful escape either. |
| The campaign screens offered banner and generic-squad challenges on unrelated missions. | Presentation enumerated the entire mission-badge catalogue without mission requirements. | Share static eligibility between simulation and both campaign presentations. |
| Ruthless conflicts with Ranulph's mission's victory predicate. | `H04_Lei_VL` checks dead soldiers, excluding its five marked Sheriff soldiers; a protected guard's death causes defeat. See [script research](ACHIEVEMENT_MISSION_RESEARCH.md). | Do not offer Ruthless on this mission. This does not establish the eligibility of every other map. |

The ransom flag uses a **zero-based script ordinal**: script slot 6 is Rust's
`Custom7`, not `Custom6`. A regression test covers the real filename with a
different display title, the missing flag, and the same flag on another mission.

## Live gameplay observations

The full Linux game was launched headlessly with the original opening mission,
an isolated save directory, a paused simulation, and local RPC player commands.
No teleportation, invulnerability, injected money, forced death, or forced victory
was used. Existing local mod overlays were registered by the application's normal
startup; this run is therefore not a clean distribution certification. Headless
results cannot award persistent achievements.

The replay is `2026-09-09T16-53-45+02-00.rhrec.jsonl` in the local replay directory.
State captures are retained locally under `target/achievement-validation/`.

| Observation | Actual result |
|---|---|
| Initial population | 38 hostile soldiers, seven civilians, one rich civilian, one beggar. No initial corpses. |
| Running Pay order | Does not perform a payment. The existing double-click behaviour discards this Pay interaction. The treasury and hint cursor remained unchanged. |
| Running movement toward the beggar | Guards interrupted the first approach. A subsequent ordinary run order reached him. |
| First completed Pay, frame 880 | Treasury £100 → £50; beggar hint-set cursor 0 → 1. Exactly one information payment occurred. `beggars_exhausted` and `charitable_payments` both remained zero. |
| Pursuit | Seven distinct guards were tracked. The tracker did not award escape while they still pursued Robin. |
| Damage | Robin's health fell during the approach. The damage-free condition failed; no deaths were needed to fail it. |
| Rich civilian | Starts inactive and is activated by the Worman encounter, together with the money bonus and house access. Blindly ignoring every initially inactive civilian would omit a real mission participant. |

The run reached frame 1061. It did **not** win the mission, exhaust the beggar,
make a charitable donation, escape the chase, or kill a civilian. These are
partial gameplay observations, not successful badge proof runs.

## Banner preparation and static eligibility

The original full Linux profiles distinguish preparation banners from banners
collected inside a siege:

| Mission | Total needed | Collected inside | Purchasable preparation allotment |
|---|---:|---:|---:|
| `Str01_Lin_EC` | 12 | 5 | 7 |
| `Str02_Der_MP` | 12 | 6 | 6 |
| `Str03_Yor_MK` | 12 | 7 | 5 |

The opt-in `full_game_banner_purchases_use_the_real_preparation_limits` test
loads the actual CPF, buys banners through the campaign's purchase methods,
checks that the UI admission rule refuses an additional purchase at the limit,
and checks the resulting achievement evaluation. Its funded test campaign is
a controlled preparation test, not a played siege victory.

The tactical missions produce banners, but that does not make them eligible for
the two purchase badges. Pseudo defence profiles are strategic events rather
than playable field missions. Generic-only badges require an optional
Ambush/Tactical profile whose mandatory characters are all generic Merry Men;
the simulation still checks who actually participated.

## Reproduction

Build the game before running it. Start a paused original-data probe with a
separate save directory:

```sh
ROBINHOOD_DATA_DIR=/absolute/path/to/fullgame_linux \
ROBINHOOD_SAVE_DIR=/absolute/path/to/probe-saves \
RUST_LOG=robin_rs=debug target/debug/robin \
  --headless --start-paused --mission H01_Lin_VL --http-server 17649 --no-sound
```

Use `/engine-dump` to resolve current entity IDs. In this run Robin was `Pc(126)`
and the beggar was `Civilian(55)`. Normal commands use `/command`; advance with
`POST /step-forward`, for example `{"n":100,"auto_dismiss":true}`. Capture both
the treasury and the beggar cursor; an accepted command reply is not evidence
that the interaction completed. The RPC interface is documented in
`crates/robin_rs/src/http_server.rs`.

Run the opt-in profile test separately from ordinary package tests:

```sh
ROBINHOOD_DATA_DIR=/absolute/path/to/fullgame_linux \
cargo test -p robin_engine --lib \
  full_game_banner_purchases_use_the_real_preparation_limits -- --ignored
```

## Verification results

The engine library suite passed (4,526 tests), the final client library suite
passed (1,904 tests), all seven achievement-hook regressions passed, and the
opt-in original-profile purchase test passed. The normal ignored tests retain
their fixture/backend requirements. `cargo fmt --check` and the native `robin`
build passed. A temporary failure in concurrently edited campaign-history
scrolling tests was resolved by that separate change and passed on recheck.

These checks establish the fixes and the recorded observations, not the
end-to-end routes listed below.

## Still needs successful routes

- **Beggars:** exhaust every set, then make the separate unrewarded donation;
  finish the mission and inspect the frozen results. The opening treasury only
  funds two payments, so this needs a legitimate source of additional money.
- **Civilian death:** prove an indirect, player-reachable mechanism through
  ordinary inputs. A scripted or debug kill would only test the counter.
- **Scarlet:** use an ordinarily recruited Will, knock out six distinct enemies,
  and win with Clean Hands. His original profile permits 12 stones, but capacity
  alone does not prove a stocked squad or a viable route.
- **Escape:** break the seven-guard chase, or a smaller three-guard chase, with
  all pursuers alive and the party still on-map, then win.
- **Banners:** play each siege after the zero-purchase and full-purchase
  preparations. Preparation arithmetic alone does not prove victory.
- **Ruthless and rich civilians:** finish variant-specific reachability audits.
  The [scoring guide](third-party/guides/steam-scoring.md) reports additional
  inaccessible actors. Keep those reports distinct from verified predicates.

These remaining routes also need checks on supported difficulties and other
data distributions. The unapproved S candidates remain outside implementation.
