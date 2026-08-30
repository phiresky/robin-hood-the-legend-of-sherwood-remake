# Feature 34 integration: bounded canonical replay export

## Status

- **Owner decision:** accepted.
- **Integrated baseline:** root main `9ca99a741` after Features 39 and 18.
- **Accepted source reference:** `a32af987fe`.
- **Novel delta:** bounded mission-generation replay spooling and asynchronous
  export of the existing canonical compact-bitcode replay.
- **Schemas:** unchanged at native save 65, replay 23, and network 32. This
  feature changes neither simulation state nor a persisted/wire layout.

The HTTP/RPC server, screenshots, stepping, replay import, and basic replay
export were already present on main. This integration adds only the accepted
recorder/export hardening and does not replay stale service ancestry.

## Integrated behavior

The old unbounded `Arc<Mutex<Vec<u8>>>` recorder mirror is replaced by a
mission-generation-scoped spool:

- 64 MiB total and 16 MiB per-record hard ceilings;
- 64 KiB immutable chunks and one bounded mutable tail;
- visibility only after complete recorder flush boundaries;
- fail-closed handling for overflow, stale mission writers, durable primary
  write failures, and durable primary flush failures;
- snapshots that clone immutable chunk handles and copy at most one incomplete
  64 KiB tail on the mission thread;
- a capacity-one native export worker with explicit busy/disconnected errors;
- a single-flight browser export task that yields before canonical encoding;
- output exclusively through `replay_format::encode_compact`, preserving the
  existing compact bitcode envelope and engine-hash binding.

`GetReplay` is routed directly to this exporter from pre-engine, interactive,
and headless drains. Feature 39's cooperative UI and typed modal/step handling
remain untouched, so encoding no longer runs synchronously inside generic
engine dispatch.

The tee writer preflights spool capacity before changing the durable primary.
It mirrors only the exact prefix accepted by a legitimate short write, and it
poisons the export spool after a primary write or flush failure instead of
returning a plausible truncated replay.

## Deliberate exclusions

This integration does not add:

- JSONL inside a compact-looking envelope;
- a public replay-format enum or format negotiation;
- raw or compact untrusted-admission budgets (Feature 36);
- leaderboard upload, rankability, or mission-end submission (Feature 43);
- changes to Feature 39 multiplayer pause, stepping, reconnect, or UI policy.

Native developer automation retains the already-existing explicit JSONL
`load-replay` input. Production export remains exactly one format: canonical
compact bitcode.

## Verification boundary

Focused coverage exercises complete-flush visibility, atomic overflow poison,
mission generations, fixed chunk bounds during a long run, canonical compact
round-trip, primary short writes and failures, and saturated native export
backpressure. HTTP/RPC routing tests protect the already-integrated service
surface.

Canonical bitcode encoding is still a whole-value operation. The browser task
yields before encoding, but it cannot yield within serialization/compression
without changing the bytes. If real-world browser runs require stronger
isolation, the follow-up must move this exact encoder to a worker or provide a
proven byte-identical incremental implementation; JSONL-in-compact is not an
acceptable substitute.
