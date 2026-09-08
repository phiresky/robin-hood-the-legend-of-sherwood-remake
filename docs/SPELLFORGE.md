# Spellforge mission contract

Spellforge missions use one versioned, deterministic Lua package on native,
Android, and `wasm32-unknown-unknown`. The interpreter is process-local and
disposable. Exact package bytes and the engine-owned event/native journal are
the authoritative state used by saves, rollback, replay, multiplayer snapshot
adoption, and reconnect.

Spellforge is a post-Original extension. Callback placement and native
suspension follow the Original script driver's synchronous boundaries; the Lua
language, package, and distribution policies are explicit Rust-port contracts.

## Author validation

Use the production package loader and VM before publishing a mission:

```console
cargo build -p robin_rs --example spellforge_check
target/debug/examples/spellforge_check mission.zip \
  --rhm-entry English/DATA/Levels/H06_Lin_VL.rhm \
  --shared-lib lib_2026_01_25.zip \
  --json
```

`--shared-lib` is optional when the selected mission directory contains its own
`lib/` tree. The checker verifies that the exact selected `.rhm` exists, maps
to `Data/Levels/<basename>.rhm`, and has a readable header, then performs the
same byte-oriented ZIP admission, module compilation, sandbox construction,
entrypoint execution, executable-ABI check, and package hashing as gameplay.
It does not invoke engine callbacks or mutate a campaign.

The report's `package_sha256` identifies the exact contract version, executable
VM ABI, script mode, canonical paths, and bytes. Editing any of those produces
a different package.

## Package layout and admission

The selected `.rhm` entry must have one exact same-directory, same-basename
`.lua` companion. Other direct sibling `.lua` helpers are retained as root
modules, except companions belonging to another sibling `.rhm`. Exact root
module names take precedence over optional leaf aliases from nested libraries.
Mission-local `lib/**/*.lua` files form one atomic library set. A shared
library ZIP is used only when that set is empty. Optional
`spellforge.contract.json` selects `replace`, `augment_before`, or
`augment_after`; its `contract_version` must match the engine.

Admission rejects case-folded duplicates, ambiguous companions or manifests,
absolute/traversing/backslash paths, symlinks, ZIP64/multi-disk archives,
trailing polyglot data, unsafe module aliases, and malformed manifests. Current
ceilings are:

| Resource | Limit |
| --- | ---: |
| Compressed bytes per ZIP | 64 MiB |
| Entries per ZIP | 2,048 |
| ZIP central directory | 4 MiB |
| Combined retained Lua source | 16 MiB |
| Package files | 2,048 |
| One canonical path | 1,024 bytes |
| Combined path metadata | 2 MiB |

The retained-source limit is also the interpreter allocation ceiling. Package
source, limits, bootstrap, sandbox, native registry/signatures/aliases,
deterministic standard-library behavior, pinned interpreter source, and the
journal contract all contribute to `vm_abi`. A semantic or ceiling change must
therefore repin the ABI and authentic-corpus package hashes and advance replay
and multiplayer schemas.

## Authoritative journal and limits

Every completed top-level callback appends one immutable event record. Native
requests retain their exact argument and return words. Callbacks synchronously
triggered while a native is suspended use the flat, non-recursive grammar
`Begin -> NativeRequest -> NativeReturn -> Complete` at that native's call
site.

In memory the journal is a persistent linked history. A rollback clone shares
one tail pointer and each new event adds one node. Native/Serde snapshot codecs
flatten it to chronological records, so the wire layout is non-recursive. A
rolling SHA-256 chain authenticates every event; deterministic frame hashes use
that digest and fixed-size counters instead of rescanning the full mission
history. Long unique histories are reclaimed iteratively rather than through a
recursive destructor.

Current runtime ceilings are:

| Resource | Limit |
| --- | ---: |
| Top-level events per mission | 131,072 |
| Native requests per mission | 1,048,576 |
| Nested transcript entries per mission | 1,048,576 |
| Canonical retained journal values | 16 MiB |
| Direct native requests in one event | 65,536 |
| Nested transcript entries in one event | 131,072 |
| Argument words in one invocation/request | 256 |
| Event or target-class string | 4 KiB |
| Combined package and journal snapshot values | 32 MiB |

The runtime enforces in-flight per-event and aggregate counts before a callback
can construct an unbounded host-side transcript. Event append is transactional.
Reaching a ceiling produces `SpellforgeGuestErrorKind::ResourceLimit`, which
uses the ordinary structured mission-abort/disconnect path.

Snapshot decode preserves every exact record even when the infallible native
codec observes a bad limit. Engine admission then rejects its retained
diagnostic before state hashing or level attachment. Snapshot encoding also
preflights the package digest, journal counters/digest, and combined budget.

## Determinism and compatibility verification

The checked-in tests cover:

- native and Node-executed WebAssembly ABI identity;
- identical golden bits/strings for parsing, formatting, character handling,
  table behavior, and deterministic math on native and WebAssembly;
- package and nested transcript native-codec round trips;
- package tampering, same-length journal divergence, and resource rejection;
- an 8,192-event long-mission journal clone/hash/codec regression; and
- SHA-pinned upstream libraries and ten cases from representative published
  missions under `crates/robin_spellforge/tests/corpus/manifest.json`.

The external corpus test is opt-in because the copyrighted mission ZIPs are not
committed:

```console
SPELLFORGE_CORPUS_DIR=/path/to/verified-corpus \
  cargo test -p robin_spellforge --test authentic_corpus -- --ignored
```

The manifest pins every download SHA-256 before production package admission.

## Release policies

The compatibility promise covers every ordinary mission currently inventoried
under the repository's `datadirs/mods` selections and the published safe API.
It is not a promise to execute arbitrary future LuaJIT programs. FFI, JIT
controls, host file I/O, dynamic native modules, and other ambient authority are
rejected with diagnostics.

Joining unknown exact content requires an explicit prompt. Approval is keyed by
the complete-mod hash and optional Lua-package hash, persisted per durable
player profile, and can be revoked individually or all at once under Gameplay
settings. `Allow Spellforge Missions` is the global fail-closed master switch.
Profile deletion, replacement, or regeneration revokes the associated grants
before a numeric profile id can be reused.

Portable replays embed the complete bounded canonical `SpellforgePackage`.
JSONL and compact readers validate its hash, executable ABI, canonical paths,
and resource limits before playback; no local mod installation is consulted.

Multiplayer hosts distribute one canonical envelope containing the selected
mission/map, Lua, art, audio, text, and selected shared library. The host must
show licence metadata or explicitly attest redistribution permission. Clients
consent before transfer, stage bounded resumable chunks, validate the exact
complete hash, mount only authenticated bytes, disconnect after no-seat
preflight, then reconnect under the same durable public-key identity and send
`ContentReady` only for the identical offer. Changed, introduced, omitted, or
downgraded content is rejected without a local-install or vanilla fallback.

Native hosts and clients use iroh directly. Browser clients use iroh's relay
transport and may join an authenticated, host-signed
`#join=<BrowserJoinTicket>` invitation. The stable shell captures and removes
the fragment before any network request, verifies the signed build/content
identity, and hands the exact ticket to Rust for consent and content preflight;
a bare endpoint ID is not a valid browser invitation.
Public browser lobby discovery is not part of this release because the current
repository rendezvous bootstrap uses native-only Mainline DHT; WebRTC remains a
future transport optimization rather than a safety fallback.
