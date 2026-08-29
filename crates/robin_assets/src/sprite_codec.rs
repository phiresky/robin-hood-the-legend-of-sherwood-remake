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
//! Escape mass is estimated adaptively (SEE: learned hit/escape ratios
//! bucketed by chain level, context size, maturity, and top-symbol skew —
//! measured -1..4% over PPMC's fixed distinct-count heuristic), with full
//! exclusion: symbols ruled out by an escape at a more specific level cost
//! no probability mass further down (measured ~-3%). Counts update at every
//! level on each symbol, and each context halves its counts when they
//! saturate, which keeps the model adaptive and the range-coder totals well
//! inside precision.
//!
//! The entropy stage is a carry-aware LZMA-style range coder rather than
//! rANS: rANS emits symbols last-in-first-out, which fights adaptive
//! context models (the decoder must see updates in encode order), while a
//! range coder is FIFO and pairs with adaptation naturally.
//!
//! Measured on `datadirs/fullgame_linux` (see COMPRESSION.md): ~-33% vs
//! zstd-22 standalone, ~3.9x on family variants coded against their base;
//! the whole character corpus comes out 2.27x smaller than zstd-19.

use anyhow::{Result, anyhow};

/// Context maps are on the per-tile hot path; foldhash beats SipHash by a
/// wide margin here and lookup-only use never depends on iteration order.
type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;

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

/// Promote a context to dense counts once it holds this many distinct
/// symbols (only when the alphabet is small enough for an 8 KB flat array).
/// Measured: promotion at 320 REGRESSED decode 6.4s -> 11.8s on Knight01 —
/// the hot contexts are so skewed that the bubbled list answers from its
/// head in ~1 cache line while Fenwick ops touch ~12 cold ones. Disabled
/// (usize::MAX) until someone finds a workload where dense wins; the
/// machinery is kept because exclusion-adjusted Fenwick coding is the known
/// path to O(log) escapes if decode time ever becomes the priority.
const PROMOTE_AT: usize = usize::MAX;
const PROMOTE_MAX_ALPHABET: u32 = 4096;

/// One adaptive context.
///
/// `Small` keeps (symbol, count) pairs frequency-bubbled to the front so hot
/// lookups terminate early. `Big` is a flat per-symbol count array with O(1)
/// updates — its intervals are symbol-ordered rather than frequency-ordered,
/// which changes nothing about arithmetic-coding cost, only the layout, and
/// promotion happens at the same deterministic point on both coder sides.
enum Ctx {
    Small {
        syms: Vec<(u16, u16)>,
        sum: u32,
    },
    Big {
        counts: Box<[u16]>,
        /// Fenwick tree over `counts` (1-based): prefix sums, point updates
        /// and target descent in O(log alphabet).
        tree: Box<[u32]>,
        sum: u32,
        distinct: u32,
    },
}

/// Fenwick prefix sum of `counts[0..x]`.
fn fenwick_prefix(tree: &[u32], x: usize) -> u32 {
    let mut i = x;
    let mut s = 0u32;
    while i > 0 {
        s += tree[i];
        i &= i - 1;
    }
    s
}

fn fenwick_add(tree: &mut [u32], mut i: usize, delta: u32) {
    i += 1;
    while i < tree.len() {
        tree[i] += delta;
        i += i & i.wrapping_neg();
    }
}

/// Binary descent: the symbol `s` with `prefix(s) <= target < prefix(s+1)`,
/// returned as `(s, prefix(s))`.
fn fenwick_descend(tree: &[u32], target: u32) -> (usize, u32) {
    let n = tree.len() - 1;
    let mut step = n.next_power_of_two();
    if step > n {
        step >>= 1;
    }
    let mut idx = 0usize;
    let mut acc = 0u32;
    while step > 0 {
        let next = idx + step;
        if next <= n && acc + tree[next] <= target {
            idx = next;
            acc += tree[next];
        }
        step >>= 1;
    }
    (idx, acc)
}

fn rebuild_fenwick(counts: &[u16]) -> Box<[u32]> {
    let mut tree = vec![0u32; counts.len() + 1].into_boxed_slice();
    for (i, &c) in counts.iter().enumerate() {
        if c > 0 {
            fenwick_add(&mut tree, i, c as u32);
        }
    }
    tree
}

impl Default for Ctx {
    fn default() -> Self {
        Ctx::Small {
            syms: Vec::new(),
            sum: 0,
        }
    }
}

enum CtxCode {
    /// (cum, freq, total, see bucket) of the coded symbol.
    Sym(u32, u32, u32, SeeKey),
    /// (cum, freq, total, see bucket) of the escape.
    Escape(u32, u32, u32, SeeKey),
    /// Context was empty: nothing is coded (probability-1 escape).
    Empty,
}

/// Escape weight for a context with `distinct` non-excluded symbols: PPMC.
/// PPMD-style `distinct.div_ceil(2)` was measured worse net: +1.3..1.9% on
/// standalone characters (the bulk of the coded bytes), -0.3..0.5% on
/// cross-variant streams.
///
/// Used for bookkeeping totals (`Ctx::total` fast paths); actual coding
/// uses [`See`]-adaptive escape frequencies where a `See` table is passed.
#[inline]
fn escape_weight(distinct: u32) -> u32 {
    distinct
}

/// Secondary escape estimation: adaptive escape statistics per
/// (chain level, log2 distinct-symbol count) bucket, replacing PPMC's fixed
/// "escape mass = distinct" heuristic with learned hit/escape ratios.
struct See {
    /// [level][log2 distinct][log2 sum][top-symbol skew quartile]
    /// -> (symbol hits, escapes).
    stats: [[[[(u32, u32); 4]; 15]; 13]; 6],
}

