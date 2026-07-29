# Windows-binary portability parity

## Goal and authority

This audit maps every finding in
`original-code/CAST_BUGS.md` and
`original-code/PORTABILITY_AUDIT.md` to the Rust implementation.

For behavior, the authority order is:

1. the GOG Windows retail `Game.exe`;
2. the corrected C/C++ source in `original-code`;
3. the Rust implementation.

The Windows executable performs the affected floating-point calculations on
x87. The C/C++ parity build deliberately uses SSE (`-mfpmath=sse`), and Rust
normally rounds `f32` after every operation. Merely spelling the same
expression in all three therefore does not guarantee the same integer at a
truncation boundary. The corrected C/C++ now promotes the original binary32
inputs and constants to `double`; Rust promotes those same values to `f64`.
This reproduces the retained x87 intermediates while preserving the authored
binary32 constants.

## Confirmed calculation bugs

| Calculation | Windows/C++ behavior | Rust result |
| --- | --- | --- |
| Camera edge scrolling | Compare signed floating scroll values and clamp against map boundaries; never convert a negative scroll to an unsigned word. | Already matched. `engine/camera.rs::perform_check_scroll` retains signed floating vectors, and its left-boundary test covers negative motion directly. |
| Campaign reinforcements | Compute `min + trunc(warcrime * (max - min))`. The complete warcrime and range calculation remains extended until conversion. | Corrected in `campaign.rs::calculate_warcrime_recruitment`; previous Rust deliberately reproduced the Linux premature cast. |
| AI `ValueBetween` | Compute the complete interpolation before conversion. `0.01f` retains its binary32 value in the x87-width expression. | Corrected in `ai/controller.rs::value_between`. The `f64` expression intentionally retains the promoted binary32 constant, including Windows truncation-boundary results such as `(0, 100, 100) -> 99`. |
| Reservist experience | Multiply experience by the binary32 coefficient in extended precision, then convert. | Corrected in `pc_status.rs::scale_experience`; `80 * 1.5` now becomes 120 before promotion rollover. |
| Reservist life | Multiply life by 1.5 before converting and clamping to 100. | Corrected in `campaign.rs::move_to_gang`; 50 life now becomes 75. |
| Music diagnostic meters | Scale `weight / 256.0 * 128.0` before converting. | Already matched. `game_session/render.rs::fill_display_bar` uses the exact integer identity `(weight * 128) / 256` for the valid 0–256 range. |
| Deafness diagnostic ellipse | Convert `radius * zoom`, then convert `long_axis * ASPECT_RATIO`; both products use promoted binary32 inputs. | Already had the correct operation order. `draw_manager.rs::draw_ellipse` now also models the Windows x87 truncation boundaries explicitly. |
| Bow distance percentage | Compute `trunc((distance / range) * 100)` before selecting 20-point buckets. | Corrected in `weapons.rs::get_hit_chance`; previous Rust forced every in-range distance into the zero-distance bucket. |
| NPC alert/sorrow color | Compute `trunc(value * 0.001f * 32.0f)` after both scales. | Corrected in `alert_colors.rs::npc_tint`; intermediate values now select the gradient instead of remaining at entry zero. Rust retains a final table bound for invalid/out-of-contract values. |

The corrected C/C++ expressions were checked against these Windows retail
instruction ranges:

- campaign reinforcements: `0x004e3463–0x004e347d`;
- AI interpolation: `0x0041a0f0–0x0041a136`;
- experience scaling: `0x0051d550–0x0051d57b`;
- life scaling: `0x00455c45–0x00455c60`;
- music meters: starting at `0x00511dd9`;
- deafness ellipse: `0x004d4c98–0x004d4cc4`;
- bow percentage: `0x00450107–0x0045011a`;
- alert/sorrow indices: `0x0048a7a8–0x0048a7ed`.

## C/C++ portability defects and Rust equivalents

