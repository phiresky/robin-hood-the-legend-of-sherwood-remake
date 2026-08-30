# Feature 16: rebalanced items and deterministic previews

## Decision summary

- **Owner decision:** Accepted, including the evidence-backed expansion.
- **Implementation:** Complete on `codex/feature16-current-main`.
- **Merge recommendation:** Merge after the recorded current-main validation.
- **Compatibility policy:** Fresh profiles enable every rebalance and preview.
  Each switch is independent. Original-parity replay forces all rebalances and
  previews off. Existing profiles with no Feature 16 fields retain shipped
  behavior until their owner enables the new settings.

## Implemented gameplay changes

The following six authoritative rules are independently configurable and
default on for fresh profiles:

1. A direct apple hit can interrupt an active enemy swordfight before applying
   the normal apple reaction.
2. A wasp nest uses a 75-unit initial acquisition radius instead of 50. Chase,
   sting, forget, apple-scent, VIP, and swordfight rules are unchanged.
3. A real stone projectile may target valid ground and emits one deterministic
   240-unit noise stimulus at its terminal impact.
4. Will Scarlet's stone uses base throw range 300 instead of the shipped 200,
   matching the other comparable throwables before the shared `1.153` factor.
5. VIPs, riders, and Stuteley no longer crumple a net or prevent unrelated
   captures in the same strict 40-unit area. They remain immune; allies remain
   catchable; terrain crumpling is unchanged.
6. An active outdoor, non-VIP soldier with authored beer potency zero can take
   ale and receives minimum potency 20. Indoors, VIPs, and positive authored
   beer values retain their original behavior.

Purses are deliberately not nerfed: shipped value, coin count, attraction,
and recovery behavior remain unchanged.

## Presentation controls

Eight local targeting aids can be switched independently:

- apple effect and combat-interrupt eligibility;
- direct-stone effect;
- ground-stone impact area;
- net capture area;
- net crumple prediction;
- ale eligibility and potency;
- purse value and interest conditions; and
- wasp acquisition area and target eligibility.

The optional stone-impact cue is a ninth, separate presentation control.
Disabling any preview or the cue cannot change an authoritative item outcome.
The Gameplay menu exposes all six rules, all eight previews, and the cue as
individual settings with explanatory tooltips.

## Determinism, replay, and multiplayer

`ItemGameplayConfig` lives in `SimConfig`, is serialized, state-hashed,
snapshotted, restored by rollback, included in multiplayer mission identity,
and changed only through authoritative frame commands. Ground-stone commands
carry their exact 3D target, layer, and `NoiseDistractionTarget` discriminator
through direct input, planned actions, replay, and multiplayer. The projectile
owns the serialized distraction bit until impact.

Original-parity construction selects `ItemGameplayConfig::classic()`, disables
the cue and local previews, and rejects extension-only ground-stone and settings
commands. Ranked runs seal their frame-zero item configuration and reject later
changes. Current Rust save/replay schemas require the new deterministic state;
older current-format Rust saves/replays are not silently guessed.

## Original-game and research basis

Shipped behavior was checked in `original-code`:

- `RHProjectileSettings.h` supplies the common `1.153` range factor and the
  stone's exceptional 200 base range versus 300 for comparable throwables.
- `RHElementStone.cpp` confirms the direct-hit path and live damage/concussion
  behavior.
- `RHElementNet.cpp` confirms the strict radius-40 sweep, ally capture,
  resistant-actor crumpling, and separate terrain checks.
- `RHartificialmalignity.cpp` confirms the outdoor beer-interest test, while
  `RHelementactorsoldier.cpp` confirms the authored potency increment.
- `RHElementWasp.cpp` confirms the 50-unit initial acquisition radius and its
  separation from chase, charge, sting, and forget distances.
- `RHElementPurse.cpp` confirms five recoverable coins, retained unchanged.

The complete bibliography, counter-evidence, value derivation, and scope
guardrails are in [`../ITEM_REBALANCE_RESEARCH.md`](../ITEM_REBALANCE_RESEARCH.md).
It includes contemporary professional reviews, the Strategy First manual, a
detailed walkthrough, player retrospectives, and positive counter-evidence.
The evidence supports making underused tactical consumables dependable while
keeping every change optional and preserving the already-useful purse.

## Implementation map

- `crates/robin_engine/src/gameplay_config.rs`: constants, six gameplay
  switches, eight preview switches, defaults, classic mode, and migrations.
- `ai_enemy/event_handlers.rs` and `engine/wasp_nest.rs`: apple interruption
  and wasp acquisition.
- `abilities.rs`, `engine/sequence_validity.rs`,
  `engine/sequence_runtime/mod.rs`, and `engine/combat.rs`: real ground-stone
  projectile validation, launch, persistence, and one-shot impact stimulus.
- `engine/input.rs`: classic/rebalanced stone range authority.
- `engine/nets.rs`: selective immunity without changing terrain or ally rules.
- `ai_enemy` and `engine/tick/deferred_outcomes.rs`: outdoor ale eligibility
  and minimum-potency completion.
- `player_command.rs`, `replay.rs`, `engine/rollback_safe.rs`, and
  `multiplayer.rs`: command, schema, replay, ranked, rollback, and network
  authority.
- `robin_rs/src/host_mouse.rs`, `ui_panel.rs`, and
  `ingame_menu/gameplay.rs`: previews, explanations, and independent controls.

## Verification coverage

Focused tests cover:

- fresh-on/classic-off defaults and disabling each setting without changing a
  sibling;
- apple interruption versus exact classic swordfight behavior;
- 50/75 wasp acquisition without post-acquisition changes;
- direct and planned ground-stone command round trips, layer/field retention,
  ammo/range/terrain checks, projectile serialization, one-shot impact, and the
  exact 240-unit stimulus;
- classic/rebalanced stone range;
- resistant net actors before and after ordinary victims, unchanged ally
  capture, unchanged terrain crumpling, and exact classic behavior when off;
- zero-beer outdoor eligibility, indoor and VIP exclusion, minimum potency 20,
  and preservation of positive authored potency;
- independent preview text/radii and presentation-only behavior;
- replay, rollback, multiplayer identity, ranked sealing, save schemas, and
  exhaustive Original-parity command rejection.

Current-main test and build results are reported with the final branch handoff.

## Known follow-up

- Record a manual screenshot/play matrix for the eight previews at every
  supported logical resolution and scale mode before release.
- Collect item-use telemetry if available. Review and walkthrough evidence is
  not controlled frequency data, so future value tuning should remain an
  evidence-based settings change rather than an unconfigurable rewrite.