/// A flattened See bucket index.
#[derive(Clone, Copy)]
struct SeeKey(usize, usize, usize, usize);

impl See {
    fn new() -> Self {
        Self {
            stats: [[[[(1, 1); 4]; 15]; 13]; 6],
        }
    }

    #[inline]
    fn key(level: usize, sum: u32, distinct: u32, top: u32) -> SeeKey {
        let d = (31 - (distinct.max(1)).leading_zeros()).min(12) as usize;
        // Context maturity: escape probability falls as observations grow.
        let s = (31 - (sum.max(1)).leading_zeros()).min(14) as usize;
        // How dominant the most frequent symbol is: quartile of top/sum.
        let skew = ((top as u64 * 4) / sum.max(1) as u64).min(3) as usize;
        SeeKey(level, d, s, skew)
    }

    /// Escape frequency for a context whose non-excluded interval has `sum`
    /// total counts. Never 0, and capped so the coder total stays well
    /// inside range precision.
    #[inline]
    fn esc_freq(&self, k: SeeKey, sum: u32) -> u32 {
        let (hits, esc) = self.stats[k.0][k.1][k.2][k.3];
        ((sum as u64 * esc as u64) / hits.max(1) as u64).clamp(1, (4 * sum.max(1)) as u64) as u32
    }

    #[inline]
    fn update(&mut self, k: SeeKey, escaped: bool) {
        let slot = &mut self.stats[k.0][k.1][k.2][k.3];
        if escaped {
            slot.1 += 1;
        } else {
            slot.0 += 1;
        }
        if slot.0 + slot.1 >= 1 << 13 {
            slot.0 = (slot.0 / 2).max(1);
            slot.1 = (slot.1 / 2).max(1);
        }
    }
}

impl Ctx {
    fn sum(&self) -> u32 {
        match self {
            Ctx::Small { sum, .. } | Ctx::Big { sum, .. } => *sum,
        }
    }

    fn distinct(&self) -> u32 {
        match self {
            Ctx::Small { syms, .. } => syms.len() as u32,
            Ctx::Big { distinct, .. } => *distinct,
        }
    }

    fn is_empty(&self) -> bool {
        self.distinct() == 0
    }

    /// Count of the most frequent symbol (bubbling keeps it at the front of
    /// a Small context). Big contexts don't track it; 0 selects the lowest
    /// skew bucket, which only affects See bucketing, not correctness.
    fn top(&self) -> u32 {
        match self {
            Ctx::Small { syms, .. } => syms.first().map(|&(_, c)| c as u32).unwrap_or(0),
            Ctx::Big { .. } => 0,
        }
    }

    fn total(&self) -> u32 {
        self.sum() + escape_weight(self.distinct())
    }

    /// Locate `x` for encoding, ignoring symbols in `excl` (PPM exclusion:
    /// an escape at a more specific level proves the symbol is none of that
    /// level's candidates, so they must not cost probability mass here).
    /// Escape mass comes from the adaptive `see` table for `level`.
    fn code_for(&self, x: u16, excl: &Excl, see: &See, level: usize) -> CtxCode {
        if self.is_empty() {
            return CtxCode::Empty;
        }
        if excl.is_empty() {
            // Fast path: `sum` is tracked, so the scan can stop at `x`.
            let sum = self.sum();
            let key = See::key(level, sum, self.distinct(), self.top());
            let esc = see.esc_freq(key, sum);
            let total = sum + esc;
            let mut cum = 0u32;
            match self {
                Ctx::Small { syms, .. } => {
                    for &(s, c) in syms {
                        if s == x {
                            return CtxCode::Sym(cum, c as u32, total, key);
                        }
                        cum += c as u32;
                    }
                    CtxCode::Escape(sum, esc, total, key)
                }
                Ctx::Big { counts, tree, .. } => {
                    let c = counts[x as usize] as u32;
                    if c > 0 {
                        cum += fenwick_prefix(tree, x as usize);
                        CtxCode::Sym(cum, c, total, key)
                    } else {
                        CtxCode::Escape(sum, esc, total, key)
                    }
                }
            }
        } else {
            let mut cum = 0u32;
            let mut found: Option<(u32, u32)> = None;
            let mut distinct = 0u32;
            let mut visit = |s: u16, c: u32| {
                if excl.contains(s) {
                    return;
                }
                if s == x {
                    found = Some((cum, c));
                }
                cum += c;
                distinct += 1;
            };
            match self {
                Ctx::Small { syms, .. } => {
                    for &(s, c) in syms {
                        visit(s, c as u32);
                    }
                }
                Ctx::Big { counts, .. } => {
                    for (s, &c) in counts.iter().enumerate() {
                        if c > 0 {
                            visit(s as u16, c as u32);
                        }
                    }
                }
            }
            if distinct == 0 {
                return CtxCode::Empty;
            }
            // Bucket by the context's overall top count (not the non-excluded
            // top): an approximation, but identical on both coder sides.
            let key = See::key(level, cum, distinct, self.top());
            let esc = see.esc_freq(key, cum);
            let total = cum + esc;
            match found {
                Some((start, freq)) => CtxCode::Sym(start, freq, total, key),
                None => CtxCode::Escape(cum, esc, total, key),
            }
        }
    }

