# Complete-frame manual stepping repair

Base: `ecfdfbb10`. This repair addresses the graphical pending-history crash
documented in `docs/validation/multiplayer.md` and the missing live manual-tick
records documented in `docs/validation/browser.md`. It does not change replay
identity, weaken contiguous history validation, or introduce a replay format.

## Ownership and behavior

The graphical driver now finishes the ordinary frame's modal contributions,
refresh, PostInitialize, history commit, and recorder finalization before
draining manual steps. Each fresh manual tick then owns its own pre-command
checkpoint and complete recorded input. Keyboard and HTTP forward stepping use
the same implementation. Paused replay host records remain stationary and do
not append a simulation-history tick; admitted replay input takes precedence
over a buffered simulation tick at the same timeline position.

An already-open modal dismissed at the request boundary becomes a stationary
host-control replay record, including zero-tick requests. Modals reached during
a manual tick belong to that tick's record. A strict modal policy can stop a
request after execution, but cannot omit that already-executed tick from the
recorder. Fresh live ticks also consume pending external facts and persist the
effective hourglass gate.

Multiplayer uses the separate repair's explicit host-resynchronization hook
before requesting the replacement snapshot barrier. Requests are rejected
before mutation unless initial admission/start or the preceding resynchronization
has reached Running.

## Backwards movement boundary

Existing backward controls remain enabled. Forward scrubbing of retained
history does not append old timeline positions to the live recorder; returning
to its existing frontier preserves the linear stream. Reconstructing a retained
frame and replaying its complete input is covered by the live-recorder test.

Creating a new recorded branch, or recording stationary host changes while
behind that frontier, is **not repaired here**. A raw debugger checkpoint is not
a canonical game save: the existing replay save/load path applies serialized
save projection and post-load synchronization. `ReplayLoadBack` contains only
`to_frame` and `is_continue` and has a fixed binary codec shape; the streaming
writer also permits markers only at its current ordinal. Faithfully encoding
arbitrary backward branches therefore requires coordinated checkpoint semantics,
ordinal association, codec/schema and verifier work. No fake save marker, dropped
journal, blanket rewind disablement, or format-compatibility claim substitutes
for that work.

## Verification

At source `d5aaed8f9`, both commands passed:

```sh
CARGO_BUILD_JOBS=1 cargo test --locked -p robin_rs --lib game_session::tick::tests --features multiplayer
CARGO_BUILD_JOBS=1 cargo test --locked -p robin_rs --lib game_session:: --features multiplayer
```

Results: 13/13 stepping tests and 184/184 broader mission-session tests. New
coverage uses a real file-backed recorder, complete normal-frame late inputs,
four fresh manual ticks, initial modal dismissal, dense ordinals, exact replay
versus history inputs, replay execution hashes, backwards/buffered-forward
reconstruction with the recorder still attached, rewound modal dismissal and
zero-tick requests, paused replay records, and strict-modal error finalization.
Readiness tests reject all pre-Running multiplayer step admissions before
mutation. `ManualTransactionBegin` makes each independent recorder lifecycle
explicit without weakening the diagnostic duplicate-phase assertion.

## Actual graphical acceptance

### Final combined repair snapshot: passing

Repeated the entire standalone graphical run, export and graphical replay on
final combined source `185ea5783905d4ce2cb470a2525e64883275106e` (production
content matches root integration `83f87b1d1`). All five harness checks passed.

- Binary: `/tmp/robin-multiplayer-repair-qyUh8ZdY/robin-185ea5783-multiplayer`.
- Binary SHA-256: `4a9df31e4ae89c56d6ba965369b514dd34a328d6dc64b9a31a15966a6a110814`.
- Evidence: `/tmp/robin-frame-step-repair-185ea5783-final`.
- Export SHA-256: `e6da9dba380ce73f745c3d8c9251a0c70747f7d734513ac22115ce4d7227979a`.

