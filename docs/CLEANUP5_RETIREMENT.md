# Explicit session retirement boundary

## Change

`game_session::retirement::run` encloses the complete body of each callback-owning
entrypoint: campaign `run_session`, direct graphical `run_mission`, and direct
`run_mission_headless`. Returning early from preflight, loading, or mission control
now returns to this runner; the runner always invokes callback save retirement
before returning the outcome. There is one implementation of result combination.

The callback boundary remains `RustCallbacks::finish_save_operations`. It drains
the save manager's accepted special-save operations and then the autosave
coordinator, collecting errors without skipping the second owner when the first
fails. This refactor does not change filesystem ownership or that ordering.

## Preserved contracts

- Successful retirement leaves the original outcome untouched, including errors.
- Failed retirement retains the prior outcome as diagnostic context using the
  existing mission/session wording; campaign and simulation metadata survive.
- Restart loops are inside the completion boundary. They do not retire an owner
  between attempts or discard restart checkpoints.
- A cancelled/failed save-store admission has no constructed callback owner and
  remains outside the session runner.
- Projection export remains separate and does not acquire this save boundary.
- This is explicitly awaited completion, not asynchronous `Drop`. Future
  cancellation and panics retain existing destructor fallbacks; they do not
  pretend that controlled retirement succeeded.

## Verification

Added focused tests for early-return coverage, unchanged success/original error,
combined errors, preserved mission metadata, and retirement after all restart
iterations. The callback executor lane is coordinating the real two-owner
retirement regression in its owned callback tests.

Formatting and whitespace checks are run locally. Cargo compilation and affected
client tests are intentionally consolidated by the integration lane; no local
test-pass claim is made before that run.
