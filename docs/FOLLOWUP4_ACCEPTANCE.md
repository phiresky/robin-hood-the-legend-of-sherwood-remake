# Ownership and recovery follow-up acceptance

Implemented all four follow-ups approved after audit3. Implementation, package
and consumer checks, native runtime acceptance and browser acceptance are complete.

## Changes

| Former design | Implemented boundary |
| --- | --- |
| Detached special-save writers with eager metadata | One bounded, owned background job; join before metadata/index publication; durable recovery receipt; explicit retirement |
| Public mutable save collection and interchangeable indices | Non-cloneable store authority, private collection, explicit Draft/Published/Session states and owner/generation-bound handles |
| Failed save admission without a usable recovery state | Typed admission error, Retry/Cancel/Quit, no empty writable fallback, visible picker/save failures |
| Backup cancellation/heartbeat failure could outlive its physical-work fence | Invocation owner retains locks, exact token and exclusive fences until physical jobs and SQLite workers finish |

Background serialization remains off-thread. Later conflicting mutations drain
the owned job; completion errors block further mutation until reopening the
store, while per-frame error delivery is deduplicated. Digest-bound receipts
allow reopening to reconcile payload success followed by index failure. Recovery
validates the untrusted index before processing quick-save, owned-save and
deletion receipts, in that order.

Queued saves/loads, cross-mission transitions and quick-load confirmations now
carry stable runtime handles. Deleted/recreated, foreign-manager and deserialized
handles cannot authorize operations. The multiplayer sentinel index is gone:
slotless decoded snapshots require explicit process-local committed provenance,
which serialization cannot recreate. Already-decoded snapshots retain their
exact payload instead of being reread after preflight.

Recovery input/widget state survives both frames and failed Retry attempts,
including repeated clicks without moving the pointer. Cancellation never grants
a manager or resets files. Session retirement explicitly drains both special-save
and autosave owners, including early-return paths. Browser Restart remains
session-owned memory, not a fabricated durable save.

Backup writes use tracked blocking operations with bounded-buffer copying.
Destination SQLite transaction cleanup closes its own connection; failed partial
cleanup also waits for the source pool to become idle. Caller cancellation does
not release operation.lock or the exclusive fences early. Existing exact-token
checks before authenticated installation/status publication remain intact.

Detailed contracts and limits:

- [Save owner and store API](FOLLOWUP4_SAVE_OWNER.md)
- [Queued slot handles and multiplayer provenance](FOLLOWUP4_SLOT_HANDLES.md)
- [Recovery and error UI](FOLLOWUP4_RECOVERY_UI.md)
- [Backup physical completion](FOLLOWUP4_BACKUP_OWNER.md)

## Source provenance

Started from `a6884ab7035d0a2bf09b1e6144b70b64708e5663` in six isolated,
same-named `followup4-*` worktrees. Independent read-only save/backup reviewers
found no remaining source blockers after the implementation revisions.

Final tested source: `89b42da98d757ac4eb9105ac75789eddde518fa9`.
The user's intervening `f8c871ab3` changes were incorporated before acceptance,
preserving official projection startup's independence from player save storage.
Later main commit `1f36b5ac6` added replay measurement scripts and documentation;
merge `6cd774dbb` preserves it. An exact diff confirms that merge changed none of
`crates/`, Cargo manifests/lockfile, `.cargo/`, the pinned toolchain or the
`scripts/validation/` and `scripts/check-quality.sh` acceptance harnesses relative
to the tested source. The retained binaries identify their actual build commit,
not the later documentation/measurement merge.

## Package, build and backend checks

Cargo used `--locked`, `RUSTC_WRAPPER=`, `CARGO_BUILD_JOBS=1` and normal
worktree-local targets; test execution used `RUST_TEST_THREADS=2`. Builds ran
separately from bounded game execution. No Clippy, all-features substitution,
target-directory redirection, dependency installation or shared-cache mutation.

| Check | Result |
| --- | --- |
| `cargo test -p robin_rs`, default features, final source | Passed: 1,655 library tests, plus integration/doc tests; eight explicit library ignores |
| `cargo test -p robin_rs --no-default-features --features release`, final source | Passed: 1,759 library tests, plus integration/doc tests; eight explicit library ignores |
| Separate `cargo build -p robin_rs --bin robin`, default features | Passed on final source |
| Separate `cargo build -p robin_rs --bin robin --no-default-features --features release` | Passed on final source; exact binary retained below |
| `cargo test -p robin_highscores` | Passed: 232 tests; seven explicit ignores; rerun on combined `8cf3a122f`, with service sources unchanged in final source |
| Save-owner LLVM panic regression | Passed on `de4a97098`; subsequent changes affect only recovery UI and its documentation |
| Two admin LLVM unwind regressions | Passed; operation/heartbeat/destination-transaction panic schedules covered |
| Explicit Vulkan multipass/readback test | Passed using the freshly built final default-client test executable and private XDG runtime directory |
| Optional tools/projection consumers | Passed: `cargo check -p robin_rs --features tools,projection-export --bins --examples` |
| Optional parity client | Passed: `cargo check -p robin_parity --features client`; compilation, not corpus execution |

The full release-feature package test command also compiled the audio benchmark
example with its active audio implementation; a redundant separate cold check
was not run. Optional-consumer checks used the same final source and `--locked`.