The live scene advanced from observed frame 26 through normal frame 27 and
four requested ticks to 31, then from paused frame 32 through 30 requested
ticks to 62. Compact export succeeded. The same binary's graphical replay
finished all **65 records**, verifying hashes at ordinals 0, 25 and 50
(`1ebb60a16a484703`, `6badc09f1fc644bb`, `f7d832c1f1f02841`) without desync.
The scene screenshot is `live.png`. All owned harness processes completed and
were cleaned up; no build/game process is left running by this repair track.
This final repeat includes the merged modal/window/save/pacing fixes, not just
the earlier stepping-only snapshot. Headless playback was not relabelled
passing; its earlier distinct bootstrap failure remains described below.

### Earlier isolated repair evidence

The standalone driver `scripts/validation/frame_steps_live.py` ran in fresh
loopback-only user/network namespaces with unused save/config/cache/identity
directories, Xvfb and the real Leicester Demo corpus. It used the multiplayer
owner's immutable combined binary (all stepping fixes plus their complete
resynchronization repair), source `762225be1`:

- Binary: `/tmp/robin-multiplayer-repair-qyUh8ZdY/robin-762225be1-multiplayer`.
- Binary SHA-256: `0173cc5c57b6403f63139fbbb1294fc97d4b711d25d18a6f2db737496cb5163b`.
- Run02 evidence: `/tmp/robin-frame-step-repair-d5aaed8f9-02`.
- Run04 evidence: `/tmp/robin-frame-step-repair-d5aaed8f9-04`.

Run02 observed ordinary graphical frame 6, then a request serviced after the
next normal frame: `from_frame=7`, `advanced=4`, `frame=11`. After pausing at
frame 12, another request advanced 30 ticks to 42. Canonical compact export
succeeded. `live.png` visibly shows the populated Leicester scene and HUD;
this is not a pixel-parity or physical-GPU performance claim.

Run04 loaded **that exact export** using the same executable's graphical
replay driver. It verified frame-0 hash `1ebb60a16a484703`, frame-25 hash
`845d46e8367d2517`, and finished all 45 records without desync. The compact
file's SHA-256 is
`85361f375ba2dcb4e0af00d4697478a814e779a462429adc9d1a23077512132b`.
This establishes real graphical normal-frame/manual-step transaction ordering,
live manual recording, canonical export and complete graphical playback.

Reproduce after the separate native build, with a NEW evidence directory:

```sh
timeout --signal=TERM --kill-after=15s 330s unshare --user --map-root-user --net \
  python3 scripts/validation/frame_steps_live.py \
  --binary /tmp/robin-multiplayer-repair-qyUh8ZdY/robin-185ea5783-multiplayer \
  --data /home/phire/robinhood/datadirs/demo_leicester_ecoste \
  --snapshot 185ea5783905d4ce2cb470a2525e64883275106e \
  --evidence /tmp/NEW-FRAME-STEP-EVIDENCE --graphical-replay
```

`--replay-file <existing-export>` runs just the playback half, preserving the
original bytes. The driver retains its source, summary, unfiltered logs and
artifacts; terminates its own game/Xvfb processes; and stages the required
core overlay when testing an executable retained outside its install layout.
Private runtime identity files remain private and must not be published.

### Distinct headless bootstrap limitation

Run02's initial **headless** replay attempt failed before any manual tick:
frame-0 hash was `4d36b4f2ddccd015` instead of `1ebb60a16a484703`, and the
strict bootstrap save-marker guard rejected it. Run03 independently repeated
successful GUI stepping/export (24→25+4→29, pause at 30, +30→60) followed by
the same headless failure. The passing graphical replay of the identical file
isolates this as a cross-driver bootstrap discrepancy; it is not relabelled
headless acceptance, attributed to manual stepping, or hidden by weakening
hash/marker validation. The subsequent explicitly authorized bootstrap repair
below addresses that independently identified cause.

