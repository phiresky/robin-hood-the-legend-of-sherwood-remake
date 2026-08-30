# Feature 29 integration: advanced combat gestures

## Decision summary

- **Owner decision:** accepted.
- **Integrated baseline:** root main `678a39483`, after Feature 12.
- **Accepted source reference:** `84475eff9` from
  `codex/every-feature-combat-gestures`; the old integration ancestry was not
  replayed.
- **Recommendation:** merge the reconciled revision.
- **Schemas:** native save **67**, replay **25**, and multiplayer protocol
  **34**. These are required changes because resolved gesture state and two
  authoritative gameplay rules enter persisted and wire state.

## Player-visible behavior

The Original A-I sword-gesture vocabulary remains the first classifier. Only
an Original `Attempt` is offered to the new recognizer, which adds nine
single-stroke techniques: Rising Feint, Falling Feint, Lightning, Backslash,
Triad, Rampart, Vortex, Stag, and Serpent. A recognized technique expands into
two ordinary authored sword commands, preserving normal animation,
interruption, energy, protection, targeting, and seek behavior.

Gesture accuracy is resolved once into a 25/50/75/100 percent tier. When its
rule is enabled, that tier scales cutting and concussion without changing
geometry or protection RNG. Guide and coach overlays can independently show
the templates and the last resolved result. Mouse and one-finger touch share
the recognizer.

All four controls are under Gameplay and default on for composites and quality
damage, off for guide and coach. The standalone Gameplay screen is paginated
in bounded 12-row pages, so all 42 integrated settings remain reachable at the
minimum logical viewport.

## Determinism and authority

`more_combat_gestures` and `gesture_quality_damage` are authoritative
`SimConfig` mission rules. Only the host may change them; guide and coach are
local profile presentation. Input resolves platform-dependent pointer samples
into a typed technique, quality tier, command, and exact seek distance before
the command enters replay, rollback, or multiplayer.

Admission validates both direct strikes and the restricted sword command nested
inside an automatic quick action before recording or simulation mutation. It
rejects non-sword payloads, invalid fixed-point qualities, mismatched composite
first strikes, disabled composites, and reduced qualities when quality damage
is disabled. This check is required because derived native decoders can create
private newtype fields without calling a public constructor.

The exact resolved quality is retained in queued quick actions, macro replay,
active sequences, and active sweeps. Native state missing these fields or the
two authoritative rules is rejected; there is deliberately no compatibility
path for obsolete Rust saves or replays. Original C++ save import remains a
separate boundary.

## Integration notes

- Feature 12 diplomacy state and command handling are preserved alongside the
  new combat rules.
- Feature 39's cooperative Options UI is preserved; its Gameplay model grows
  to 42 settings and its standalone legacy screen receives matching bounded
  pagination.
- Feature 34's bounded canonical replay spool/export and Features 18, 07, and
  45 are unchanged.
- Timed-mission, ambience, diplomacy, and combat-rule edits are all covered by
  the central host-authority predicate after this integration.

## Verification

Focused automated coverage passes for:

- direct and queued command semantic admission and host authority;
- composite recognition, sequence expansion, quality damage/RNG invariants,
  active-sweep and active-sequence serialization;
- quick-action recording, exact seek distance, and native missing-field
  rejection;
- mouse/touch input resolution, Gameplay pagination, and all integrated
  setting mappings;
- save 67, replay 25, and protocol 34 assertions, including browser join-ticket
  fixtures; and
- the native `robin` build plus formatting and whitespace checks.

The remaining manual smoke boundary is visual feel: draw the Original and nine
new techniques at several speeds and scales, verify guide/coach readability,
and compare mouse with touch in single-player and a host/client session. No
`original-code/` tree was available in this isolated worktree, so this review
makes no new pixel- or threshold-exact C++ parity claim beyond preserving the
already-ported Original classifier ordering.
