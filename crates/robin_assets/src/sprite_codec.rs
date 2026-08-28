//! Context-model codec for VQ sprite tile-index grids.
//!
//! Character sprites in the shipping bank are vector-quantised: a sprite is a
//! `(width/4) x height` grid of tile indices into a per-character dictionary
//! (see `docs/COMPRESSION.md`, "Sprite research" 2026-08-28). zstd compresses
//! that stream to roughly its order-0 entropy — LZ cannot see the 2-D grid.
//! This module codes each index with an adaptive PPM-style model driven by
//! its 2-D neighborhood, and optionally by the tile at the same position in a
//! *base* character (cross-variant coding for palette families):
//!
//!   standalone: (above, left) -> above -> order-0 -> uniform
//!   vs base:    (base, above) -> base -> above -> order-0 -> uniform
//!
//! Escapes use PPMC (escape weight = number of distinct symbols seen in the
//! context) with full exclusion: symbols ruled out by an escape at a more
//! specific level cost no probability mass further down (measured ~-3%).
//! Counts update at every level on each symbol, and each context halves its
//! counts when they saturate, which keeps the model adaptive and the
//! range-coder totals well inside precision.
//!
//! The entropy stage is a carry-aware LZMA-style range coder rather than
//! rANS: rANS emits symbols last-in-first-out, which fights adaptive
//! context models (the decoder must see updates in encode order), while a
//! range coder is FIFO and pairs with adaptation naturally.
//!
//! Measured on `datadirs/fullgame_linux` (see COMPRESSION.md): ~-33% vs
//! zstd-22 standalone, ~3.9x on family variants coded against their base;
//! the whole character corpus comes out 2.27x smaller than zstd-19.

use std::collections::HashMap;

use anyhow::{Result, anyhow};

// ---------------------------------------------------------------------------
// Range coder (LZMA-style, 32-bit range, byte renormalisation)
// ---------------------------------------------------------------------------

const RC_TOP: u32 = 1 << 24;

struct RangeEncoder {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    out: Vec<u8>,
}

impl RangeEncoder {
    fn new() -> Self {
        Self {
            low: 0,
            range: u32::MAX,
            cache: 0,
            cache_size: 1,
            out: Vec::new(),
        }
    }

    /// Encode a symbol occupying `[start, start+size)` of `total`.
    fn encode(&mut self, start: u32, size: u32, total: u32) {
        debug_assert!(size > 0 && start + size <= total);
        let r = self.range / total;
        self.low += start as u64 * r as u64;
        self.range = r * size;
        while self.range < RC_TOP {
            self.shift_low();
            self.range <<= 8;
        }
    }

    fn shift_low(&mut self) {
        if self.low < 0xFF00_0000 || self.low > 0xFFFF_FFFF {
            let carry = (self.low >> 32) as u8;
            self.out.push(self.cache.wrapping_add(carry));
            for _ in 1..self.cache_size {
                self.out.push(0xFFu8.wrapping_add(carry));
            }
            self.cache = (self.low >> 24) as u8;
            self.cache_size = 0;
        }
        self.cache_size += 1;
        self.low = (self.low << 8) & 0xFFFF_FFFF;
    }

    fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        self.out
    }
}

struct RangeDecoder<'a> {
    range: u32,
    code: u32,
    input: &'a [u8],
    pos: usize,
}

impl<'a> RangeDecoder<'a> {
    fn new(input: &'a [u8]) -> Self {
        let mut d = Self {
            range: u32::MAX,
            code: 0,
            input,
            pos: 1, // first byte is the encoder's initial cache byte (0)
        };
        for _ in 0..4 {
            d.code = (d.code << 8) | d.next_byte() as u32;
        }
        d
    }

