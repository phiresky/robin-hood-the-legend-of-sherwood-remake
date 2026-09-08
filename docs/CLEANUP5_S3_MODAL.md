# S3: modal publication and dismissal authority

Implemented from second-audit base `4878a9162`.

`ModalNet::publish` now returns `Result<ModalPublication, String>`: a queued host
decision, a queued client proposal, or an explicit failure. The modal instance is
completed only after a host decision is queued, or when its matching remote
decision is received. A failed send leaves the instance open.

All production publication callers use `ModalDismissalGate`: dialogue (both
drivers), popup scroll, generic modal batches, mission-state popups, and terminal
mission/final/HTTP/load-picker decisions. The gate retains the exact local result
on failure and retries on later UI ticks without requiring another click. Failed
publication is not an authority-wait state. Errors are logged once per changed
diagnostic rather than once per frame. Successful client proposals wait without
resending. Completion is one-shot. The pending result is bound to its original
session/modal occurrence and cannot be published through a different modal or
silently acquire local authority when its transport disappears.

Terminal HTTP admission rejects a second decision while the first is queued,
retrying, or awaiting authority. Applying an already-authoritative terminal or
batch replay result retires any retry state. Existing replay admission/recording
ordering and unmatched remote-event preservation remain unchanged.

No timeout or fabricated dismissal is introduced. A permanently disconnected
transport remains an explicit session failure: the owning outer lifecycle must
retire the screen/session. The legacy blocking dialogue wrapper retains its
existing waiting loop, now with retained-result publication retries rather than
an unsubmitted-client wait or failed-host local completion.

Regression coverage added:

- Real disconnected outbound channels reject publication for both roles without
  changing the modal occurrence.
- The production gate retains failure outcomes, retries through a replacement
  channel with the same identity, and never resends accepted proposals/decisions.
- Client proposals wait for the host's exact result and deliver completion once;
  host authority can also resolve a previously failed client proposal.
- Local completion is one-shot; mismatched modal identities cannot consume a
  retained outcome.
- Actual terminal state HTTP admission preserves queued and failed decisions for
  both mission-state and final-debriefing boundaries without advancing their
  decision order; retry retains the chosen final load slot.

Validation: `cargo fmt --all` and `git diff --check` passed. Cargo compilation and
tests are deliberately consolidated in the root integration lane; no separate
build or test pass is claimed here.
