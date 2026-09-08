# S5: Application-owned cache-maintenance completion

The settings panel no longer owns the sole cache-clear receiver. Application
services now own a single-flight `CacheMaintenance` operation. Panels keep only
the last observed status; closing them does not discard pending work or its
result. Reopening observes pending state (Clear Cache disabled) or the retained,
localized success/failure notice. Observation is non-consuming, so another panel
cannot steal completion. An explicit subsequent request replaces the prior notice.

Admission and receiver installation are serialized by the application owner's
mutex. A duplicate request during pending work returns the same operation identity
without spawning another worker. Worker disconnection becomes an explicit retained
error, not a claim that dropping a receiver cancelled physical deletion. Native
thread creation failures are also retained outcomes.

Physical clear implementations are unchanged: native work uses the context's
`DistributedModCache::clear`, and browser work uses its existing async cache clear.
Their mounted-content and pin protections remain in force. Workers retain an
application clone until sending their completion. No new cancellation or shutdown
join guarantee is claimed for process termination or browser runtime destruction.

Runtime maintenance ownership is skipped during serialization. Defaults,
deserialized contexts and closed projection contexts cannot admit maintenance;
normal application initialization explicitly creates the owner. Context clones
share it, separately initialized applications do not.

Added tests cover close/reopen across success and failure, duplicate requests,
disconnected workers, spawn failures, retained notices, independent owners, and
application clone/serialization boundaries. Formatting and whitespace checks are
local; native/browser compilation and affected tests are consolidated by root.
No local Cargo-pass claim is made before those gates.