    /// Find the symbol whose no-exclusion interval contains `target`
    /// (caller has already checked `target < self.sum()`).
    fn find_by_target(&self, target: u32) -> (u16, u32, u32) {
        let mut cum = 0u32;
        match self {
            Ctx::Small { syms, .. } => {
                for &(s, c) in syms {
                    if target < cum + c as u32 {
                        return (s, cum, c as u32);
                    }
                    cum += c as u32;
                }
            }
            Ctx::Big { counts, tree, .. } => {
                let (s, prefix) = fenwick_descend(tree, target);
                return (s as u16, prefix, counts[s] as u32);
            }
        }
        unreachable!("target beyond context sum");
    }

    /// Copy the non-excluded (symbol, count) pairs into `out`.
    /// Returns `(sum, total)` of the reduced interval (0, 0 when empty).
    fn fill_scratch(
        &self,
        excl: &Excl,
        see: &See,
        level: usize,
        out: &mut Vec<(u16, u32)>,
    ) -> (u32, u32, SeeKey) {
        out.clear();
        let mut sum = 0u32;
        let mut push = |s: u16, c: u32, out: &mut Vec<(u16, u32)>| {
            if !excl.contains(s) {
                out.push((s, c));
                sum += c;
            }
        };
        match self {
            Ctx::Small { syms, .. } => {
                for &(s, c) in syms {
                    push(s, c as u32, out);
                }
            }
            Ctx::Big { counts, .. } => {
                for (s, &c) in counts.iter().enumerate() {
                    if c > 0 {
                        push(s as u16, c as u32, out);
                    }
                }
            }
        }
        if out.is_empty() {
            (0, 0, See::key(level, 0, 1, 0))
        } else {
            let key = See::key(level, sum, out.len() as u32, self.top());
            (sum, sum + see.esc_freq(key, sum), key)
        }
    }

    /// Append this context's symbols to the exclusion list.
    fn exclude_into(&self, excl: &mut Excl) {
        match self {
            Ctx::Small { syms, .. } => {
                for &(s, _) in syms {
                    excl.insert(s);
                }
            }
            Ctx::Big { counts, .. } => {
                for (s, &c) in counts.iter().enumerate() {
                    if c > 0 {
                        excl.insert(s as u16);
                    }
                }
            }
        }
    }

    fn bump(&mut self, x: u16, alphabet: u32) {
        match self {
            Ctx::Small { syms, sum } => {
                match syms.iter().position(|&(s, _)| s == x) {
                    Some(mut i) => {
                        syms[i].1 += BUMP;
                        // Bubble toward the front while more frequent than
                        // the predecessor so scans hit hot symbols first.
                        // Deterministic, so both coder sides keep identical
                        // interval layouts.
                        while i > 0 && syms[i].1 > syms[i - 1].1 {
                            syms.swap(i, i - 1);
                            i -= 1;
                        }
                    }
                    None => syms.push((x, BUMP)),
                }
                *sum += BUMP as u32;
                if *sum + escape_weight(syms.len() as u32) >= CTX_HALVE_LIMIT {
                    *sum = 0;
                    for (_, c) in syms.iter_mut() {
                        *c = (*c / 2).max(1);
                        *sum += *c as u32;
                    }
                }
                if syms.len() >= PROMOTE_AT && alphabet <= PROMOTE_MAX_ALPHABET {
                    let mut counts = vec![0u16; alphabet as usize].into_boxed_slice();
                    let mut new_sum = 0u32;
                    for &(s, c) in syms.iter() {
                        counts[s as usize] = c;
                        new_sum += c as u32;
                    }
                    let distinct = syms.len() as u32;
                    let tree = rebuild_fenwick(&counts);
                    *self = Ctx::Big {
                        counts,
                        tree,
                        sum: new_sum,
                        distinct,
                    };
                }
            }
            Ctx::Big {
                counts,
                tree,
                sum,
                distinct,
            } => {
                if counts[x as usize] == 0 {
                    *distinct += 1;
                }
                counts[x as usize] += BUMP;
                fenwick_add(tree, x as usize, BUMP as u32);
                *sum += BUMP as u32;
                if *sum + escape_weight(*distinct) >= CTX_HALVE_LIMIT {
                    *sum = 0;
                    for c in counts.iter_mut() {
                        if *c > 0 {
                            *c = (*c / 2).max(1);
                            *sum += *c as u32;
                        }
                    }
                    *tree = rebuild_fenwick(counts);
                }
            }
        }
    }
}

/// Sentinel context symbol for "no neighbor" (grid edge / no base).
const EDGE: u16 = 0xFFFF;

/// See-statistics slot for the two-predecessor (b1, b2) level. Fixed at 4
/// so the established levels 0..3 keep their indices and single-base
/// bitstreams stay byte-identical.
const SEE_LEVEL_PAIR2: usize = 4;

/// See-statistics slot for the auxiliary-reference (aux, above) level.
const SEE_LEVEL_AUX: usize = 5;

/// Per-symbol exclusion set as a generation-stamped array: O(1) insert and
/// membership, O(1) reset per coded symbol (bump the generation).
struct Excl {
    stamp: Vec<u32>,
    generation: u32,
    inserted: u32,
}

impl Excl {
    fn new(alphabet: u16) -> Self {
        Self {
            stamp: vec![0; alphabet as usize],
            generation: 0,
            inserted: 0,
        }
    }

