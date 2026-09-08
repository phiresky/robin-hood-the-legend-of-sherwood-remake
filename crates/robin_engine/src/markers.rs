//! Visual markers drawn on the ground — sim state.
//!
//! Destination/move markers and character selection circles. The host-side
//! renderer lives in `robin_rs::markers`.

/// Number of animation frames in the ground mark sprite.
pub const NUMBER_OF_GROUND_FRAMES: u16 = 6;

/// Number of frames in the selection mark ping-pong animation.
const NUMBER_OF_SELECT_FRAMES: u32 = 9;

/// How many ticks per animation step for selection marks.
const SLOWERATOR: u32 = 3;

// ---------------------------------------------------------------------------
// GroundMark
// ---------------------------------------------------------------------------

/// One active destination marker on the ground.
///
/// `x`/`y` is the sprite's top-left corner in world coordinates —
/// `add_mark` subtracts the sprite half-diagonal from the click position at
/// add time and stores the result, which is then used directly as the
/// sprite's top-left at render time.
///
/// `current_frame` is the *next* frame the animation will advance to;
/// `render_frame` is the frame the renderer should draw this tick. The
/// per-mark `{ render; advance; retire }` body must run as one
/// indivisible unit — see `tick` for the sequencing.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct GroundMarkEntry {
    pub x: f32,
    pub y: f32,
    pub layer: u16,
    pub current_frame: u16,
    /// Pre-advance snapshot of `current_frame` taken at the start of each
    /// `tick`; this is the frame the renderer reads. Required because the
    /// per-mark `{ render; advance; retire }` body must run as one
    /// indivisible unit, whereas here `tick` runs inside
    /// `perform_hourglass` (sim-state mutation) and rendering runs after
    /// the hourglass — splitting them on `current_frame` alone would skip
    /// frame 0 (advanced before first render) and frame
    /// `NUMBER_OF_GROUND_FRAMES - 1` (retired before final render).
    pub render_frame: u16,
}

/// Manager for the list of active destination markers.
#[derive(
    Debug,
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct GroundMark {
    pub marks: Vec<GroundMarkEntry>,
    /// Sprite half-diagonal (half-width, half-height) in world pixel units.
    /// Set by the host when the ground-focus sprite resource is loaded.
    /// `add_mark` subtracts this from the click position so the stored
    /// `x`/`y` is the sprite top-left.
    sprite_half_diagonal: Option<(f32, f32)>,
    /// Per-frame sprite sizes `(w, h)` in world pixel units, indexed by
    /// `current_frame`. Set at load time alongside `sprite_half_diagonal`.
    /// Needed inside `tick` for the per-frame on-screen test without
    /// poking at host-owned surfaces.
    frame_sizes: Vec<(u16, u16)>,
    /// Per-frame `(x, y)` offset in world pixel units — the auto-crop
    /// origin (the `(x_min, y_min)` of the opaque region) recorded when
    /// the packed sprite was created.  The on-screen test adds this to
    /// the sprite position before computing the AABB.  Same length as
    /// `frame_sizes`; empty when the host couldn't extract per-frame
    /// metadata, in which case the cull falls back to a zero offset
    /// (uncropped behaviour).
    per_frame_offsets: Vec<(i16, i16)>,
}

impl GroundMark {
    /// Record the sprite's half-diagonal and per-frame sizes. Called once
    /// per session after loading the ground-focus sprite resource row.
    /// Stores the move-box half-diagonal plus the per-frame packed-sprite
    /// sizes used by the on-screen test inside `tick`.
    pub fn set_sprite_data(
        &mut self,
        half_w: f32,
        half_h: f32,
        frame_sizes: Vec<(u16, u16)>,
        per_frame_offsets: Vec<(i16, i16)>,
    ) {
        self.sprite_half_diagonal = Some((half_w, half_h));
        self.frame_sizes = frame_sizes;
        self.per_frame_offsets = per_frame_offsets;
    }

    pub fn add_mark(&mut self, x: f32, y: f32, layer: u16) {
        let (hx, hy) = match self.sprite_half_diagonal {
            Some(v) => v,
            None => {
                tracing::warn!(
                    "GroundMark::add_mark called before sprite data was set; \
                     skipping (sprite resource not loaded yet)"
                );
                return;
            }
        };
        // Prepend the new entry, so the list is ordered newest-first.
        // Both render and retire iterate forward, so when two marks
        // overlap the older mark blits on top of the newer one.
        self.marks.insert(
            0,
            GroundMarkEntry {
                x: x - hx,
                y: y - hy,
                layer,
                current_frame: 0,
                render_frame: 0,
            },
        );
    }

