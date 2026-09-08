# Engine diagnostic configuration

## Implemented boundary

`engine/diagnostics.rs` owns one process-local `OnceLock<DiagnosticConfig>` and a
pure lookup-injected parser. Tick and movement observers borrow its filters;
there are no engine references, mutation capabilities, RNG calls, or simulation
fields in the configuration. Serde derives describe data only: the configuration
is never attached to an engine, save, replay, or state-hash schema.

Centralized gates: Drop Execute boundary, motion latch, attentive-owner handoff,
path-owner lifecycle, path barrier, post-seek handoff, and movement goal-owner
handoff. Existing diagnostic emission sites and output text remain unchanged.

## Environment contract and deliberate changes

- Any present gate value enables it, including empty strings and `0`.
- Disabled gates ignore their unused filter variables.
- Drop accepts only `pc:INDEX`, uses inclusive ranges, defaults `UNTIL` to `FROM`,
  and gives exact `FRAME` precedence over range values (which are then unused).
- Goal-owner accepts `pc`, `soldier`, and `civilian`; frame and owner are required.
- Motion-latch and attentive-owner require frame and creation order.
- Path-owner frame and creation order remain independently optional.
- Invalid enabled filters fail loudly with a named-variable panic during first
  diagnostic access, rather than silently disabling an observer. This retains
  the previous fail-fast policy for malformed numeric filters. Validation is
  now eager across enabled gates, so it no longer depends on observing the
  selected owner/frame first. Non-Unicode supplied filters also fail explicitly;
  the old optional lookups could silently treat them as absent.
- All these gates are sampled once per process at first access. Some were already
  cached; repeated environment reads are now eliminated for the others. Configure
  the environment before starting the process. Runtime environment changes are
  intentionally unsupported. “Observational” means no simulation-state mutation,
  not suppression of invalid diagnostic configuration errors.

## Validation and follow-up

Eight pure parser/filter tests cover disabled gates, presence semantics, inclusive
ranges, exact-frame precedence, missing/malformed/overflow/reversed filters,
supported owner kinds, exact/optional creation filters, and Unix non-Unicode
values. They never read or write the process environment.

Formatting and whitespace checks passed. Coordinated engine suite passed at
`9f0b52c2a`: 4472 tests, including all eight parser/filter regressions, with four
ignored. Fresh native ordinary/save-load headless and graphical EOF replays and
both multiplayer rollback/hash scenarios passed against the retained `f5f531c7`
binary. See [final acceptance](AUDIT2_PLAN.md#final-acceptance); no original
licensed parity corpus execution is claimed.

TODO: diagnostic gates in other subsystems (sprite row tracing, command-specific
capture and parity tools) remain outside this bounded engine tick/movement pass.
Do not move capture state into this immutable configuration merely to share a file.
