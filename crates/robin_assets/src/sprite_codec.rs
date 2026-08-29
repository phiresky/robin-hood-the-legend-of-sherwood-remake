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
//! Hit-vs-escape at each level is coded as one LZMA-style adaptive binary
//! decision (schema v12): an 11-bit probability per SEE bucket (chain level,
//! context size, maturity, top-symbol skew), updated by shift toward the
//! coded outcome. This replaced v11's SEE-priced escape-in-total scheme
//! (learned hit/escape ratios folded into the coding interval) because the
//! binary form needs no division at all — `bound = (range >> 11) * p` — and
//! escape pricing was ~15-20% of decode time; on a hit the symbol is then
//! coded in the context's plain frequency interval (the one remaining
//! division per level). The model also supports full exclusion (symbols
//! ruled out by an escape at a more specific level cost no probability mass
//! further down), though shipping chunks disable it for decode speed
//! (`EXCL_SOURCE_CAP = 0`). Counts update at every level on each symbol,
//! and each context halves its counts when they saturate, which keeps the
//! model adaptive and the range-coder totals well inside precision.
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

/// Adaptive binary probabilities are 11-bit, LZMA style: `p` out of
/// [`PROB_ONE`] is the probability of the `bit == false` outcome.
const PROB_BITS: u32 = 11;
const PROB_ONE: u16 = 1 << PROB_BITS;
const PROB_INIT: u16 = PROB_ONE / 2;
/// Adaptation rate: `p` moves 1/2^SHIFT of the way toward the coded outcome
/// per update. Measured on Knight01/RobinTown/Guard-pair coded bytes
/// (2026-08-29): shift 4 and 5 are within 0.01% of each other (5,419,538 vs
/// 5,420,021 B total), 3 and 6 are +0.5% worse; 5 is kept (the LZMA
/// default). Part of the bitstream contract — changing it is a schema bump.
const PROB_SHIFT: u32 = 5;

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

    /// Encode one adaptive binary decision. `p` is the 11-bit probability of
    /// the `false` outcome and adapts toward what was coded (LZMA shift
    /// update) — multiply-only, no division. Interoperates freely with
    /// [`Self::encode`]: both keep the same `low`/`range` invariants
    /// (`range >= RC_TOP` after every call, carries resolved by
    /// [`Self::shift_low`]).
    #[inline]
    fn encode_bit(&mut self, p: &mut u16, bit: bool) {
        let bound = (self.range >> PROB_BITS) * (*p as u32);
        if !bit {
            self.range = bound;
            *p += (PROB_ONE - *p) >> PROB_SHIFT;
        } else {
            self.low += bound as u64;
            self.range -= bound;
            *p -= *p >> PROB_SHIFT;
        }
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
    /// `range / total` from the preceding [`Self::decode_target`], reused by
    /// [`Self::commit`] (every commit follows a decode_target with the same
    /// total, and the division is the hottest single instruction here).
    last_r: u32,
    input: &'a [u8],
    pos: usize,
}