The explicit save unwind command was:

```sh
cargo test --locked -p robin_rs --lib --no-default-features --features release \
  --config 'profile.test.package.robin_rs.codegen-backend="llvm"' \
  savegame::tests::llvm_owned_worker_panic_is_joined_and_reported_once \
  -- --ignored --exact
```

The backup report records its corresponding LLVM command and a negative
experiment: restoring the old immediate-return heartbeat policy made the
physical-ownership test fail; restoring the committed implementation made it
pass. Cranelift's ordinary panic harness is not claimed as unwind proof.
Compiler warnings remain; this is not a zero-warning or Clippy cleanup pass.

## Final native runtime acceptance

Retained release-feature **development-profile** binary:
`/tmp/robin-followup4-runtime.4pv9m4/final/robin`, with adjacent assets/mods.
SHA256: `c35cfbdd534cb104b1c79f043a030a22f1d82da409fbd6f39db41cbc964f8318`.
Build and harness source: `89b42da98d757ac4eb9105ac75789eddde518fa9`.

All runs used private profiles/save roots and loopback-only network namespaces,
the existing Leicester demo, and bounded execution. Binary/source integrity was
checked before and after execution. Evidence paths below are relative to
`/tmp/robin-followup4-runtime.4pv9m4/final/`.

| Gate | Result | Evidence |
| --- | --- | --- |
| Native lifecycle | All four phases passed: ordinary and save/load replay, each headless and graphical; actual quicksave restored frame 42, continued to 142, post-load hashes verified through EOF | `native-lifecycle/summary.json` |
| Headless multiplayer | Ten checks, two rollbacks, hash agreement at 0/25/50/75, successful reconnect, zero desyncs/missed comparisons | `multiplayer-headless/summary.json` |
| Graphical multiplayer | Same ten checks and hash/rollback/reconnect results | `multiplayer-graphical/summary.json` |
| Recovery UI | Five scenarios passed: repeated keyboard/pointer Retry while corrupt, stationary Retry after external fixture repair, pointer Cancel, pointer Quit, Escape and window-close; repaired mission launched and closed normally | `recovery-ui-rerun1/summary.json` |

The recovery row groups the Retry sequence as one scenario. It verifies broken
bytes remain unchanged before deliberate external fixture repair; the product
does not perform that repair. The final readable diagnostic/buttons were
visually inspected. The temporary driver is retained at
`/tmp/robin-recovery-driver.KPoN1C/recovery_live.py`.

Native lifecycle used a 1,500-second outer bound and its per-phase limits;
multiplayer used 600-second outer bounds and the 540-second driver limit, with
`--observe-hashes-before-reconnect`, not process-restart mode. Recovery used a
320-second outer bound. All runtime children stopped.

## Final browser acceptance

The complete `browser-audio` gate passed on clean, frozen final source: audio and
audio/multiplayer bin/test checks, linked WASM module, and **29 Chrome tests**.
This includes browser store/autosave failure propagation and profile/key
persistence, alongside audio ownership, shared protocol and identity tests.

Evidence: `/tmp/robin-followup4-browser-corrected.88Kg6J/summary.json`,
`browser-tests.log` and retained `browser-tests.wasm`.
WASM SHA256: `6096e96b674325222dd3b3d609c3194203c314672589791ef7b8672b85832202`.
Chrome/driver: 152.0.7977.64; wasm-bindgen runner: 0.2.127.
No dedicated browser recovery-dialog execution test was added; the native UI
results are not presented as browser UI or full-game browser rendering coverage.

## Failures resolved during integration

- Migrated two non-test multiplayer diagnostic-save callers left on the removed
  draft API; added successful/rejected-allocation callback regressions.
- Updated old error-message expectations while strengthening unchanged-file and
  pre-recovery rejection assertions. Fixed an old load fixture to publish a real
  save instead of placing a file beside a deliberately unpublished draft.
- Fixed a menu-font coordinate conversion and public Mission imports in fixtures.
- Actual UI execution found lost focus on stationary repeated Retry. Persistent
  recovery-episode state fixes it, with both a regression test and final live proof.
- Corrected temporary driver issues separately: namespace inspection must use
  network-namespace-aware `ip`, initial input must await the actual dialog, and
  Escape's successful window destruction can race the helper's later key-up.
  Failed/diagnostic evidence remains retained; workaround movement was not used
  to accept stationary Retry.

## Deliberate limits

Local stores remain single-owner, not a multi-process locking or adversarial
symlink-security boundary. Conflicting mutations can wait for the bounded owned
job. Autosave refresh conservatively invalidates old autosave handles. Durable
receipt reconciliation is not guessed repair of an arbitrary corrupt index.
Retry/guidance translations remain explicit TODOs; existing Cancel/Quit labels
are localized. Browser manual special-save persistence remains unsupported;
session Restart and durable autosaves retain their separate contracts.

Physical draining does not promise recovery from process kill/abort, destroyed
async runtimes or permanently stuck kernel I/O. Existing process-death recovery
is still needed. No licensed-corpus sweep, optional GL success, Android/Windows
execution, performance improvement, deployment or remote push is claimed.
The `/tmp` artifacts survive worktree removal but are not permanent remote
archives. The user's unrelated worktrees and `original-code/` remain untouched.
