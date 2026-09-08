# S1: one owned publication transaction

Synchronous manual/diagnostic saves now own the complete publication sequence:
validated capture → durable recovery receipt → payload publication → catalog
promotion → durable index → receipt retirement. Background saves use the same
receipted payload primitive and the same catalog/index finalizer. Quick-save
rotation retains its existing two-payload receipt and ordering protocol.

`write_save_from_engine` and `write_multiplayer_diagnostic_from_engine` return
`CommittedSave` only after the complete transaction succeeds. Its stable handle
and payload digest are process-local evidence of that publication. Serialization
is diagnostic only; deserialization always rejects, including a genuine receipt
round trip. Executor callers no longer publish the index independently;
special-save wrappers retain their existing unit-result API.

Payloads, indexes, and receipt JSON shapes are unchanged. The existing owned
receipt now admits validated manual/diagnostic slots as well as special slots.
This widened domain is necessary for the shared transaction, not a new path
escape: basename checks, exact special-kind agreement, complete metadata, payload
digest binding, and autosave/control-name rejection remain mandatory. Autosaves
continue to use their separate manifest authority.

A failure before payload publication leaves old metadata intact. The owner
retires a definitely uncommitted prospective receipt durably so unrelated safe
operations can continue. An explicit no-clobber rejection never promotes an
orphan, even if its bytes happen to match the intended payload. A matching
published payload or uncertain recovery read retains receipt evidence and a
sticky failure; subsequent mutation cannot overwrite the recovery record.
Reopening reconciles the matching payload and metadata before further writes.
Index/retirement errors never return committed evidence.

Thumbnail failure remains nonfatal. Browser manual/diagnostic persistence remains
explicitly unsupported before capture or publication; memory Restart and durable
autosave behavior are unchanged.

Tests exercise the real manager transaction for new drafts and overwrites, both
ordinary and diagnostic, with injected failures before receipt, before/after
payload publication, and before/after index publication. They check reopen
identity, diagnostic tagging, unrelated-slot preservation, draft state, sticky
mutation blocking, and receipt retirement. Additional cases cover successful
committed evidence and identical-byte no-clobber collisions. A browser test
checks unsupported publication leaves Session/autosave metadata intact.

Formatting and diff checks run in this lane. Native/browser execution is
coordinated in the combined client lane; no duplicate Cargo build was started.