impl<'a> RangeDecoder<'a> {
    fn new(input: &'a [u8]) -> Self {
        let mut d = Self {
            range: u32::MAX,
            code: 0,
            last_r: 0,
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

    /// Decoder mirror of [`RangeEncoder::encode_bit`]: same probability
    /// state, same update, multiply-only.
    #[inline]
    fn decode_bit(&mut self, p: &mut u16) -> bool {
        let bound = (self.range >> PROB_BITS) * (*p as u32);
        let bit = if self.code < bound {
            self.range = bound;
            *p += (PROB_ONE - *p) >> PROB_SHIFT;
            false
        } else {
            self.code -= bound;
            self.range -= bound;
            *p -= *p >> PROB_SHIFT;
            true
        };
        while self.range < RC_TOP {
            self.code = (self.code << 8) | self.next_byte() as u32;
            self.range <<= 8;
        }
        bit
    }

    /// Returns a value in `[0, total)`; caller finds the symbol whose
    /// interval contains it and confirms with `commit`.
    fn decode_target(&mut self, total: u32) -> u32 {
        let r = self.range / total;
        self.last_r = r;
        (self.code / r).min(total - 1)
    }

    /// Must directly follow the [`Self::decode_target`] call whose `total`
    /// produced the interval being committed.
    fn commit(&mut self, start: u32, size: u32, _total: u32) {
        let r = self.last_r;
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
        /// Lazily-allocated alphabet-sized count mirror, kept in exact sync
        /// with `syms` once the list crosses [`DENSE_MIRROR_AT`] entries.
        /// Pure accelerator for the exclusion path: coding decisions and
        /// interval layout still come from `syms` alone, but
        /// [`Ctx::excl_stats`] can subtract the excluded mass by iterating
        /// the (small) exclusion list with O(1) count lookups instead of
        /// walking the whole symbol list.
        dense: Option<Box<[u16]>>,
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
        i += i.isolate_lowest_one();
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
            dense: None,
        }
    }
}

/// Allocate the [`Ctx::Small`] dense count mirror once the symbol list holds
/// this many entries. Contexts this large are the expensive ones to walk on
/// the exclusion path (order-0 approaches the full alphabet); the mirror
/// costs `2 * alphabet` bytes for each context that crosses the threshold.
const DENSE_MIRROR_AT: usize = 128;

enum CtxCode {
    /// (cum, freq, interval sum, see bucket) of the coded symbol: a hit bit
    /// in the bucket's binary model, then the symbol's plain frequency
    /// interval over `sum`. When `freq == sum` (single candidate) the
    /// interval spans the whole range and is skipped on both coder sides.
    Sym(u32, u32, u32, SeeKey),
    /// See bucket of the escape: one escape bit, nothing further here.
    Escape(SeeKey),
    /// Context was empty: nothing is coded (probability-1 escape).
    Empty,
}

/// Escape weight for a context with `distinct` non-excluded symbols: PPMC.
/// Since schema v12 this is bookkeeping only — it pads the count-halving
/// threshold ([`CTX_HALVE_LIMIT`]) exactly as it did when escapes shared the
/// coding interval, keeping halving points deterministic on both sides.
/// Escape *pricing* is the [`See`] binary model.
#[inline]
fn escape_weight(distinct: u32) -> u32 {
    distinct
}

/// Secondary escape estimation, schema v12 form: one adaptive 11-bit binary
/// probability (hit vs escape) per (chain level, log2 distinct, log2 sum,
/// top-symbol skew quartile) bucket, coded via
/// [`RangeEncoder::encode_bit`] / [`RangeDecoder::decode_bit`]. Replaces the
/// v11 hit/escape counters whose `(sum * esc) / hits` pricing put a 64-bit
/// division (plus the enlarged coding total's division) on every visited
/// level.
struct See {
    /// [level][log2 distinct][log2 sum][top-symbol skew quartile]
    /// -> P(hit) in 11 bits.
    prob: [[[[u16; 4]; 15]; 13]; 6],
}

/// A flattened See bucket index.
#[derive(Clone, Copy)]
struct SeeKey(usize, usize, usize, usize);

impl See {
    fn new() -> Self {
        Self {
            prob: [[[[PROB_INIT; 4]; 15]; 13]; 6],
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

    /// The bucket's adaptive hit-probability slot; the range coder's bit
    /// primitives read and update it in one call, so encoder and decoder
    /// stay in lockstep by construction.
    #[inline]
    fn esc_prob(&mut self, k: SeeKey) -> &mut u16 {
        &mut self.prob[k.0][k.1][k.2][k.3]
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

    /// Locate `x` for encoding, ignoring symbols in `excl` (PPM exclusion:
    /// an escape at a more specific level proves the symbol is none of that
    /// level's candidates, so they must not cost probability mass here).
    /// Hit-vs-escape is priced by the caller through the [`See`] binary
    /// model for `level`; this only reports the bucket and the interval.
    fn code_for(&self, x: u16, excl: &Excl, level: usize) -> CtxCode {
        if self.is_empty() {
            return CtxCode::Empty;
        }
        if excl.is_empty() {
            // Fast path: `sum` is tracked, so the scan can stop at `x`.
            let sum = self.sum();
            let key = See::key(level, sum, self.distinct(), self.top());
            let mut cum = 0u32;
            match self {
                Ctx::Small { syms, .. } => {
                    for &(s, c) in syms {
                        if s == x {
                            return CtxCode::Sym(cum, c as u32, sum, key);
                        }
                        cum += c as u32;
                    }
                    CtxCode::Escape(key)
                }
                Ctx::Big { counts, tree, .. } => {
                    let c = counts[x as usize] as u32;
                    if c > 0 {
                        cum += fenwick_prefix(tree, x as usize);
                        CtxCode::Sym(cum, c, sum, key)
                    } else {
                        CtxCode::Escape(key)
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
            match found {
                Some((start, freq)) => CtxCode::Sym(start, freq, cum, key),
                None => CtxCode::Escape(key),
            }
        }
    }

    /// Find the symbol whose no-exclusion interval contains `target`
    /// (caller has already checked `target < self.sum()`). Also returns the
    /// symbol's list index so the caller can [`Self::bump_at`] it without a
    /// second scan (for `Big`, the index slot carries the symbol itself).
    fn find_by_target(&self, target: u32) -> (usize, u16, u32, u32) {
        let mut cum = 0u32;
        match self {
            Ctx::Small { syms, .. } => {
                for (i, &(s, c)) in syms.iter().enumerate() {
                    if target < cum + c as u32 {
                        return (i, s, cum, c as u32);
                    }
                    cum += c as u32;
                }
            }
            Ctx::Big { counts, tree, .. } => {
                let (s, prefix) = fenwick_descend(tree, target);
                return (s, s as u16, prefix, counts[s] as u32);
            }
        }
        unreachable!("target beyond context sum");
    }

    /// Sum and distinct count of the non-excluded interval (pass 1 of the
    /// exclusion path). Symbols are re-walked by
    /// [`Self::find_by_target_excl`] on a hit instead of being materialized
    /// into a scratch list: the copy's `Vec` pushes were over half the
    /// decode profile, and the escape case (the common one — an exclusion
    /// set exists because a more specific level already escaped) never needs
    /// the individual pairs at all.
    /// Returns `(0, 0, _)` when every present symbol is excluded.
    fn excl_stats(&self, excl: &Excl, level: usize) -> (u32, u32, SeeKey) {
        let mut sum = 0u32;
        let mut distinct = 0u32;
        match self {
            Ctx::Small {
                syms,
                sum: ctx_sum,
                dense,
            } => {
                let ex = excl.list();
                if let Some(dense) = dense
                    && ex.len() * 2 < syms.len()
                {
                    // Subtract the excluded mass via the mirror instead of
                    // walking the whole (large) symbol list. Identical
                    // result by construction: the mirror holds exactly the
                    // counts in `syms`.
                    let mut ex_sum = 0u32;
                    let mut ex_distinct = 0u32;
                    for &s in ex {
                        let c = dense[s as usize] as u32;
                        if c > 0 {
                            ex_sum += c;
                            ex_distinct += 1;
                        }
                    }
                    sum = ctx_sum - ex_sum;
                    distinct = syms.len() as u32 - ex_distinct;
                } else {
                    for &(s, c) in syms {
                        if !excl.contains(s) {
                            sum += c as u32;
                            distinct += 1;
                        }
                    }
                }
            }
            Ctx::Big { counts, .. } => {
                for (s, &c) in counts.iter().enumerate() {
                    if c > 0 && !excl.contains(s as u16) {
                        sum += c as u32;
                        distinct += 1;
                    }
                }
            }
        }
        if distinct == 0 {
            (0, 0, See::key(level, 0, 1, 0))
        } else {
            let key = See::key(level, sum, distinct, self.top());
            (sum, distinct, key)
        }
    }

    /// Find the symbol whose exclusion-reduced interval contains `target`
    /// (caller has already checked `target < sum` from [`Self::excl_stats`]
    /// with the same exclusion set). Index slot as in
    /// [`Self::find_by_target`].
    fn find_by_target_excl(&self, excl: &Excl, target: u32) -> (usize, u16, u32, u32) {
        let mut cum = 0u32;
        match self {
            Ctx::Small { syms, .. } => {
                for (i, &(s, c)) in syms.iter().enumerate() {
                    if excl.contains(s) {
                        continue;
                    }
                    let c = c as u32;
                    if target < cum + c {
                        return (i, s, cum, c);
                    }
                    cum += c;
                }
            }
            Ctx::Big { counts, .. } => {
                for (s, &c) in counts.iter().enumerate() {
                    if c == 0 || excl.contains(s as u16) {
                        continue;
                    }
                    let c = c as u32;
                    if target < cum + c {
                        return (s, s as u16, cum, c);
                    }
                    cum += c;
                }
            }
        }
        unreachable!("target beyond non-excluded sum");
    }

    /// Append this context's symbols to the exclusion list.
    fn exclude_into(&self, excl: &mut Excl) {
        // Capped exclusion: a large escaped context would flood the
        // exclusion set and force full filtered scans at every fallback
        // level for marginal probability-mass savings. Skipping those
        // contexts must match on both coder sides (it does: this is the
        // single exclusion entry point).
        if self.distinct() > excl_source_cap() {
            return;
        }
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
        let small_pos = match self {
            Ctx::Small { syms, .. } => Some(syms.iter().position(|&(s, _)| s == x)),
            Ctx::Big { .. } => None,
        };
        match small_pos {
            Some(Some(i)) => self.bump_at(i, x, alphabet),
            Some(None) => self.push_new(x, alphabet),
            None => self.bump_big(x),
        }
    }

    /// [`Self::bump`] for a symbol whose list index the caller already knows
    /// (the coding level's find returned it) — skips the position scan.
    fn bump_at(&mut self, i: usize, x: u16, alphabet: u32) {
        let _ = alphabet;
        match self {
            Ctx::Small { syms, sum, dense } => {
                debug_assert_eq!(syms[i].0, x);
                let c = syms[i].1 + BUMP;
                syms[i].1 = c;
                // Bubble toward the front past every predecessor with a
                // smaller count so scans hit hot symbols first. The list is
                // non-increasing by count, so the destination is found by
                // binary search and the intervening block shifts right in
                // one rotate — the same final layout the pairwise-swap loop
                // produced (deterministic, both coder sides identical),
                // without walking long equal-count runs one swap at a time.
                let dest = syms[..i].partition_point(|&(_, pc)| pc >= c);
                if dest < i {
                    syms[dest..=i].rotate_right(1);
                }
                if let Some(dense) = dense {
                    dense[x as usize] = c;
                }
                small_settle(syms, sum, dense);
            }
            Ctx::Big { .. } => self.bump_big(x),
        }
    }

    /// [`Self::bump`] for a symbol the caller has proven absent from this
    /// context: the level escaped (or was empty) while NO exclusions were in
    /// force, so the coded escape covered the full symbol set. With
    /// exclusions active, absence from the reduced interval proves nothing
    /// and the general [`Self::bump`] must be used instead.
    // PROMOTE_AT is a tuning knob currently parked at usize::MAX (promotion
    // disabled), which makes the threshold comparison trivially false.
    #[allow(clippy::absurd_extreme_comparisons)]
    fn push_new(&mut self, x: u16, alphabet: u32) {
        match self {
            Ctx::Small { syms, sum, dense } => {
                debug_assert!(syms.iter().all(|&(s, _)| s != x));
                syms.push((x, BUMP));
                if let Some(dense) = dense {
                    dense[x as usize] = BUMP;
                } else if excl_source_cap() > 0 && syms.len() >= DENSE_MIRROR_AT {
                    let mut mirror = vec![0u16; alphabet as usize].into_boxed_slice();
                    for &(s, c) in syms.iter() {
                        mirror[s as usize] = c;
                    }
                    *dense = Some(mirror);
                }
                small_settle(syms, sum, dense);
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
            Ctx::Big { .. } => self.bump_big(x),
        }
    }

    fn bump_big(&mut self, x: u16) {
        match self {
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
            Ctx::Small { .. } => unreachable!("bump_big on a Small context"),
        }
    }
}

/// One decoder chain level's outcome.
enum LevelCode {
    /// The level coded the symbol: (list index, symbol) from the find, so
    /// the bump can go straight to [`Ctx::bump_at`].
    Hit(usize, u16),
    /// The level coded an escape, was empty, or had its whole interval
    /// excluded. In every miss case the symbol is PROVEN absent from this
    /// context, so the learning pass uses [`Ctx::push_new`]: the exclusion
    /// set can never contain the coded symbol (inductively — the first
    /// escape happens with no exclusions and proves absence outright, so
    /// the symbols it excludes are all non-matches, and so on down), which
    /// makes an escape over the reduced interval a proof of full absence
    /// too.
    Miss,
}

/// Decode one chain level: pull the adaptive hit/escape bit, then either
/// resolve the symbol from the context's plain frequency interval or record
/// the escape (feeding the exclusion set). Shared by all three decoder
/// chains; mirrors `Ctx::code_for` + the encode loops exactly.
fn decode_level(
    ctx: &Ctx,
    level: usize,
    dec: &mut RangeDecoder,
    see: &mut See,
    excl: &mut Excl,
) -> LevelCode {
    if excl.is_empty() {
        // Fast path: the escape decision is one adaptive bit (no division),
        // and on a hit the symbol scan stops at the target (hot symbols sit
        // at the front).
        if ctx.is_empty() {
            return LevelCode::Miss;
        }
        let sum = ctx.sum();
        let distinct = ctx.distinct();
        let key = See::key(level, sum, distinct, ctx.top());
        if dec.decode_bit(see.esc_prob(key)) {
            ctx.exclude_into(excl);
            return LevelCode::Miss;
        }
        // Hit. A single-candidate context codes no interval at all (it
        // would span the whole range; the encoder skips it identically via
        // the `freq == sum` check), so the division disappears too.
        let (i, s, _, _) = if distinct == 1 {
            ctx.find_by_target(0)
        } else {
            let target = dec.decode_target(sum);
            let f = ctx.find_by_target(target);
            dec.commit(f.2, f.3, sum);
            f
        };
        LevelCode::Hit(i, s)
    } else {
        let (sum, distinct, key) = ctx.excl_stats(excl, level);
        if distinct == 0 {
            return LevelCode::Miss;
        }
        if dec.decode_bit(see.esc_prob(key)) {
            ctx.exclude_into(excl);
            return LevelCode::Miss;
        }
        let (i, s, _, _) = if distinct == 1 {
            ctx.find_by_target_excl(excl, 0)
        } else {
            let target = dec.decode_target(sum);
            let f = ctx.find_by_target_excl(excl, target);
            dec.commit(f.2, f.3, sum);
            f
        };
        LevelCode::Hit(i, s)
    }
}

/// Shared tail of every `Small` bump: account the increment and halve the
/// counts at the adaptation limit (with the dense mirror refreshed to
/// match — symbols never leave the list, so refilling from `syms` covers
/// every nonzero mirror slot).
fn small_settle(syms: &mut Vec<(u16, u16)>, sum: &mut u32, dense: &mut Option<Box<[u16]>>) {
    *sum += BUMP as u32;
    if *sum + escape_weight(syms.len() as u32) >= CTX_HALVE_LIMIT {
        *sum = 0;
        for (_, c) in syms.iter_mut() {
            *c = (*c / 2).max(1);
            *sum += *c as u32;
        }
        if let Some(dense) = dense {
            for &(s, c) in syms.iter() {
                dense[s as usize] = c;
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

/// Escaped contexts holding more than this many distinct symbols do not
/// contribute to the exclusion set (see [`Ctx::exclude_into`]). Part of the
/// bitstream contract: both coder sides must share the value, so changing
/// it is a chunk-schema version change (RHMISN06 and RHMISN07 ship with 0).
/// Measured
/// trade vs uncapped exclusion (Knight01/RobinTown, size, decode time):
/// 256 -> +0.4/+0.6%, -10..15%; 128 -> +0.8/+1.2%, -14..24%;
/// 64 -> +1.1/+1.7%, -16..30%.
///
/// 2026-08-29, re-measured against cap 256 after the decode-path rework
/// (no scratch materialization, dense mirrors): 0 (exclusion disabled)
/// costs +1.1% (Knight01) / +1.4% (RobinTown) / +2.7% (Guard A02 variant
/// stream) and cuts decode a further 35-43% — every level stays on the
/// tracked-sum fast path and the filtered rescans disappear entirely.
/// Speed was declared the priority over ~1.5 MB of fullgame rhs bucket,
/// so exclusion is off.
const EXCL_SOURCE_CAP: u32 = 0;

/// TEMPORARY experiment override for [`EXCL_SOURCE_CAP`] via the
/// `ROBIN_EXCL_CAP` env var, to measure the size/decode-time ladder without
/// rebuilding per value. Bitstream contract still applies: encode and decode
/// must run with the same value. TODO: bake the chosen value back into the
/// const and delete this before shipping chunks encoded with it.
fn excl_source_cap() -> u32 {
    static CAP: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("ROBIN_EXCL_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(EXCL_SOURCE_CAP)
    })
}

/// Per-symbol exclusion set as a generation-stamped array: O(1) insert and
/// membership, O(1) reset per coded symbol (bump the generation). The
/// deduplicated insertion order is also kept as a list so scans can iterate
/// the (usually much smaller) exclusion side instead of a context's symbols.
struct Excl {
    stamp: Vec<u32>,
    generation: u32,
    list: Vec<u16>,
}

impl Excl {
    fn new(alphabet: u16) -> Self {
        Self {
            stamp: vec![0; alphabet as usize],
            generation: 0,
            list: Vec::new(),
        }
    }

    fn begin(&mut self) {
        self.generation += 1;
        self.list.clear();
        if self.generation == u32::MAX {
            self.stamp.fill(0);
            self.generation = 1;
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    #[inline]
    fn contains(&self, s: u16) -> bool {
        self.stamp[s as usize] == self.generation
    }

    #[inline]
    fn insert(&mut self, s: u16) {
        if self.stamp[s as usize] != self.generation {
            self.stamp[s as usize] = self.generation;
            self.list.push(s);
        }
    }

    #[inline]
    fn list(&self) -> &[u16] {
        &self.list
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
    see: See,
}

impl Model {
    fn new(alphabet: u16) -> Self {
        Self {
            // Pre-size the context maps: models routinely end with tens of
            // thousands of order-2 contexts, and growing there from empty
            // shows up as rehash churn in decode profiles.
            c2: HashMap::with_capacity_and_hasher(1 << 15, Default::default()),
            c2pair: HashMap::with_capacity_and_hasher(1 << 14, Default::default()),
            c2aux: HashMap::with_capacity_and_hasher(1 << 14, Default::default()),
            // Order-1 contexts are direct-indexed by symbol (last slot =
            // EDGE): no hashing on the per-tile hot path.
            c1: (0..=alphabet as usize).map(|_| Ctx::default()).collect(),
            c1b: (0..=alphabet as usize).map(|_| Ctx::default()).collect(),
            c0: Ctx::default(),
            alphabet: alphabet as u32,
            excl: Excl::new(alphabet),
            see: See::new(),
        }
    }

    fn encode_sym(&mut self, enc: &mut RangeEncoder, primary: u16, second: u16, x: u16) {
        let key2 = ((primary as u32) << 16) | second as u32;
        let alphabet = self.alphabet;
        self.excl.begin();
        let excl = &mut self.excl;
        let see = &mut self.see;
        // Update exclusion: only the levels actually visited (escaped ones
        // plus the coding level) learn the symbol. Lower levels specialize
        // to the escape material that actually reaches them, and the update
        // cost drops from four context touches per tile to ~1.3.
        for (level, ctx) in [
            self.c2.entry(key2).or_default(),
            &mut self.c1[(primary as usize).min(alphabet as usize)],
            &mut self.c1b[(second as usize).min(alphabet as usize)],
            &mut self.c0,
        ]
        .into_iter()
        .enumerate()
        {
            match ctx.code_for(x, excl, level) {
                CtxCode::Sym(cum, f, sum, key) => {
                    enc.encode_bit(see.esc_prob(key), false);
                    if f != sum {
                        enc.encode(cum, f, sum);
                    }
                    ctx.bump(x, alphabet);
                    return;
                }
                CtxCode::Escape(key) => {
                    enc.encode_bit(see.esc_prob(key), true);
                    ctx.exclude_into(excl);
                }
                CtxCode::Empty => {}
            }
            ctx.bump(x, alphabet);
        }
        enc.encode(x as u32, 1, self.alphabet);
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
        for (level, ctx) in [
            (SEE_LEVEL_PAIR2, self.c2pair.entry(key_pair).or_default()),
            (0, self.c2.entry(key2).or_default()),
            (1, &mut self.c1[(b1 as usize).min(alphabet as usize)]),
            (2, &mut self.c1b[(above as usize).min(alphabet as usize)]),
            (3, &mut self.c0),
        ] {
            match ctx.code_for(x, excl, level) {
                CtxCode::Sym(cum, f, sum, key) => {
                    enc.encode_bit(see.esc_prob(key), false);
                    if f != sum {
                        enc.encode(cum, f, sum);
                    }
                    ctx.bump(x, alphabet);
                    return;
                }
                CtxCode::Escape(key) => {
                    enc.encode_bit(see.esc_prob(key), true);
                    ctx.exclude_into(excl);
                }
                CtxCode::Empty => {}
            }
            ctx.bump(x, alphabet);
        }
        enc.encode(x as u32, 1, self.alphabet);
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
            match ctx.code_for(x, excl, level) {
                CtxCode::Sym(cum, f, sum, key) => {
                    enc.encode_bit(see.esc_prob(key), false);
                    if f != sum {
                        enc.encode(cum, f, sum);
                    }
                    ctx.bump(x, alphabet);
                    return;
                }
                CtxCode::Escape(key) => {
                    enc.encode_bit(see.esc_prob(key), true);
                    ctx.exclude_into(excl);
                }
                CtxCode::Empty => {}
            }
            ctx.bump(x, alphabet);
        }
        enc.encode(x as u32, 1, self.alphabet);
    }

    /// Decoder mirror of [`Self::encode_sym_aux`].
    fn decode_sym_aux(&mut self, dec: &mut RangeDecoder, aux: u16, above: u16, left: u16) -> u16 {
        let key_aux = ((aux as u32) << 16) | above as u32;
        let key2 = ((above as u32) << 16) | left as u32;
        self.excl.begin();
        let excl = &mut self.excl;
        let see = &mut self.see;
        let skip_aux = aux == EDGE;
        let alphabet = self.alphabet;
        // The aux level runs first and separately (its context entry is not
        // even allocated for EDGE tiles); the remainder is the plain
        // standalone chain.
        let mut aux_ctx: Option<&mut Ctx> = if skip_aux {
            None
        } else {
            Some(self.c2aux.entry(key_aux).or_default())
        };
        if let Some(ctx) = aux_ctx.as_deref_mut()
            && let LevelCode::Hit(i, s) = decode_level(ctx, SEE_LEVEL_AUX, dec, see, excl)
        {
            // Hot exit — 43-68% of aux tiles resolve here; the rest of the
            // chain (including its hash entry) is never touched.
            ctx.bump_at(i, s, alphabet);
            return s;
        }
        let mut chain: [&mut Ctx; 4] = [
            self.c2.entry(key2).or_default(),
            &mut self.c1[(above as usize).min(alphabet as usize)],
            &mut self.c1b[(left as usize).min(alphabet as usize)],
            &mut self.c0,
        ];
        let mut hit: Option<(usize, usize, u16)> = None;
        for level in 0..chain.len() {
            if let LevelCode::Hit(i, s) = decode_level(&*chain[level], level, dec, see, excl) {
                hit = Some((level, i, s));
                break;
            }
        }
        let (coded_at, x) = match hit {
            Some((level, i, s)) => {
                chain[level].bump_at(i, s, alphabet);
                (level, s)
            }
            None => {
                let target = dec.decode_target(alphabet);
                dec.commit(target, 1, alphabet);
                (chain.len(), target as u16)
            }
        };
        if let Some(ctx) = aux_ctx {
            ctx.push_new(x, alphabet);
        }
        for ctx in chain.iter_mut().take(coded_at) {
            ctx.push_new(x, alphabet);
        }
        x
    }

    /// Decoder mirror of [`Self::encode_sym3`].
    fn decode_sym3(&mut self, dec: &mut RangeDecoder, b1: u16, b2: u16, above: u16) -> u16 {
        let key_pair = ((b1 as u32) << 16) | b2 as u32;
        let key2 = ((b1 as u32) << 16) | above as u32;
        let alphabet = self.alphabet;
        self.excl.begin();
        let excl = &mut self.excl;
        let see = &mut self.see;
        let pair_ctx = self.c2pair.entry(key_pair).or_default();
        if let LevelCode::Hit(i, s) = decode_level(pair_ctx, SEE_LEVEL_PAIR2, dec, see, excl) {
            // Hot exit: the (b1, b2) level resolves most pair-coded tiles
            // without touching the rest of the chain or its hash entry.
            pair_ctx.bump_at(i, s, alphabet);
            return s;
        }
        let mut chain: [&mut Ctx; 4] = [
            self.c2.entry(key2).or_default(),
            &mut self.c1[(b1 as usize).min(alphabet as usize)],
            &mut self.c1b[(above as usize).min(alphabet as usize)],
            &mut self.c0,
        ];
        let mut hit: Option<(usize, usize, u16)> = None;
        for level in 0..chain.len() {
            if let LevelCode::Hit(i, s) = decode_level(&*chain[level], level, dec, see, excl) {
                hit = Some((level, i, s));
                break;
            }
        }
        let (coded_at, x) = match hit {
            Some((level, i, s)) => {
                chain[level].bump_at(i, s, alphabet);
                (level, s)
            }
            None => {
                let target = dec.decode_target(alphabet);
                dec.commit(target, 1, alphabet);
                (chain.len(), target as u16)
            }
        };
        pair_ctx.push_new(x, alphabet);
        for ctx in chain.iter_mut().take(coded_at) {
            ctx.push_new(x, alphabet);
        }
        x
    }

    fn decode_sym(&mut self, dec: &mut RangeDecoder, primary: u16, second: u16) -> u16 {
        let key2 = ((primary as u32) << 16) | second as u32;
        let alphabet = self.alphabet;
        self.excl.begin();
        let excl = &mut self.excl;
        let see = &mut self.see;
        // Decode over the chain first (contexts stay unmodified; exclusions
        // accumulate in the stamp set), then learn on the visited levels
        // through the same references (update exclusion: levels below the
        // coding level never see the symbol — mirrors encode_sym exactly).
        // Every level's outcome is known: the hit carries its list index,
        // and a miss proves absence (see [`LevelCode`]), so no learning
        // step ever rescans a symbol list.
        let mut chain: [&mut Ctx; 4] = [
            self.c2.entry(key2).or_default(),
            &mut self.c1[(primary as usize).min(alphabet as usize)],
            &mut self.c1b[(second as usize).min(alphabet as usize)],
            &mut self.c0,
        ];
        let mut hit: Option<(usize, usize, u16)> = None;
        for level in 0..chain.len() {
            if let LevelCode::Hit(i, s) = decode_level(&*chain[level], level, dec, see, excl) {
                hit = Some((level, i, s));
                break;
            }
        }
        let (coded_at, x) = match hit {
            Some((level, i, s)) => {
                chain[level].bump_at(i, s, alphabet);
                (level, s)
            }
            None => {
                let target = dec.decode_target(alphabet);
                dec.commit(target, 1, alphabet);
                (chain.len(), target as u16)
            }
        };
        for ctx in chain.iter_mut().take(coded_at) {
            ctx.push_new(x, alphabet);
        }
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
        if let Some(list) = list
            && list.len() != grids.len()
        {
            return Err(anyhow!(
                "{label} list length {} != grid count {}",
                list.len(),
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
        let b2 = base2.and_then(|b| b[gi]);
        if b2.is_some() && b.is_none() {
            return Err(anyhow!("grid {gi}: base2 without base"));
        }
        for (label, s) in [("base", b), ("base2", b2)] {
            if let Some(s) = s
                && s.len() != g.indices.len()
            {
                return Err(anyhow!("grid {gi}: {label} length mismatch"));
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
        if let Some(r) = &aux[gi]
            && r.indices.len() != r.cols as usize * r.rows as usize
        {
            return Err(anyhow!("grid {gi}: aux reference dims mismatch"));
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
        if let Some(r) = &aux[gi]
            && r.indices.len() != r.cols as usize * r.rows as usize
        {
            return Err(anyhow!("grid {gi}: aux reference dims mismatch"));
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

/// A within-batch auxiliary reference: grid `i` reads its aux context from
/// batch grid `grid` (which must satisfy `grid < i`, so the decoder has
/// already produced it), shifted by `(dtx, dy)` in tile space. Derived
/// deterministically from shipped script metadata on both coder sides, so
/// it costs no format bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfRef {
    pub grid: u32,
    pub dtx: i32,
    pub dy: i32,
}

/// The full shipping combination: family bases (cross-chunk) for variant
/// sprites plus within-batch aux references for standalone sprites. A grid
/// may use bases or a self-reference, not both.
pub fn encode_grids_shipping(
    alphabet: u16,
    grids: &[SpriteGrid],
    base: Option<&[Option<&[u16]>]>,
    base2: Option<&[Option<&[u16]>]>,
    selfref: &[Option<SelfRef>],
) -> Result<Vec<u8>> {
    if selfref.len() != grids.len() {
        return Err(anyhow!(
            "selfref list length {} != grid count {}",
            selfref.len(),
            grids.len()
        ));
    }
    for (label, list) in [("base", base), ("base2", base2)] {
        if let Some(list) = list
            && list.len() != grids.len()
        {
            return Err(anyhow!(
                "{label} list length {} != grid count {}",
                list.len(),
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
        let b2 = base2.and_then(|b| b[gi]);
        if b2.is_some() && b.is_none() {
            return Err(anyhow!("grid {gi}: base2 without base"));
        }
        for (label, s) in [("base", b), ("base2", b2)] {
            if let Some(s) = s
                && s.len() != g.indices.len()
            {
                return Err(anyhow!("grid {gi}: {label} length mismatch"));
            }
        }
        let aux = match &selfref[gi] {
            Some(r) if b.is_some() => {
                return Err(anyhow!(
                    "grid {gi}: self-reference combined with base ({r:?})"
                ));
            }
            Some(r) => {
                if r.grid as usize >= gi {
                    return Err(anyhow!(
                        "grid {gi}: self-reference to grid {} is not causal",
                        r.grid
                    ));
                }
                let t = &grids[r.grid as usize];
                Some(AuxRef {
                    indices: t.indices,
                    cols: t.cols,
                    rows: t.rows,
                    dtx: r.dtx,
                    dy: r.dy,
                })
            }
            None => None,
        };
        for (i, &x) in g.indices.iter().enumerate() {
            if x as u32 >= alphabet as u32 {
                return Err(anyhow!("grid {gi}: index {x} >= alphabet {alphabet}"));
            }
            let above = if i >= cols { g.indices[i - cols] } else { EDGE };
            let left = if i % cols > 0 { g.indices[i - 1] } else { EDGE };
            match (b, b2) {
                (Some(b), Some(b2)) => model.encode_sym3(&mut enc, b[i], b2[i], above, x),
                (Some(b), None) => model.encode_sym(&mut enc, b[i], above, x),
                _ => {
                    let a = aux_tile(&aux, i, cols);
                    model.encode_sym_aux(&mut enc, a, above, left, x);
                }
            }
        }
    }
    Ok(enc.finish())
}

/// Decoder for [`encode_grids_shipping`]; every input must match the
/// encoding call exactly, with self-references resolved against the
/// decoder's own earlier output grids.
pub fn decode_grids_shipping(
    alphabet: u16,
    dims: &[(u16, u16)],
    base: Option<&[Option<&[u16]>]>,
    base2: Option<&[Option<&[u16]>]>,
    selfref: &[Option<SelfRef>],
    blob: &[u8],
) -> Result<Vec<Vec<u16>>> {
    if selfref.len() != dims.len() {
        return Err(anyhow!(
            "selfref list length {} != grid count {}",
            selfref.len(),
            dims.len()
        ));
    }
    for (label, list) in [("base", base), ("base2", base2)] {
        if let Some(list) = list
            && list.len() != dims.len()
        {
            return Err(anyhow!(
                "{label} list length {} != grid count {}",
                list.len(),
                dims.len()
            ));
        }
    }
    let mut dec = RangeDecoder::new(blob);
    let mut model = Model::new(alphabet);
    let mut out: Vec<Vec<u16>> = Vec::with_capacity(dims.len());
    for (gi, &(cols16, rows)) in dims.iter().enumerate() {
        let cols = cols16 as usize;
        let n = cols * rows as usize;
        let b = base.and_then(|b| b[gi]);
        let b2 = base2.and_then(|b| b[gi]);
        if b2.is_some() && b.is_none() {
            return Err(anyhow!("grid {gi}: base2 without base"));
        }
        for (label, s) in [("base", b), ("base2", b2)] {
            if let Some(s) = s
                && s.len() != n
            {
                return Err(anyhow!("grid {gi}: {label} length mismatch"));
            }
        }
        let aux = match &selfref[gi] {
            Some(r) if b.is_some() => {
                return Err(anyhow!(
                    "grid {gi}: self-reference combined with base ({r:?})"
                ));
            }
            Some(r) => {
                if r.grid as usize >= gi {
                    return Err(anyhow!(
                        "grid {gi}: self-reference to grid {} is not causal",
                        r.grid
                    ));
                }
                let (tc, tr) = dims[r.grid as usize];
                Some((r.grid as usize, tc, tr, r.dtx, r.dy))
            }
            None => None,
        };
        let mut g: Vec<u16> = Vec::with_capacity(n);
        for i in 0..n {
            let above = if i >= cols { g[i - cols] } else { EDGE };
            let left = if i % cols > 0 { g[i - 1] } else { EDGE };
            let x = match (b, b2) {
                (Some(b), Some(b2)) => model.decode_sym3(&mut dec, b[i], b2[i], above),
                (Some(b), None) => model.decode_sym(&mut dec, b[i], above),
                _ => {
                    let a = match aux {
                        Some((tg, tc, tr, dtx, dy)) => {
                            let r = AuxRef {
                                indices: &out[tg],
                                cols: tc,
                                rows: tr,
                                dtx,
                                dy,
                            };
                            aux_tile(&Some(r), i, cols)
                        }
                        None => EDGE,
                    };
                    model.decode_sym_aux(&mut dec, a, above, left)
                }
            };
            g.push(x);
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
        if let Some(list) = list
            && list.len() != dims.len()
        {
            return Err(anyhow!(
                "{label} list length {} != grid count {}",
                list.len(),
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
        let b2 = base2.and_then(|b| b[gi]);
        if b2.is_some() && b.is_none() {
            return Err(anyhow!("grid {gi}: base2 without base"));
        }
        for (label, s) in [("base", b), ("base2", b2)] {
            if let Some(s) = s
                && s.len() != n
            {
                return Err(anyhow!("grid {gi}: {label} length mismatch"));
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
                if (1..=5).contains(&i) {
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
    fn roundtrip_shipping_with_self_refs_and_bases() {
        // Batch: grid0 standalone, grid1 self-refs grid0, grid2 self-refs
        // grid1 with a shift, grid3 coded against an external base, grid4
        // against two bases.
        let g0: Vec<u16> = (0..40u16).map(|i| (i * 29) % 4096).collect();
        let g1: Vec<u16> = g0
            .iter()
            .map(|&v| if v % 5 == 0 { v } else { (v + 7) % 4096 })
            .collect();
        let g2: Vec<u16> = (0..30u16)
            .map(|i| g1[(i as usize + 3) % g1.len()])
            .collect();
        let ext_base: Vec<u16> = (0..24u16).map(|i| (i * 13) % 4096).collect();
        let ext_base2: Vec<u16> = ext_base.iter().map(|&v| (v + 100) % 4096).collect();
        let g3: Vec<u16> = ext_base.iter().map(|&v| (v + 5) % 4096).collect();
        let g4: Vec<u16> = ext_base
            .iter()
            .zip(&ext_base2)
            .map(|(&a, &b)| (a + b) % 4096)
            .collect();
        let grids = [
            SpriteGrid {
                cols: 5,
                rows: 8,
                indices: &g0,
            },
            SpriteGrid {
                cols: 5,
                rows: 8,
                indices: &g1,
            },
            SpriteGrid {
                cols: 5,
                rows: 6,
                indices: &g2,
            },
            SpriteGrid {
                cols: 4,
                rows: 6,
                indices: &g3,
            },
            SpriteGrid {
                cols: 4,
                rows: 6,
                indices: &g4,
            },
        ];
        let base: Vec<Option<&[u16]>> = vec![None, None, None, Some(&ext_base), Some(&ext_base)];
        let base2: Vec<Option<&[u16]>> = vec![None, None, None, None, Some(&ext_base2)];
        let selfref = vec![
            None,
            Some(SelfRef {
                grid: 0,
                dtx: 0,
                dy: 0,
            }),
            Some(SelfRef {
                grid: 1,
                dtx: 1,
                dy: -1,
            }),
            None,
            None,
        ];
        let blob =
            encode_grids_shipping(4096, &grids, Some(&base), Some(&base2), &selfref).unwrap();
        let dims: Vec<(u16, u16)> = grids.iter().map(|g| (g.cols, g.rows)).collect();
        let decoded =
            decode_grids_shipping(4096, &dims, Some(&base), Some(&base2), &selfref, &blob).unwrap();
        for (g, d) in grids.iter().zip(decoded.iter()) {
            assert_eq!(g.indices, d.as_slice());
        }
        // Non-causal self-reference is rejected.
        let bad = vec![
            Some(SelfRef {
                grid: 0,
                dtx: 0,
                dy: 0
            });
            1
        ];
        let one = [SpriteGrid {
            cols: 2,
            rows: 2,
            indices: &g0[..4],
        }];
        assert!(encode_grids_shipping(4096, &one, None, None, &bad).is_err());
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