    fn next_byte(&mut self) -> u8 {
        let b = self.input.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    /// Returns a value in `[0, total)`; caller finds the symbol whose
    /// interval contains it and confirms with `commit`.
    fn decode_target(&mut self, total: u32) -> u32 {
        let r = self.range / total;
        (self.code / r).min(total - 1)
    }

    fn commit(&mut self, start: u32, size: u32, total: u32) {
        let r = self.range / total;
        self.code -= start * r;
        self.range = r * size;
        while self.range < RC_TOP {
            self.code = (self.code << 8) | self.next_byte() as u32;
            self.range <<= 8;
        }
    }
}

// ---------------------------------------------------------------------------
// PPM contexts
// ---------------------------------------------------------------------------

/// Halve a context's counts once its coder total reaches this.
const CTX_HALVE_LIMIT: u32 = 1 << 14;

/// Count increment per observation. Measured on the character corpus:
/// faster adaptation (4) lost 4-9% — these streams are stationary within a
/// character, so slow, stable statistics win.
const BUMP: u16 = 1;

/// One adaptive context: seen symbols with counts, in insertion order.
#[derive(Default)]
struct Ctx {
    syms: Vec<(u16, u16)>,
    sum: u32,
}

enum CtxCode {
    /// (cum, freq, total) of the coded symbol.
    Sym(u32, u32, u32),
    /// (cum, freq, total) of the escape.
    Escape(u32, u32, u32),
    /// Context was empty: nothing is coded (probability-1 escape).
    Empty,
}

/// Escape weight for a context with `distinct` non-excluded symbols: PPMC.
/// PPMD-style `distinct.div_ceil(2)` was measured worse net: +1.3..1.9% on
/// standalone characters (the bulk of the coded bytes), -0.3..0.5% on
/// cross-variant streams.
#[inline]
fn escape_weight(distinct: u32) -> u32 {
    distinct
}

impl Ctx {
    fn total(&self) -> u32 {
        self.sum + self.syms.len() as u32
    }

    /// Locate `x` for encoding, ignoring symbols in `excl` (PPM exclusion:
    /// an escape at a more specific level proves the symbol is none of that
    /// level's candidates, so they must not cost probability mass here).
    fn code_for(&self, x: u16, excl: &Excl) -> CtxCode {
        let mut cum = 0u32;
        let mut found: Option<(u32, u32)> = None;
        let mut distinct = 0u32;
        for &(s, c) in &self.syms {
            if excl.contains(s) {
                continue;
            }
            if s == x {
                found = Some((cum, c as u32));
            }
            cum += c as u32;
            distinct += 1;
        }
        if distinct == 0 {
            return CtxCode::Empty;
        }
        let total = cum + escape_weight(distinct);
        match found {
            Some((start, freq)) => CtxCode::Sym(start, freq, total),
            None => CtxCode::Escape(cum, escape_weight(distinct), total),
        }
    }

    /// Total of the non-excluded interval (needed before `sym_at`).
    fn total_excl(&self, excl: &Excl) -> u32 {
        let mut cum = 0u32;
        let mut distinct = 0u32;
        for &(s, c) in &self.syms {
            if excl.contains(s) {
                continue;
            }
            cum += c as u32;
            distinct += 1;
        }
        if distinct == 0 {
            0
        } else {
            cum + escape_weight(distinct)
        }
    }

    /// Locate the symbol containing `target` for decoding (over the
    /// non-excluded interval). `Ok((sym, cum, freq, total))` or
    /// `Err(escape (cum, freq, total))`.
    #[allow(clippy::type_complexity)]
    fn sym_at(
        &self,
        target: u32,
        excl: &Excl,
    ) -> std::result::Result<(u16, u32, u32, u32), (u32, u32, u32)> {
        let total = self.total_excl(excl);
        let mut cum = 0u32;
        let mut hit: Option<(u16, u32, u32)> = None;
        let mut distinct = 0u32;
        for &(s, c) in &self.syms {
            if excl.contains(s) {
                continue;
            }
            if hit.is_none() && target < cum + c as u32 {
                hit = Some((s, cum, c as u32));
            }
            cum += c as u32;
            distinct += 1;
        }
        match hit {
            Some((s, start, freq)) => Ok((s, start, freq, total)),
            None => Err((cum, escape_weight(distinct), total)),
        }
    }