Run01 stopped before gameplay because the copied executable initially lacked
its required install-side core overlay; staging was corrected without changing
production code. No original validation artifact was overwritten. The
multiplayer repair owner retains separate two-peer reconnect results; this
document does not duplicate or substitute for those results.

The separate worktree build also passed:
`CARGO_BUILD_JOBS=1 cargo build --locked -p robin_rs --bin robin --features multiplayer`
(12m48s, default worktree `target/`, no output filtering or clippy). Existing
unrelated warnings remain; no build warning was treated as a runtime failure.

### Follow-up: true-headless bootstrap and recorded popup scheduling

Commit `0dfc83581` shares generated Merry Men name registration between the
graphical and CPU-only bootstrap. Both already load the same authored name
pools into `LevelAssets`. The graphical call remains at its original point,
after audio setup and before seat/snapshot initialization; headless now runs
the identical auxiliary-RNG operation there without constructing a window,
renderer, portrait cache or UI. Engine RNG, state hash definition and replay
schema are unchanged. Empty pools retain the existing warning/no-registration
behavior. The exhausted-pool display-only label is unchanged.

An actual paused old-binary headless engine dump in
`/tmp/robin-headless-names-185ea5783-before/bootstrap.engine.json` confirms
empty `campaign.peasant_names`. Both graphical logs previously registered
Peter Hunter, Matt Little and Peter Chopper. The repaired headless log registers
the same three names and verifies the original frame-zero hash
`1ebb60a16a484703`. The old compact export was preserved unchanged; new
acceptance uses fresh recording/export from each committed candidate, never
relabels an artifact's engine identity.

That successful bootstrap exposed a separate strict replay-modal bug:
headless eagerly dropped the pending popup, then rejected its subsequently
recorded dismissal as unused. Commit `1a698f23d` disables eager automation
only during replay, retains pending effects until their matching recorded
host boundary, and preserves the unused-control assertion. Engine effects
of the decision remain owned by the recorded tick input, not synthesized
again by the headless presentation drain. Live-headless automation is unchanged.

Actual graphical normal/manual stepping and compact export followed by
**true-headless playback to EOF** passed at `1a698f23d`:

- Evidence: `/tmp/robin-headless-names-1a698f23d-live-headless`.
- Binary: `/tmp/robin-headless-names-candidate-58hQJRWo/robin-1a698f23d`;
  SHA-256 `4979eb2f5027f9fc4d3258798298fd37b22c2e03d473b53cfb340803584226a0`.
- Normal frame 7 plus four manual ticks reached 11; pause at 12 plus thirty
  manual ticks reached 42. All 46 exported host records replayed to EOF.
- Hash 0: `1ebb60a16a484703`; hash 25: `f29f910f4a2b525a`; no desync.
- Compact SHA-256:
  `e2b51c14aa3a7803b09aa2804e78fef347f4b59043373267d35f42540e9bd7e7`.
- Setup tests: 15/15; combined `game_session::` tests at the bootstrap commit:
  190/190; headless tests after scheduling repair: 5/5. Separate native
  multiplayer builds passed at each committed candidate.

The first candidate's fresh export also exposed a graphical input race:
physical Return could dismiss a popup before its recorded stationary host
record. Both failures remain retained under
`/tmp/robin-headless-names-0dfc83581-{live-headless,graphical}`; they are not
reported as graphical EOF acceptance. The graphical modal owner is repairing
that separate input-admission boundary before combined final validation.

Known boundary: full terminal/headless batch ownership is not implemented by
this popup scheduling repair. In particular an aborted scripted-debriefing
batch must retire only its own active siblings, not newly emitted effects.
Headless replay now rejects that unsupported control explicitly **before**
mutating effects or acknowledging any controls; the regression checks both
queues remain intact. This is not claimed as FinalDebriefing/EndState parity.
Live-headless automation is unaffected.

### Paired popup-path acceptance after graphical input admission repair