| C/C++ finding | Rust handling |
| --- | --- |
| Forced Amiga `va_list` representation | Not applicable. Rust has no C variadic forwarding in these paths. |
| Negative seeks routed through unsigned `ULONG` | Safe. `SbFile::skip` accepts `i64` and uses `SeekFrom::Current/End(i64)`. |
| Mutation through `const` path references | Safe. `resolve_case_insensitive` normalizes into owned `String`/`PathBuf` values. |
| Wide-format capacities supplied as byte counts | Not applicable. Rust formatting writes owned UTF-8 strings and does not expose a `wchar_t` element-count API. |
| Incorrect `mbstowcs` bounds and result test | Not applicable. Rust string conversion uses checked UTF-8/string APIs. |
| Four-byte resource tag copied with pointer size | Safe. Level tags are typed as `[u8; 4]` and read as exactly four bytes. |
| VIP table iterated with byte size | Safe. Rust resolves VIP identity through profile data/`CharacterKind`; collection access is bounds checked. |
| Console `new[]`/`delete` mismatch and signed `toupper` input | Safe. Console tokens are owned strings/vectors and command normalization uses `to_ascii_uppercase`. |
| Renderer `va_list` casts and missing `va_end` | Not applicable. Rust formatting has no `va_list` lifetime. |
| Dynamic strings reused as format strings | Safe. `format!` has a compile-time literal format; dynamic strings are values, never format programs. |
| Temporary `SBInput()` mistaken for constructor delegation | Not applicable. `ThreadedInput::new` calls `Default` directly and all fields are initialized. |

## Architecture-dependent C/C++ constructs

The C/C++ audit also records host-width risks rather than current 32-bit
behavioral bugs. Rust does not reproduce them:

- persistent numeric fields use explicit `u8/u16/u32/i16/i32/f32` types;
- level and resource readers use `from_le_bytes` with explicit byte widths;
- runtime object references use typed IDs/indices instead of pointer-to-`ULONG`
  round trips;
- enums are represented explicitly and serialized through typed
  `serde` state or field-specific legacy readers;
- native `wchar_t`, native `bool`, and pointer-sized placeholders are not
  written into legacy data structures.

This is intentionally format compatibility, not C++ object-layout
compatibility.

## Bugs and quirks present in Windows retail

These remain unchanged because fixing them would diverge from the original
Windows behavior:

- The `GOTO_NOHALT` precedence bug is preserved in
  `engine/movement.rs`: ordinary AI `GoTo` does not perform the apparent
  pre-launch halt.
- Both single-circle gamepad branches produce `ThrustH` in
  `gamepad.rs::recognize_swing`.
- Minimap priority packs `is_soldier` into both bits 14 and 13 in
  `minimap.rs::element_priority`.

## Dormant and invalid-input findings

The remaining C/C++ audit findings do not require Rust parity mutations:

- There is no Rust equivalent of the unused broken `SBFile(FILE*, bool)`
  constructor.
- Rust `Waypoint` has no fake comparison operators; paths keep authored vector
  order and index directly.
- Rust input state uses initialized typed fields rather than clearing one C
  struct with another struct's `sizeof`.
- `Campaign::get_progression` uses checked division and returns zero for an
  empty ordinary-mission set. Windows shipped data never exercises that
  invalid campaign shape.
- Projection-area selection carries the current best candidate in an
  `Option`, so an all-nonpositive-height candidate set cannot return an
  uninitialized pointer.
- The `_DEBUG` surface-ID precedence expressions and dead
  `PrintConsoleEx` semicolon have no Rust equivalents.
- Sound-cache entries live in `Vec`/`BTreeMap`; Rust never applies `realloc`
  byte moves to live string objects.

The two unconfirmed bounds concerns are also safe in the Rust implementation:
mouse-trail columns are explicitly clipped to the viewport, and fast-grid
cell ranges are clamped before block access in
`get_active_motion_lines_for_segments`.

## Verification

Focused tests cover the six corrected Rust gameplay calculations, including
the x87-sensitive AI interpolation endpoints. Existing tests cover signed
camera clamping and the three intentionally retained original quirks.

The complete `robin_engine` test run during this audit was not a clean
baseline because the surrounding in-progress parity work had unrelated
failures. Those failures must not be attributed to this calculation set; the
focused tests are the acceptance boundary for this audit.