    /// Append this context's non-excluded symbols to the exclusion list.
    fn exclude_into(&self, excl: &mut Excl) {
        for &(s, _) in &self.syms {
            excl.insert(s);
        }
    }

    fn bump(&mut self, x: u16) {
        match self.syms.iter_mut().find(|(s, _)| *s == x) {
            Some((_, c)) => *c += BUMP,
            None => self.syms.push((x, BUMP)),
        }
        self.sum += BUMP as u32;
        if self.total() >= CTX_HALVE_LIMIT {
            self.sum = 0;
            for (_, c) in &mut self.syms {
                *c = (*c / 2).max(1);
                self.sum += *c as u32;
            }
        }
    }
}

/// Sentinel context symbol for "no neighbor" (grid edge / no base).
const EDGE: u16 = 0xFFFF;

/// Per-symbol exclusion set as a generation-stamped array: O(1) insert and
/// membership, O(1) reset per coded symbol (bump the generation).
struct Excl {
    stamp: Vec<u32>,
    generation: u32,
}

impl Excl {
    fn new(alphabet: u16) -> Self {
        Self {
            stamp: vec![0; alphabet as usize],
            generation: 0,
        }
    }

    fn begin(&mut self) {
        self.generation += 1;
        if self.generation == u32::MAX {
            self.stamp.fill(0);
            self.generation = 1;
        }
    }

    #[inline]
    fn contains(&self, s: u16) -> bool {
        self.stamp[s as usize] == self.generation
    }

    #[inline]
    fn insert(&mut self, s: u16) {
        self.stamp[s as usize] = self.generation;
    }
}

struct Model {
    /// Most specific: (primary, second) — (above, left) standalone,
    /// (base tile, above) cross-variant. An order-3 level (adding the
    /// diagonal / the variant's left) was measured a wash: ±1% per
    /// character, extra memory and time — PPMC escape costs cancel the
    /// sharper predictions.
    c2: HashMap<u32, Ctx>,
    /// primary alone (the stronger single predictor).
    c1: HashMap<u16, Ctx>,
    /// second alone.
    c1b: HashMap<u16, Ctx>,
    c0: Ctx,
    alphabet: u32,
    excl: Excl,
}

impl Model {
    fn new(alphabet: u16) -> Self {
        Self {
            c2: HashMap::new(),
            c1: HashMap::new(),
            c1b: HashMap::new(),
            c0: Ctx::default(),
            alphabet: alphabet as u32,
            excl: Excl::new(alphabet),
        }
    }

    fn encode_sym(&mut self, enc: &mut RangeEncoder, primary: u16, second: u16, x: u16) {
        let key2 = ((primary as u32) << 16) | second as u32;
        self.excl.begin();
        let excl = &mut self.excl;
        let mut coded = false;
        for ctx in [
            self.c2.entry(key2).or_default(),
            self.c1.entry(primary).or_default(),
            self.c1b.entry(second).or_default(),
            &mut self.c0,
        ] {
            if !coded {
                match ctx.code_for(x, excl) {
                    CtxCode::Sym(cum, f, t) => {
                        enc.encode(cum, f, t);
                        coded = true;
                    }
                    CtxCode::Escape(cum, f, t) => {
                        enc.encode(cum, f, t);
                        ctx.exclude_into(excl);
                    }
                    CtxCode::Empty => {}
                }
            }
            ctx.bump(x);
        }
        if !coded {
            enc.encode(x as u32, 1, self.alphabet);
        }
    }

