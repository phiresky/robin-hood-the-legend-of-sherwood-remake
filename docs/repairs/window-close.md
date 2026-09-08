# Orderly native window shutdown

Base: `ecfdfbb10`; committed repair source:
`1dffb5e31c870bb8ad5d307f514b5593e8d1ab5b`.
Scope: native window/game-thread shutdown coordination,
the main menu's OS-close handling, and focused validation. No replay schema,
simulation, Android device setup, or production deployment changes.

## Cause and repair

`WM_DELETE_WINDOW` previously queued `GameEvent::Quit` and immediately stopped
winit. The caller then used `try_recv` before the game could publish its exit
code. A normal close consequently returned exit 1 with a misleading
window/event-loop initialization message.

The event loop now remains responsive while the game handles Quit and cleanup.
The existing unbounded input queue cannot lose Quit through capacity pressure;
a disconnected send is explicitly logged as a failure. No UI-thread join,
blocking receive, timeout-based success, or default exit code was introduced.

A completion guard publishes the game's actual code, closes its result sender,
and only then requests/wakes event-loop exit. Dropping the guard without a code
also closes the sender before waking winit, preserving the explicit abnormal
termination error. The guard is created before the runtime factory/future, so
unstarted-factory and never-polled-future cancellation have the same behavior.
Initialization failures retain exit 1 and nonzero game results are forwarded.

The pinned Cranelift development/test backend does not provide the LLVM-style
unwind behavior assumed by a local `catch_unwind` test. Independent probes
showed local cleanup/catch being skipped and a spawned-thread panic aborting
the process with exit 134. That is a genuine nonzero failure, not successful
shutdown; adding a joining watcher would not help a process-wide abort.
Default tests cover explicit cancellation/drop and a bounded isolated child
panic. A separately selected ignored regression verifies unwind cleanup with
a temporary LLVM override for `robin_rs` only. No tracked toolchain/profile
settings or optimized dependencies are changed.

Two existing event-consumption paths mattered once the premature exit was
removed: loading screens discard ordinary event batches, and nested menus may
interpret Quit as dialog dismissal. `GameWindow` now keeps delivering its
already-latched close request until the application exits. This is a persistent
OS-close request, not fabricated input. The main menu bypasses its confirmation
only for that latch, retaining profile saving and the normal Exit result.
Escape and the menu Exit button still follow the interactive confirmation path.
The OS-close latch also suppresses a simultaneous activated menu action, so a
close/Start event batch cannot reset the campaign or begin another mission.

## Validation

Native desktop builds passed, including the final rebuild after committing the
repair so Git-derived runtime identity names the repaired source. Focused tests
passed (11 passed, 1 explicitly ignored backend-specific test); the separately
selected LLVM unwind test also passed (1 passed). Tests cover publication ordering, actual zero
and nonzero results, panic/drop, cancellation before polling, failed Quit
delivery, and persistence across loading/modal drains.

```sh
CARGO_BUILD_JOBS=1 cargo build --locked -j1 -p robin_rs --bin robin --features desktop
CARGO_BUILD_JOBS=1 cargo test --locked -j1 -p robin_rs --features desktop --profile dev --lib window::tests
CARGO_BUILD_JOBS=1 cargo test --locked -j1 -p robin_rs --features desktop --profile dev \
  --config 'profile.dev.package.robin_rs.codegen-backend="llvm"' --lib \
  window::tests::unwinding_panic_disconnects_before_waking_the_loop -- --ignored --exact
```

The original in-process panic regression failed under Cranelift before it was
split into the explicit backend-appropriate checks above. That failure and
the independent small probes exposed a test assumption, not a normal-close
failure. The fast child test runs only its exact named test with a sentinel,
is bounded to ten seconds, and requires a nonzero child exit.

The diagnostic harness `scripts/validation/window_close.py` runs a separately
built client on an isolated X display with isolated XDG/save directories. It
sends the real X11 `WM_PROTOCOLS/WM_DELETE_WINDOW` message, waits at most 90
seconds for shutdown, and records the exact process return code. A bounded
termination is a failed test, not normal-close success. Modes cover early
startup, loading, startup briefing, advancing gameplay, menu inputs,
and genuinely missing startup data.

Xvfb `:106` used a 1280x960x24 screen and an actual 1024x768 game surface,
with llvmpipe software rendering. Screenshots confirmed the briefing and the
advancing gameplay/HUD. Early-close checks returned 0 after the existing load
finished (about 30 seconds); this repair preserves the request, but does not
make expensive mission loading immediately cancellable. Briefing and gameplay
close on the pre-commit candidate finished in about 0.4 seconds. The final
committed-source gameplay run observed frames 2 to 12 and exited 0 in 1.682
seconds after WM_DELETE_WINDOW. These are functional observations under
concurrent software-rendered validation, not shutdown performance benchmarks.
No bounded-stop termination was needed.

Main-menu close on the committed build returned 0 in 0.116 seconds. Escape and a synthetic Quit-button
probe left the client alive, but captures showed the menu rather than a visible
confirmation dialog. This is not claimed as full rendered-confirmation
coverage. The interactive code branch remains unchanged when the OS-close
latch is false. A preliminary gameplay harness run needed supplemental input
because synthetic keys lacked focus; the corrected standalone run explicitly
focused the window and observed frame progression from 2 to 17 before exit 0.

| Scenario | Executable | Exit | Evidence subdirectory |
| --- | --- | --- | --- |
| Close immediately after window creation | Pre-commit candidate | 0 | `startup` |
| Close while mission loading | Pre-commit candidate | 0 | `loading` |
| Close visible startup briefing | Pre-commit candidate | 0 | `briefing` |
| Close advancing graphical gameplay | Committed `1dffb5e31` | 0 | `committed-gameplay` |
| Close main menu after input probes | Committed `1dffb5e31` | 0 | `committed-menu` |
| Missing datadir startup error | Committed `1dffb5e31` | 1 (expected) | `committed-startup-failure` |

Evidence is retained outside the worktree under
`/tmp/robin-window-close.mwMCVv`, including logs, exact X pixels, screenshots,
and per-run JSON results. Game data is read-only from the absolute main-repo
demo datadir. No user saves or settings are used or changed.
All harness-owned clients and Xvfb `:106` were stopped after validation;
evidence and the worktree remain available for review.

The final retained executable is `robin-1dffb5e31`, SHA-256:

```text
27d1e57a4977b6f3eb8b148211d11cd8266331eacad8cc1a1ba084def93d3be9
```

Its required retail-content-free `assets/core-datadir` is retained alongside it.
An initial relocated-binary trial omitted this closure and correctly failed
startup with exit 1; it is retained as a harness setup failure, not counted as
a successful close. The final committed-source missing-datadir test instead
reports the intended `Unable to install datadir` error and exit 1.

TODO(validation): physical window-manager/platform coverage beyond Xvfb;
hardware rendering/audio checks remain separate from this shutdown repair.