    fn begin(&mut self) {
        self.generation += 1;
        self.inserted = 0;
        if self.generation == u32::MAX {
            self.stamp.fill(0);
            self.generation = 1;
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.inserted == 0
    }

    #[inline]
    fn contains(&self, s: u16) -> bool {
        self.stamp[s as usize] == self.generation
    }

    #[inline]
    fn insert(&mut self, s: u16) {
        self.stamp[s as usize] = self.generation;
        self.inserted += 1;
    }
}

struct Model {
    /// Most specific: (primary, second) — (above, left) standalone,
    /// (base tile, above) cross-variant. An order-3 level (adding the
    /// diagonal / the variant's left) was measured a wash: ±1% per
    /// character, extra memory and time — PPMC escape costs cancel the
    /// sharper predictions.
    c2: HashMap<u32, Ctx>,
    /// Two-predecessor level for family members with two already-decoded
    /// siblings: (base1-tile, base2-tile). Its See statistics live at index
    /// 4 so the established levels keep their indices (and single-base
    /// bitstreams stay byte-identical).
    c2pair: HashMap<u32, Ctx>,
    /// Auxiliary-reference level for standalone sprites with an aligned
    /// previously-decoded reference (temporal predecessor or adjacent
    /// direction): (aux-tile, above). See slot [`SEE_LEVEL_AUX`].
    c2aux: HashMap<u32, Ctx>,
    /// primary alone (the stronger single predictor).
    c1: Vec<Ctx>,
    /// second alone.
    c1b: Vec<Ctx>,
    c0: Ctx,
    alphabet: u32,
    excl: Excl,
    /// Reusable buffer for the exclusion-aware decode path: one scan copies
    /// the non-excluded (symbol, count) pairs here, and the target search
    /// walks this compact array instead of re-scanning with stamp checks.
    scratch: Vec<(u16, u32)>,
    see: See,
}

impl Model {
    fn new(alphabet: u16) -> Self {
        Self {
            c2: HashMap::default(),
            c2pair: HashMap::default(),
            c2aux: HashMap::default(),
            // Order-1 contexts are direct-indexed by symbol (last slot =
            // EDGE): no hashing on the per-tile hot path.
            c1: (0..=alphabet as usize).map(|_| Ctx::default()).collect(),
            c1b: (0..=alphabet as usize).map(|_| Ctx::default()).collect(),
            c0: Ctx::default(),
            alphabet: alphabet as u32,
            excl: Excl::new(alphabet),
            scratch: Vec::new(),
            see: See::new(),
        }
    }

    fn encode_sym(&mut self, enc: &mut RangeEncoder, primary: u16, second: u16, x: u16) {
        let key2 = ((primary as u32) << 16) | second as u32;
        let alphabet = self.alphabet;
        self.excl.begin();
        let excl = &mut self.excl;
        let see = &mut self.see;
        let mut coded = false;
        for (level, ctx) in [
            self.c2.entry(key2).or_default(),
            &mut self.c1[(primary as usize).min(alphabet as usize)],
            &mut self.c1b[(second as usize).min(alphabet as usize)],
            &mut self.c0,
        ]
        .into_iter()
        .enumerate()
        {
            if !coded {
                match ctx.code_for(x, excl, see, level) {
                    CtxCode::Sym(cum, f, t, key) => {
                        enc.encode(cum, f, t);
                        see.update(key, false);
                        coded = true;
                    }
                    CtxCode::Escape(cum, f, t, key) => {
                        enc.encode(cum, f, t);
                        see.update(key, true);
                        ctx.exclude_into(excl);
                    }
                    CtxCode::Empty => {}
                }
            }
            ctx.bump(x, alphabet);
        }
        if !coded {
            enc.encode(x as u32, 1, self.alphabet);
        }
    }

    /// Two-predecessor chain for sprites with two aligned decoded siblings:
    /// (b1, b2) -> (b1, above) -> b1 -> above -> order-0 -> uniform.
    /// The (b1, above) level shares the map and See slot of the single-base
    /// chain's most specific level (identical semantics), so mixed streams
    /// pool their statistics.
    fn encode_sym3(&mut self, enc: &mut RangeEncoder, b1: u16, b2: u16, above: u16, x: u16) {
        let key_pair = ((b1 as u32) << 16) | b2 as u32;
        let key2 = ((b1 as u32) << 16) | above as u32;
        let alphabet = self.alphabet;
        self.excl.begin();
        let excl = &mut self.excl;
        let see = &mut self.see;
        let mut coded = false;
        for (level, ctx) in [
            (SEE_LEVEL_PAIR2, self.c2pair.entry(key_pair).or_default()),
            (0, self.c2.entry(key2).or_default()),
            (1, &mut self.c1[(b1 as usize).min(alphabet as usize)]),
            (2, &mut self.c1b[(above as usize).min(alphabet as usize)]),
            (3, &mut self.c0),
        ] {
            if !coded {
                match ctx.code_for(x, excl, see, level) {
                    CtxCode::Sym(cum, f, t, key) => {
                        enc.encode(cum, f, t);
                        see.update(key, false);
                        coded = true;
                    }
                    CtxCode::Escape(cum, f, t, key) => {
                        enc.encode(cum, f, t);
                        see.update(key, true);
                        ctx.exclude_into(excl);
                    }
                    CtxCode::Empty => {}
                }
            }
            ctx.bump(x, alphabet);
        }
        if !coded {
            enc.encode(x as u32, 1, self.alphabet);
        }
    }