    /// Per-tick state advancement (the mutating half of mark refresh).
    ///
    /// Sequence per tick: (1) retire marks that crossed
    /// `NUMBER_OF_GROUND_FRAMES` on the *previous* tick — they get one
    /// final render at frame `NUMBER_OF_GROUND_FRAMES - 1` before being
    /// dropped, since `{ render; advance; retire }` must run as one
    /// indivisible per-mark body.  (2) snapshot
    /// `render_frame = current_frame` for the renderer.  (3) on even
    /// universal-frame-counter ticks, advance `current_frame`.
    ///
    /// Engine-owned command marks call this from
    /// `EngineInner::perform_hourglass`; host-owned preview marks call it
    /// from the same hourglass cadence while staying outside the
    /// deterministic sim snapshot.
    pub fn tick(
        &mut self,
        _view_pos: geo::Coord<f32>,
        _zoom: f32,
        _screen_w: i32,
        _screen_h: i32,
        frame_counter: u32,
    ) {
        // (1) Retire entries whose animation finished on the previous
        // tick. Delaying retirement by one tick is what gives the final
        // pre-retire frame its render.
        self.marks
            .retain(|m| m.current_frame < NUMBER_OF_GROUND_FRAMES);

        // (2) Snapshot the current frame as the render target.
        for mark in &mut self.marks {
            mark.render_frame = mark.current_frame;
        }

        // (3) Advance on even ticks. The original game gated this on the
        // render viewport. Rust keeps deterministic marker state in the
        // engine, which does not own the player's actual viewport; using the
        // cutscene camera here made visible marks freeze forever after the
        // player scrolled. Always advancing preserves the intended visible
        // animation and guarantees retirement.
        // TODO: Restore off-screen freezing if the deterministic simulation
        // gains an authoritative per-player viewport.
        let advance_this_tick = frame_counter.is_multiple_of(2);
        if advance_this_tick {
            for mark in &mut self.marks {
                mark.current_frame = mark.current_frame.saturating_add(1);
            }
        }
    }

    /// Per-frame auto-crop offset (`x_min`, `y_min` of the opaque region)
    /// in world pixel units.  Read-only access for the render path's
    /// on-screen cull, which adds this offset to the sprite position
    /// before testing the AABB.  Empty when the host couldn't extract
    /// per-frame metadata.
    pub fn per_frame_offsets(&self) -> &[(i16, i16)] {
        &self.per_frame_offsets
    }

    pub fn clear(&mut self) {
        self.marks.clear();
    }

    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

// ---------------------------------------------------------------------------
// SelectionMark
// ---------------------------------------------------------------------------

/// Global animation state for the rotating selection circle.
#[derive(
    Debug,
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SelectionMark {
    pub forward: bool,
    pub frame: u16,
    pub sub_tick: u16,
}

impl SelectionMark {
    pub const NUM_FRAMES: u16 = NUMBER_OF_SELECT_FRAMES as u16;

    pub fn new() -> Self {
        Self {
            forward: true,
            frame: 0,
            sub_tick: 0,
        }
    }

    pub fn tick(&mut self) {
        self.sub_tick += 1;
        if (self.sub_tick as u32) < SLOWERATOR {
            return;
        }
        self.sub_tick = 0;

        if self.forward {
            self.frame += 1;
            if self.frame as u32 == NUMBER_OF_SELECT_FRAMES {
                self.frame -= 1;
                self.forward = false;
            }
        } else if self.frame == 0 {
            self.forward = true;
        } else {
            self.frame -= 1;
        }
    }

    pub fn animation_frame(&self) -> u16 {
        self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_mark_add_applies_half_diagonal() {
        let mut g = GroundMark::default();
        // Before the sprite row is loaded, adds are refused.
        g.add_mark(10.0, 20.0, 1);
        assert!(g.is_empty());

        g.set_sprite_data(8.0, 6.0, vec![(16, 12); 6], vec![(0, 0); 6]);
        g.add_mark(100.0, 50.0, 2);
        assert_eq!(g.len(), 1);
        // Stored position is top-left: click − half-diagonal.
        assert_eq!(g.marks[0].x, 92.0);
        assert_eq!(g.marks[0].y, 44.0);
        assert_eq!(g.marks[0].current_frame, 0);
    }

    #[test]
    fn ground_mark_tick_retires_finished() {
        let mut g = GroundMark::default();
        g.set_sprite_data(
            0.0,
            0.0,
            vec![(1, 1); NUMBER_OF_GROUND_FRAMES as usize],
            vec![(0, 0); NUMBER_OF_GROUND_FRAMES as usize],
        );
        g.add_mark(0.0, 0.0, 0);
        g.marks[0].current_frame = NUMBER_OF_GROUND_FRAMES - 1;
        // Tick on an odd frame: no advancement, retain keeps the entry.
        g.tick(geo::Coord { x: 0.0, y: 0.0 }, 1.0, 10, 10, 1);
        assert_eq!(g.len(), 1);
        // Tick on an even frame: advances to NUMBER_OF_GROUND_FRAMES.
        // Retirement is delayed by one tick (final pre-retire render
        // happens this tick) — entry survives, render_frame == 5.
        g.tick(geo::Coord { x: 0.0, y: 0.0 }, 1.0, 10, 10, 2);
        assert_eq!(g.len(), 1);
        assert_eq!(g.marks[0].render_frame, NUMBER_OF_GROUND_FRAMES - 1);
        assert_eq!(g.marks[0].current_frame, NUMBER_OF_GROUND_FRAMES);
        // Next tick: retain at start of tick removes the finished entry.
        g.tick(geo::Coord { x: 0.0, y: 0.0 }, 1.0, 10, 10, 3);
        assert!(g.is_empty());
    }

    #[test]
    fn selection_mark_ping_pong() {
        let mut s = SelectionMark::new();
        for _ in 0..(NUMBER_OF_SELECT_FRAMES * SLOWERATOR) {
            s.tick();
        }
        assert!(!s.forward);
    }
}
