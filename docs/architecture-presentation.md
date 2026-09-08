# Presentation and audio authority

This pass completes the host side of the mission frame-phase boundary.

- `MissionPresentationPhase` now owns a `HostPresentation` borrow rather than
  `&mut Host`. Render helpers, screenshot/thumbnail composition, interpolation
  and display-refresh pacing cannot reach transport, scripting, application
  effect queues, mutable audio, or a mutable engine/manager.
- Presentation can mutate `HostFrontend`, read sound diagnostics/options and
  the copied local seat, and query active graphics settings. Its application
  reference is private: no general application-context accessor is exposed.
- `MissionAudioPhase` and `tick_audio` now receive immutable `Engine` and
  `ViewportState` references plus the mutable `HostAudio` owner. They return the
  existing `SoundBoundary`; authoritative sound adoption stays in the timeline.
- Cursor command production and background-decal effect draining remain at the
  fixed-tick boundary before rendering. `post_tick_input_phase` explicitly
  selects `post_commands` and `post_external_actions`, not the pre-tick batch.
- Network clock correction and state-hash publication are performed before
  lending presentation authority to the display-rate scheduler. Its callback
  has no host transport access.
- Deferred console output is drained through its text queue, not a whole host.

Behavior preserved: fixed-tick versus display-refresh transient advancement,
camera snapshot/restore, screenshot and save-thumbnail rendering, post-refresh
cleanup commands, audio-before-render and PostInitialize-after-render ordering.
Privileged snapshot, administration, input and simulation boundaries remain
explicit; this change does not alter save/replay formats or simulation state.

Borrowed capabilities intentionally do not implement serialization, following
the existing phase types: they reference process resources, not durable state.

## Verification

The existing source-level phase guard now rejects broad host authority in the
presentation phase, including wrapped host references. New structural assertions
pin audio and presentation capabilities and their read-only/private service
references. Runtime tests check that post-tick producers select only the post
batch and that presentation/audio leave simulation hashes unchanged. A console
test checks exact once-only queue consumption. Existing cadence, frame trace,
camera interpolation, screenshot and frame-contract tests remain applicable.

TODO: record completed package/test commands after the isolated build finishes.

## Remaining scope

`HostFrontend` is still a cohesive but broad mutable frontend borrow; splitting
its input, presentation-transient and resource state further is worthwhile where
it enables smaller consumers. This pass does not claim to make every frontend
field private. The new boundary already excludes unrelated session and service
owners, rather than merely renaming a wrapper around `Host`.

TODO: continue narrowing individual drawing helpers to frontend subdomains;
keep process-local resource ownership distinct from persisted input/settings.