    /// Standalone chain extended with an auxiliary aligned reference:
    /// (aux, above) -> (above, left) -> above -> left -> order-0 -> uniform.
    /// With `aux == EDGE` the first level is skipped and the stream is the
    /// plain standalone chain.
    fn encode_sym_aux(&mut self, enc: &mut RangeEncoder, aux: u16, above: u16, left: u16, x: u16) {
        let key_aux = ((aux as u32) << 16) | above as u32;
        let key2 = ((above as u32) << 16) | left as u32;
        let alphabet = self.alphabet;
        self.excl.begin();
        let excl = &mut self.excl;
        let see = &mut self.see;
        let mut coded = false;
        let skip_aux = aux == EDGE;
        // Aux level first: unlike the cluster experiment, the aligned
        // reference is strong exactly where it exists (43-68% identity), and
        // ordering it after (above,left) measured worse (Knight01 +2.3%).
        for (skip, level, ctx) in [
            (
                skip_aux,
                SEE_LEVEL_AUX,
                self.c2aux.entry(key_aux).or_default(),
            ),
            (false, 0, self.c2.entry(key2).or_default()),
            (
                false,
                1,
                &mut self.c1[(above as usize).min(alphabet as usize)],
            ),
            (
                false,
                2,
                &mut self.c1b[(left as usize).min(alphabet as usize)],
            ),
            (false, 3, &mut self.c0),
        ] {
            if skip {
                continue;
            }
            if !coded {
                match ctx.code_for(x, excl, see, level) {
                    CtxCode::Sym(cum, f, t, key) => {
                        enc.encode(cum, f, t);
                        see.update(key, false);
                        coded = true;
                    }
                    CtxCode::Escape(cum, f, t, key) => {
                        enc.encode(cum, f, t);
                        see.update(key, true);
                        ctx.exclude_into(excl);
                    }
                    CtxCode::Empty => {}
                }
            }
            ctx.bump(x, alphabet);
        }
        if !coded {
            enc.encode(x as u32, 1, self.alphabet);
        }
    }