    fn decode_sym(&mut self, dec: &mut RangeDecoder, primary: u16, second: u16) -> u16 {
        let key2 = ((primary as u32) << 16) | second as u32;
        self.excl.begin();
        let excl = &mut self.excl;
        let mut decoded: Option<u16> = None;
        // Decode over the chain first (contexts stay immutable; exclusions
        // accumulate in the stamp set), then bump every level.
        {
            let chain: [&Ctx; 4] = [
                self.c2.entry(key2).or_default(),
                self.c1.entry(primary).or_default(),
                self.c1b.entry(second).or_default(),
                &self.c0,
            ];
            for ctx in chain {
                let total = ctx.total_excl(excl);
                if total == 0 {
                    continue;
                }
                let target = dec.decode_target(total);
                match ctx.sym_at(target, excl) {
                    Ok((s, cum, f, t)) => {
                        dec.commit(cum, f, t);
                        decoded = Some(s);
                        break;
                    }
                    Err((cum, f, t)) => {
                        dec.commit(cum, f, t);
                        ctx.exclude_into(excl);
                    }
                }
            }
        }
        let x = match decoded {
            Some(s) => s,
            None => {
                let target = dec.decode_target(self.alphabet);
                dec.commit(target, 1, self.alphabet);
                target as u16
            }
        };
        self.c2.entry(key2).or_default().bump(x);
        self.c1.entry(primary).or_default().bump(x);
        self.c1b.entry(second).or_default().bump(x);
        self.c0.bump(x);
        x
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// One VQ sprite's index grid: `cols = width/4`, `rows = height`,
/// `indices.len() == cols * rows`.
pub struct SpriteGrid<'a> {
    pub cols: u16,
    pub rows: u16,
    pub indices: &'a [u16],
}

/// Encode a sequence of VQ sprite grids. `alphabet` is the dictionary entry
/// count; every index must be `< alphabet`. When `base` is given, `base[i]`
/// (same layout as `grids[i].indices`) provides the cross-variant context —
/// the aligned tile of the family base character; `None` entries code that
/// sprite standalone within the same model.
///
/// Contexts persist across the whole sequence, so grids must be decoded in
/// the same order with [`decode_grids`].
pub fn encode_grids(
    alphabet: u16,
    grids: &[SpriteGrid],
    base: Option<&[Option<&[u16]>]>,
) -> Result<Vec<u8>> {
    if let Some(base) = base {
        if base.len() != grids.len() {
            return Err(anyhow!(
                "base list length {} != grid count {}",
                base.len(),
                grids.len()
            ));
        }
    }
    let mut enc = RangeEncoder::new();
    let mut model = Model::new(alphabet);
    for (gi, g) in grids.iter().enumerate() {
        let cols = g.cols as usize;
        if g.indices.len() != cols * g.rows as usize {
            return Err(anyhow!(
                "grid {gi}: {} indices for {}x{}",
                g.indices.len(),
                g.cols,
                g.rows
            ));
        }
        let b = base.and_then(|b| b[gi]);
        if let Some(b) = b {
            if b.len() != g.indices.len() {
                return Err(anyhow!("grid {gi}: base length mismatch"));
            }
        }
        for (i, &x) in g.indices.iter().enumerate() {
            if x as u32 >= alphabet as u32 {
                return Err(anyhow!("grid {gi}: index {x} >= alphabet {alphabet}"));
            }
            let above = if i >= cols { g.indices[i - cols] } else { EDGE };
            let left = if i % cols > 0 { g.indices[i - 1] } else { EDGE };
            // The chain falls back specific -> primary -> second, so put the
            // stronger single predictor first: the base tile for variants,
            // the above tile standalone (see COMPRESSION.md entropy table).
            let (primary, second) = match b {
                Some(b) => (b[i], above),
                None => (above, left),
            };
            model.encode_sym(&mut enc, primary, second, x);
        }
    }
    Ok(enc.finish())
}

/// Decode grids produced by [`encode_grids`]. `dims[i] = (cols, rows)` and
/// `base` must match the encoding call exactly.
pub fn decode_grids(
    alphabet: u16,
    dims: &[(u16, u16)],
    base: Option<&[Option<&[u16]>]>,
    blob: &[u8],
) -> Result<Vec<Vec<u16>>> {
    if let Some(base) = base {
        if base.len() != dims.len() {
            return Err(anyhow!(
                "base list length {} != grid count {}",
                base.len(),
                dims.len()
            ));
        }
    }
    let mut dec = RangeDecoder::new(blob);
    let mut model = Model::new(alphabet);
    let mut out = Vec::with_capacity(dims.len());
    for (gi, &(cols16, rows)) in dims.iter().enumerate() {
        let cols = cols16 as usize;
        let n = cols * rows as usize;
        let b = base.and_then(|b| b[gi]);
        if let Some(b) = b {
            if b.len() != n {
                return Err(anyhow!("grid {gi}: base length mismatch"));
            }
        }
        let mut g: Vec<u16> = Vec::with_capacity(n);
        for i in 0..n {
            let above = if i >= cols { g[i - cols] } else { EDGE };
            let left = if i % cols > 0 { g[i - 1] } else { EDGE };
            let (primary, second) = match b {
                Some(b) => (b[i], above),
                None => (above, left),
            };
            g.push(model.decode_sym(&mut dec, primary, second));
        }
        out.push(g);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(
        alphabet: u16,
        grids: &[(u16, u16, Vec<u16>)],
        base: Option<Vec<Option<Vec<u16>>>>,
    ) {
        let grid_refs: Vec<SpriteGrid> = grids
            .iter()
            .map(|(c, r, v)| SpriteGrid {
                cols: *c,
                rows: *r,
                indices: v,
            })
            .collect();
        let base_refs: Option<Vec<Option<&[u16]>>> = base
            .as_ref()
            .map(|b| b.iter().map(|o| o.as_deref()).collect());
        let blob = encode_grids(alphabet, &grid_refs, base_refs.as_deref()).unwrap();
        let dims: Vec<(u16, u16)> = grids.iter().map(|(c, r, _)| (*c, *r)).collect();
        let decoded = decode_grids(alphabet, &dims, base_refs.as_deref(), &blob).unwrap();
        for ((_, _, v), d) in grids.iter().zip(decoded.iter()) {
            assert_eq!(v, d);
        }
    }

    #[test]
    fn roundtrip_empty() {
        roundtrip(4096, &[], None);
    }

    #[test]
    fn roundtrip_single_tile() {
        roundtrip(4096, &[(1, 1, vec![1234])], None);
    }

    #[test]
    fn roundtrip_synthetic() {
        // Structured data: vertical stripes with noise, several sprites.
        let mut rng = 12345u64;
        let mut rand = move || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) as u32
        };
        let mut grids = Vec::new();
        for s in 0..17 {
            let cols = 3 + (s % 5) as u16;
            let rows = 4 + (s % 7) as u16;
            let n = cols as usize * rows as usize;
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let col = i % cols as usize;
                let base = ((col * 37 + s as usize * 11) % 4000) as u16;
                let noise = if rand() % 5 == 0 {
                    (rand() % 4096) as u16
                } else {
                    0
                };
                v.push((base ^ noise) % 4096);
            }
            grids.push((cols, rows, v));
        }
        roundtrip(4096, &grids, None);
    }

    #[test]
    fn roundtrip_with_base() {
        // Variant = base with sparse substitutions, plus one unbased sprite.
        let mut rng = 99u64;
        let mut rand = move || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) as u32
        };
        let mut grids = Vec::new();
        let mut bases: Vec<Option<Vec<u16>>> = Vec::new();
        for s in 0..9 {
            let cols = 4u16;
            let rows = 6u16;
            let n = cols as usize * rows as usize;
            let base: Vec<u16> = (0..n).map(|i| ((i * 7 + s * 3) % 512) as u16).collect();
            let variant: Vec<u16> = base
                .iter()
                .map(|&b| {
                    if rand() % 8 == 0 {
                        (b + 1000) % 4096
                    } else {
                        b.wrapping_add(2048) % 4096
                    }
                })
                .collect();
            grids.push((cols, rows, variant));
            bases.push(Some(base));
        }
        grids.push((2, 2, vec![7, 8, 9, 10]));
        bases.push(None);
        roundtrip(4096, &grids, Some(bases));
    }

    #[test]
    fn rejects_out_of_alphabet() {
        let g = [SpriteGrid {
            cols: 1,
            rows: 1,
            indices: &[4096],
        }];
        assert!(encode_grids(4096, &g, None).is_err());
    }
}