Combined source `f4d9227293b5e481d63bc2d47c401ac8c65f0f2a` includes the
graphical modal owner's `bd96b476f` strict recorded-dismissal admission and
the headless abort preflight. The latter returns an error before mutation;
its test does not rely on catching a panic under the Cranelift abort profile.
Combined `game_session::` tests passed **196/196**; the separate native
multiplayer build and formatting checks passed.

Fresh actual GUI recording and the **identical compact export** passed both
true-headless and graphical playback to EOF. The graphical harness continued
sending real Return presses once a second, reproducing the input pressure that
caused the previous unused-popup failure. Evidence:

- `/tmp/robin-headless-names-f4d922729-live-headless`
- `/tmp/robin-headless-names-f4d922729-graphical`
- Read-only binary `/tmp/robin-headless-names-candidate-58hQJRWo/robin-f4d922729`,
  SHA-256 `ac1be59fd19537220660a638c7beebaf83c1770fb53a7136a2033ce9bf699e24`.
- Normal frame 22 plus four manual ticks reached 26; pause at 27 plus thirty
  manual ticks reached 57. Both drivers consumed all **61 host records**,
  including the stationary ordinal-2 popup dismissal at timeline 2→2.
- Both verified hashes 0=`1ebb60a16a484703`, 25=`845d46e8367d2517`,
  50=`22d468eecc417689`, with no desync.
- Unchanged compact SHA-256:
  `62f0ad6d81787a3ce76ad14213b87b81972272a0fef5271bb9ecc0422ecd38fa`.

This is popup-path acceptance, not an audio-dialogue assertion: subsequent
review requested a separate graphical dialogue speech/presentation correction.
Final combined-source validation must include that owner's follow-up rather
than extrapolating this popup fixture to speech or terminal debriefings.

### Final production-freeze acceptance

Production/build source was exactly
`51db7aa06099b54481eb1124df8fa04dd8af6a68` (fast-forwarded, no different merge
identity), including pending replay restart and the final dialogue-audio
follow-up. The separate multiplayer desktop build passed in 22.37 seconds.
The same live/manual/export and paired-driver experiment passed again:

- Immutable binary:
  `/tmp/robin-headless-names-candidate-58hQJRWo/robin-51db7aa06`.
  Binary SHA-256:
  `cc5269df8832e7036161cba5de510527564469d50a10bc615412c284e84ce137`.
  Required tracked `assets/` and `mods/` are retained beside it outside the
  worktree; execution used isolated runtime/save directories and real game
  data, without writing user saves.
- Evidence: `/tmp/robin-headless-names-51db7aa06-live-headless` and
  `/tmp/robin-headless-names-51db7aa06-graphical`. Each retains its exact
  driver/helper source, invocation summary and full logs. All owned game and
  Xvfb processes were cleaned up by the driver.
- Ordinary frame 21 plus four manual ticks reached 25; pause at 26 plus
  thirty manual ticks reached 56. Both drivers replayed all **60 records**
  to EOF, including stationary ordinal-2 popup dismissal at timeline 2→2.
- Both verified hashes 0=`1ebb60a16a484703`, 25=`845d46e8367d2517`,
  50=`22d468eecc417689`; no desync. Graphical playback retained recurring
  physical Return pressure rather than removing the original failure trigger.
- Identical compact-export SHA-256:
  `5ce15cbf223786162f853459450f3b1148749a401282faf048f4bd64e94531ec`.
- The actual screenshot was visually checked for populated Leicester map
  and HUD. PNG SHA-256:
  `fc1a5dbdaf0f35237e1567f7d560820bf16b3be0dc0ad8a622d4752e6514124b`.

This final run validates the combined production snapshot on the exercised
popup/manual-step path. It does not turn the no-sound fixture into an audio
test or claim unsupported terminal/debriefing semantics. The separately
owned final suite and browser checks are reported by their owners.