    /// Decoder mirror of [`Self::encode_sym_aux`].
    fn decode_sym_aux(&mut self, dec: &mut RangeDecoder, aux: u16, above: u16, left: u16) -> u16 {
        let key_aux = ((aux as u32) << 16) | above as u32;
        let key2 = ((above as u32) << 16) | left as u32;
        self.excl.begin();
        let excl = &mut self.excl;
        let scratch = &mut self.scratch;
        let see = &mut self.see;
        let mut decoded: Option<u16> = None;
        let skip_aux = aux == EDGE;
        {
            let chain: [(bool, usize, &Ctx); 5] = [
                (
                    skip_aux,
                    SEE_LEVEL_AUX,
                    self.c2aux.entry(key_aux).or_default(),
                ),
                (false, 0, self.c2.entry(key2).or_default()),
                (
                    false,
                    1,
                    &self.c1[(above as usize).min(self.alphabet as usize)],
                ),
                (
                    false,
                    2,
                    &self.c1b[(left as usize).min(self.alphabet as usize)],
                ),
                (false, 3, &self.c0),
            ];
            for (skip, level, ctx) in chain {
                if skip {
                    continue;
                }
                if excl.is_empty() {
                    if ctx.is_empty() {
                        continue;
                    }
                    let sum = ctx.sum();
                    let key = See::key(level, sum, ctx.distinct(), ctx.top());
                    let esc = see.esc_freq(key, sum);
                    let total = sum + esc;
                    let target = dec.decode_target(total);
                    if target >= sum {
                        dec.commit(sum, esc, total);
                        see.update(key, true);
                        ctx.exclude_into(excl);
                        continue;
                    }
                    let (s, cum, c) = ctx.find_by_target(target);
                    dec.commit(cum, c, total);
                    see.update(key, false);
                    decoded = Some(s);
                    break;
                }
                let (sum, total, key) = ctx.fill_scratch(excl, see, level, scratch);
                if total == 0 {
                    continue;
                }
                let target = dec.decode_target(total);
                if target >= sum {
                    dec.commit(sum, total - sum, total);
                    see.update(key, true);
                    for &(s, _) in scratch.iter() {
                        excl.insert(s);
                    }
                    continue;
                }
                let mut cum = 0u32;
                for &(s, c) in scratch.iter() {
                    if target < cum + c {
                        dec.commit(cum, c, total);
                        see.update(key, false);
                        decoded = Some(s);
                        break;
                    }
                    cum += c;
                }
                debug_assert!(decoded.is_some());
                break;
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
        let alphabet = self.alphabet;
        if !skip_aux {
            self.c2aux.entry(key_aux).or_default().bump(x, alphabet);
        }
        self.c2.entry(key2).or_default().bump(x, alphabet);
        self.c1[(above as usize).min(self.alphabet as usize)].bump(x, alphabet);
        self.c1b[(left as usize).min(self.alphabet as usize)].bump(x, alphabet);
        self.c0.bump(x, alphabet);
        x
    }

    /// Decoder mirror of [`Self::encode_sym3`].
    fn decode_sym3(&mut self, dec: &mut RangeDecoder, b1: u16, b2: u16, above: u16) -> u16 {
        let key_pair = ((b1 as u32) << 16) | b2 as u32;
        let key2 = ((b1 as u32) << 16) | above as u32;
        self.excl.begin();
        let excl = &mut self.excl;
        let scratch = &mut self.scratch;
        let see = &mut self.see;
        let mut decoded: Option<u16> = None;
        {
            let chain: [(usize, &Ctx); 5] = [
                (SEE_LEVEL_PAIR2, self.c2pair.entry(key_pair).or_default()),
                (0, self.c2.entry(key2).or_default()),
                (1, &self.c1[(b1 as usize).min(self.alphabet as usize)]),
                (2, &self.c1b[(above as usize).min(self.alphabet as usize)]),
                (3, &self.c0),
            ];
            for (level, ctx) in chain {
                if excl.is_empty() {
                    if ctx.is_empty() {
                        continue;
                    }
                    let sum = ctx.sum();
                    let key = See::key(level, sum, ctx.distinct(), ctx.top());
                    let esc = see.esc_freq(key, sum);
                    let total = sum + esc;
                    let target = dec.decode_target(total);
                    if target >= sum {
                        dec.commit(sum, esc, total);
                        see.update(key, true);
                        ctx.exclude_into(excl);
                        continue;
                    }
                    let (s, cum, c) = ctx.find_by_target(target);
                    dec.commit(cum, c, total);
                    see.update(key, false);
                    decoded = Some(s);
                    break;
                }
                let (sum, total, key) = ctx.fill_scratch(excl, see, level, scratch);
                if total == 0 {
                    continue;
                }
                let target = dec.decode_target(total);
                if target >= sum {
                    dec.commit(sum, total - sum, total);
                    see.update(key, true);
                    for &(s, _) in scratch.iter() {
                        excl.insert(s);
                    }
                    continue;
                }
                let mut cum = 0u32;
                for &(s, c) in scratch.iter() {
                    if target < cum + c {
                        dec.commit(cum, c, total);
                        see.update(key, false);
                        decoded = Some(s);
                        break;
                    }
                    cum += c;
                }
                debug_assert!(decoded.is_some());
                break;
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
        let alphabet = self.alphabet;
        self.c2pair.entry(key_pair).or_default().bump(x, alphabet);
        self.c2.entry(key2).or_default().bump(x, alphabet);
        self.c1[(b1 as usize).min(self.alphabet as usize)].bump(x, alphabet);
        self.c1b[(above as usize).min(self.alphabet as usize)].bump(x, alphabet);
        self.c0.bump(x, alphabet);
        x
    }

    fn decode_sym(&mut self, dec: &mut RangeDecoder, primary: u16, second: u16) -> u16 {
        let key2 = ((primary as u32) << 16) | second as u32;
        self.excl.begin();
        let excl = &mut self.excl;
        let scratch = &mut self.scratch;
        let see = &mut self.see;
        let mut decoded: Option<u16> = None;
        // Decode over the chain first (contexts stay immutable; exclusions
        // accumulate in the stamp set), then bump every level.
        {
            let chain: [&Ctx; 4] = [
                self.c2.entry(key2).or_default(),
                &self.c1[(primary as usize).min(self.alphabet as usize)],
                &self.c1b[(second as usize).min(self.alphabet as usize)],
                &self.c0,
            ];
            for (level, ctx) in chain.into_iter().enumerate() {
                if excl.is_empty() {
                    // Fast path: `sum` is tracked and the escape interval
                    // starts at it, so escapes are O(1) and symbol scans
                    // stop at the target (hot symbols sit at the front).
                    if ctx.is_empty() {
                        continue;
                    }
                    let sum = ctx.sum();
                    let key = See::key(level, sum, ctx.distinct(), ctx.top());
                    let esc = see.esc_freq(key, sum);
                    let total = sum + esc;
                    let target = dec.decode_target(total);
                    if target >= sum {
                        dec.commit(sum, esc, total);
                        see.update(key, true);
                        ctx.exclude_into(excl);
                        continue;
                    }
                    let (s, cum, c) = ctx.find_by_target(target);
                    dec.commit(cum, c, total);
                    see.update(key, false);
                    decoded = Some(s);
                    break;
                }
                let (sum, total, key) = ctx.fill_scratch(excl, see, level, scratch);
                if total == 0 {
                    continue;
                }
                let target = dec.decode_target(total);
                if target >= sum {
                    dec.commit(sum, total - sum, total);
                    see.update(key, true);
                    for &(s, _) in scratch.iter() {
                        excl.insert(s);
                    }
                    continue;
                }
                let mut cum = 0u32;
                for &(s, c) in scratch.iter() {
                    if target < cum + c {
                        dec.commit(cum, c, total);
                        see.update(key, false);
                        decoded = Some(s);
                        break;
                    }
                    cum += c;
                }
                debug_assert!(decoded.is_some());
                break;
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
        let alphabet = self.alphabet;
        self.c2.entry(key2).or_default().bump(x, alphabet);
        self.c1[(primary as usize).min(self.alphabet as usize)].bump(x, alphabet);
        self.c1b[(second as usize).min(self.alphabet as usize)].bump(x, alphabet);
        self.c0.bump(x, alphabet);
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
    encode_grids_multi(alphabet, grids, base, None)
}

/// [`encode_grids`] with an optional SECOND predecessor per sprite: family
/// members with two already-decoded siblings code each tile through the
/// richer (b1, b2) -> (b1, above) -> ... chain (measured roughly 2x smaller
/// on third-and-later family members). `base2[i]` requires `base[i]`;
/// single-base and no-base sprites keep their established (byte-identical)
/// chains.
pub fn encode_grids_multi(
    alphabet: u16,
    grids: &[SpriteGrid],
    base: Option<&[Option<&[u16]>]>,
    base2: Option<&[Option<&[u16]>]>,
) -> Result<Vec<u8>> {
    for (label, list) in [("base", base), ("base2", base2)] {
        if let Some(list) = list {
            if list.len() != grids.len() {
                return Err(anyhow!(
                    "{label} list length {} != grid count {}",
                    list.len(),
                    grids.len()
                ));
            }
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
        let b2 = base2.and_then(|b| b[gi]);
        if b2.is_some() && b.is_none() {
            return Err(anyhow!("grid {gi}: base2 without base"));
        }
        for (label, s) in [("base", b), ("base2", b2)] {
            if let Some(s) = s {
                if s.len() != g.indices.len() {
                    return Err(anyhow!("grid {gi}: {label} length mismatch"));
                }
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
            match (b, b2) {
                (Some(b), Some(b2)) => model.encode_sym3(&mut enc, b[i], b2[i], above, x),
                (Some(b), None) => model.encode_sym(&mut enc, b[i], above, x),
                _ => model.encode_sym(&mut enc, above, left, x),
            }
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
    decode_grids_multi(alphabet, dims, base, None, blob)
}

/// An auxiliary aligned reference for one sprite: a previously decoded grid
/// (same dictionary space) plus the tile-space shift mapping this sprite's
/// grid positions into it. Positions falling outside the reference behave
/// as "no reference" for that tile.
pub struct AuxRef<'a> {
    pub indices: &'a [u16],
    pub cols: u16,
    pub rows: u16,
    /// This sprite's tile (col, row) reads the reference at
    /// (col + dtx, row + dy).
    pub dtx: i32,
    pub dy: i32,
}

/// Encode standalone grids with optional per-sprite auxiliary references
/// (temporal predecessor or adjacent camera direction, derived
/// deterministically from shipped script metadata). Chain per tile:
/// (aux, above) -> (above, left) -> above -> left -> order-0 -> uniform.
pub fn encode_grids_auxref(
    alphabet: u16,
    grids: &[SpriteGrid],
    aux: &[Option<AuxRef>],
) -> Result<Vec<u8>> {
    if aux.len() != grids.len() {
        return Err(anyhow!(
            "aux list length {} != grid count {}",
            aux.len(),
            grids.len()
        ));
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
        if let Some(r) = &aux[gi] {
            if r.indices.len() != r.cols as usize * r.rows as usize {
                return Err(anyhow!("grid {gi}: aux reference dims mismatch"));
            }
        }
        for (i, &x) in g.indices.iter().enumerate() {
            if x as u32 >= alphabet as u32 {
                return Err(anyhow!("grid {gi}: index {x} >= alphabet {alphabet}"));
            }
            let above = if i >= cols { g.indices[i - cols] } else { EDGE };
            let left = if i % cols > 0 { g.indices[i - 1] } else { EDGE };
            let a = aux_tile(&aux[gi], i, cols);
            model.encode_sym_aux(&mut enc, a, above, left, x);
        }
    }
    Ok(enc.finish())
}

/// Decoder for [`encode_grids_auxref`]; `aux` must match the encoding call.
pub fn decode_grids_auxref(
    alphabet: u16,
    dims: &[(u16, u16)],
    aux: &[Option<AuxRef>],
    blob: &[u8],
) -> Result<Vec<Vec<u16>>> {
    if aux.len() != dims.len() {
        return Err(anyhow!(
            "aux list length {} != grid count {}",
            aux.len(),
            dims.len()
        ));
    }
    let mut dec = RangeDecoder::new(blob);
    let mut model = Model::new(alphabet);
    let mut out = Vec::with_capacity(dims.len());
    for (gi, &(cols16, rows)) in dims.iter().enumerate() {
        let cols = cols16 as usize;
        let n = cols * rows as usize;
        if let Some(r) = &aux[gi] {
            if r.indices.len() != r.cols as usize * r.rows as usize {
                return Err(anyhow!("grid {gi}: aux reference dims mismatch"));
            }
        }
        let mut g: Vec<u16> = Vec::with_capacity(n);
        for i in 0..n {
            let above = if i >= cols { g[i - cols] } else { EDGE };
            let left = if i % cols > 0 { g[i - 1] } else { EDGE };
            let a = aux_tile(&aux[gi], i, cols);
            g.push(model.decode_sym_aux(&mut dec, a, above, left));
        }
        out.push(g);
    }
    Ok(out)
}

/// The reference tile for grid position `i`, or EDGE when absent.
fn aux_tile(aux: &Option<AuxRef>, i: usize, cols: usize) -> u16 {
    let Some(r) = aux else {
        return EDGE;
    };
    let (col, row) = ((i % cols) as i32, (i / cols) as i32);
    let (pc, pr) = (col + r.dtx, row + r.dy);
    if pc >= 0 && pr >= 0 && pc < r.cols as i32 && pr < r.rows as i32 {
        r.indices[(pr * r.cols as i32 + pc) as usize]
    } else {
        EDGE
    }
}

/// Decoder for [`encode_grids_multi`]; `base`/`base2` must match the
/// encoding call exactly.
pub fn decode_grids_multi(
    alphabet: u16,
    dims: &[(u16, u16)],
    base: Option<&[Option<&[u16]>]>,
    base2: Option<&[Option<&[u16]>]>,
    blob: &[u8],
) -> Result<Vec<Vec<u16>>> {
    for (label, list) in [("base", base), ("base2", base2)] {
        if let Some(list) = list {
            if list.len() != dims.len() {
                return Err(anyhow!(
                    "{label} list length {} != grid count {}",
                    list.len(),
                    dims.len()
                ));
            }
        }
    }
    let mut dec = RangeDecoder::new(blob);
    let mut model = Model::new(alphabet);
    let mut out = Vec::with_capacity(dims.len());
    for (gi, &(cols16, rows)) in dims.iter().enumerate() {
        let cols = cols16 as usize;
        let n = cols * rows as usize;
        let b = base.and_then(|b| b[gi]);
        let b2 = base2.and_then(|b| b[gi]);
        if b2.is_some() && b.is_none() {
            return Err(anyhow!("grid {gi}: base2 without base"));
        }
        for (label, s) in [("base", b), ("base2", b2)] {
            if let Some(s) = s {
                if s.len() != n {
                    return Err(anyhow!("grid {gi}: {label} length mismatch"));
                }
            }
        }
        let mut g: Vec<u16> = Vec::with_capacity(n);
        for i in 0..n {
            let above = if i >= cols { g[i - cols] } else { EDGE };
            let left = if i % cols > 0 { g[i - 1] } else { EDGE };
            let x = match (b, b2) {
                (Some(b), Some(b2)) => model.decode_sym3(&mut dec, b[i], b2[i], above),
                (Some(b), None) => model.decode_sym(&mut dec, b[i], above),
                _ => model.decode_sym(&mut dec, above, left),
            };
            g.push(x);
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
    fn roundtrip_with_two_bases() {
        let mut rng = 7u64;
        let mut rand = move || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) as u32
        };
        let mut grids = Vec::new();
        let mut b1s: Vec<Option<Vec<u16>>> = Vec::new();
        let mut b2s: Vec<Option<Vec<u16>>> = Vec::new();
        for s in 0..7u16 {
            let (cols, rows) = (4u16, 5u16);
            let n = (cols * rows) as usize;
            let b1: Vec<u16> = (0..n)
                .map(|i| ((i * 11 + s as usize * 7) % 900) as u16)
                .collect();
            let b2: Vec<u16> = b1.iter().map(|&v| (v + 300) % 4096).collect();
            let target: Vec<u16> = b1
                .iter()
                .zip(b2.iter())
                .map(|(&a, &b)| {
                    if rand() % 6 == 0 {
                        (a + b) % 4096
                    } else {
                        (a + 600) % 4096
                    }
                })
                .collect();
            grids.push((cols, rows, target));
            b1s.push(Some(b1));
            b2s.push(Some(b2));
        }
        // One sprite with only one base, one with none.
        grids.push((3, 3, (0..9).map(|i| (i * 5) as u16).collect()));
        b1s.push(Some((0..9).map(|i| (i * 5 + 1) as u16).collect()));
        b2s.push(None);
        grids.push((2, 2, vec![9, 8, 7, 6]));
        b1s.push(None);
        b2s.push(None);

        let grid_refs: Vec<SpriteGrid> = grids
            .iter()
            .map(|(c, r, v)| SpriteGrid {
                cols: *c,
                rows: *r,
                indices: v,
            })
            .collect();
        let b1_refs: Vec<Option<&[u16]>> = b1s.iter().map(|o| o.as_deref()).collect();
        let b2_refs: Vec<Option<&[u16]>> = b2s.iter().map(|o| o.as_deref()).collect();
        let blob = encode_grids_multi(4096, &grid_refs, Some(&b1_refs), Some(&b2_refs)).unwrap();
        let dims: Vec<(u16, u16)> = grids.iter().map(|(c, r, _)| (*c, *r)).collect();
        let decoded =
            decode_grids_multi(4096, &dims, Some(&b1_refs), Some(&b2_refs), &blob).unwrap();
        for ((_, _, v), d) in grids.iter().zip(decoded.iter()) {
            assert_eq!(v, d);
        }
        // base2 without base is rejected.
        let bad_b1: Vec<Option<&[u16]>> = vec![None];
        let four = [1u16, 2, 3, 4];
        let bad_b2: Vec<Option<&[u16]>> = vec![Some(&four)];
        let one = [SpriteGrid {
            cols: 2,
            rows: 2,
            indices: &four,
        }];
        assert!(encode_grids_multi(4096, &one, Some(&bad_b1), Some(&bad_b2)).is_err());
    }

    #[test]
    fn roundtrip_with_aux_refs() {
        let mut grids = Vec::new();
        // Reference-like first grid, then grids predicted from it at
        // various shifts, then one without any reference.
        let base: Vec<u16> = (0..48u16).map(|i| (i * 37) % 4096).collect();
        grids.push((6u16, 8u16, base.clone()));
        for s in 0..5u16 {
            let v: Vec<u16> = (0..30)
                .map(|i| base[(i + s as usize) % base.len()])
                .collect();
            grids.push((5, 6, v));
        }
        grids.push((3, 3, (0..9).map(|i| (i * 11) as u16).collect()));
        let grid_refs: Vec<SpriteGrid> = grids
            .iter()
            .map(|(c, r, v)| SpriteGrid {
                cols: *c,
                rows: *r,
                indices: v,
            })
            .collect();
        let aux: Vec<Option<AuxRef>> = grids
            .iter()
            .enumerate()
            .map(|(i, _)| {
                if i >= 1 && i <= 5 {
                    Some(AuxRef {
                        indices: &grids[0].2,
                        cols: 6,
                        rows: 8,
                        dtx: (i as i32) - 3,
                        dy: 1,
                    })
                } else {
                    None
                }
            })
            .collect();
        let blob = encode_grids_auxref(4096, &grid_refs, &aux).unwrap();
        let dims: Vec<(u16, u16)> = grids.iter().map(|(c, r, _)| (*c, *r)).collect();
        let decoded = decode_grids_auxref(4096, &dims, &aux, &blob).unwrap();
        for ((_, _, v), d) in grids.iter().zip(decoded.iter()) {
            assert_eq!(v, d);
        }
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
