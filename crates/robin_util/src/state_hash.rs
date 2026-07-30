//! Deterministic hashing of sim state, separate from `std::hash::Hash`.
//!
//! The rollback checker hashes the entire `Engine` after each tick to
//! detect determinism bugs. We can't use `std::hash::Hash` directly:
//!
//! * `f32` / `f64` don't implement `Hash` (NaN equality).
//! * `HashMap` iteration order isn't deterministic.
//!
//! `StateHash` works around both by being a separate trait we control:
//! float impls go through `to_bits()`, and `HashMap` impls hash a
//! sorted view of the entries. The `#[derive(StateHash)]` macro in
//! `robin_state_hash_derive` walks structs/enums field-by-field and
//! writes an explicit marker for fields omitted from the snapshot.

use std::hash::Hasher;

/// A type that can feed its byte-level state into a deterministic
/// `Hasher`. The contract: equivalent values must produce the same
/// byte sequence into the hasher, regardless of in-memory layout.
pub trait StateHash {
    fn state_hash<H: Hasher>(&self, state: &mut H);
}

/// Feed an explicit marker for one intentionally unhashed field.
///
/// Skipped fields used to contribute no bytes at all. That made adding or
/// removing a skip annotation invisible whenever the field's ordinary hash
/// was also empty (for example `()`). Keeping a marker in declaration order
/// makes the opt-out part of the hash schema while still ignoring its value.
#[doc(hidden)]
#[inline]
pub fn hash_skipped_field<H: Hasher>(state: &mut H) {
    // ASCII "SKIPFLD\0", interpreted as a fixed little-endian tag.
    state.write_u64(0x0044_4c46_5049_4b53);
}

// ─── Primitives ───────────────────────────────────────────────────

macro_rules! impl_state_hash_int {
    ($($t:ty => $write:ident),* $(,)?) => {
        $(
            impl StateHash for $t {
                #[inline]
                fn state_hash<H: Hasher>(&self, state: &mut H) {
                    state.$write(*self);
                }
            }
        )*
    };
}

impl_state_hash_int! {
    u8 => write_u8,
    u16 => write_u16,
    u32 => write_u32,
    u64 => write_u64,
    u128 => write_u128,
    usize => write_usize,
    i8 => write_i8,
    i16 => write_i16,
    i32 => write_i32,
    i64 => write_i64,
    i128 => write_i128,
    isize => write_isize,
}

impl StateHash for bool {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u8(*self as u8);
    }
}

// ─── nonmax niche-optimized integers ───────────────────────────────

macro_rules! impl_state_hash_nonmax {
    ($($t:ty => $write:ident),* $(,)?) => {
        $(
            impl StateHash for $t {
                #[inline]
                fn state_hash<H: Hasher>(&self, state: &mut H) {
                    state.$write(self.get());
                }
            }
        )*
    };
}

impl_state_hash_nonmax! {
    nonmax::NonMaxU8 => write_u8,
    nonmax::NonMaxU16 => write_u16,
    nonmax::NonMaxU32 => write_u32,
    nonmax::NonMaxU64 => write_u64,
    nonmax::NonMaxUsize => write_usize,
    nonmax::NonMaxI8 => write_i8,
    nonmax::NonMaxI16 => write_i16,
    nonmax::NonMaxI32 => write_i32,
    nonmax::NonMaxI64 => write_i64,
    nonmax::NonMaxIsize => write_isize,
}

impl_state_hash_nonmax! {
    std::num::NonZeroU8 => write_u8,
    std::num::NonZeroU16 => write_u16,
    std::num::NonZeroU32 => write_u32,
    std::num::NonZeroU64 => write_u64,
    std::num::NonZeroUsize => write_usize,
    std::num::NonZeroI8 => write_i8,
    std::num::NonZeroI16 => write_i16,
    std::num::NonZeroI32 => write_i32,
    std::num::NonZeroI64 => write_i64,
    std::num::NonZeroIsize => write_isize,
}

impl StateHash for char {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u32(*self as u32);
    }
}

impl StateHash for f32 {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        // Canonicalize NaN — every NaN bit-pattern hashes to the same
        // bytes. NaN should never appear in deterministic sim state,
        // but if it does we don't want a different bit pattern to
        // produce a different hash and trigger a false desync.
        let bits = if self.is_nan() {
            f32::NAN.to_bits()
        } else if *self == 0.0 {
            // -0.0 and 0.0 compare equal but have different bits.
            0.0f32.to_bits()
        } else {
            self.to_bits()
        };
        state.write_u32(bits);
    }
}

impl StateHash for f64 {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        let bits = if self.is_nan() {
            f64::NAN.to_bits()
        } else if *self == 0.0 {
            0.0f64.to_bits()
        } else {
            self.to_bits()
        };
        state.write_u64(bits);
    }
}

impl StateHash for str {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.len() as u64);
        state.write(self.as_bytes());
    }
}

impl StateHash for String {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().state_hash(state);
    }
}

impl<T: StateHash + ?Sized> StateHash for &T {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        (**self).state_hash(state);
    }
}

impl<T: StateHash + ?Sized> StateHash for &mut T {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        (**self).state_hash(state);
    }
}

impl<T: StateHash + ?Sized> StateHash for Box<T> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        (**self).state_hash(state);
    }
}

impl<T: StateHash + ?Sized> StateHash for std::sync::Arc<T> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        (**self).state_hash(state);
    }
}

impl<T: StateHash + ?Sized> StateHash for std::rc::Rc<T> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        (**self).state_hash(state);
    }
}

impl<T: StateHash> StateHash for Option<T> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        match self {
            None => state.write_u8(0),
            Some(v) => {
                state.write_u8(1);
                v.state_hash(state);
            }
        }
    }
}

impl<T: StateHash, E: StateHash> StateHash for Result<T, E> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Ok(v) => {
                state.write_u8(0);
                v.state_hash(state);
            }
            Err(v) => {
                state.write_u8(1);
                v.state_hash(state);
            }
        }
    }
}

// ─── Sequences ────────────────────────────────────────────────────

impl<T: StateHash> StateHash for [T] {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.len() as u64);
        for item in self {
            item.state_hash(state);
        }
    }
}

impl<T: StateHash, const N: usize> StateHash for [T; N] {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        // Length is constant; skip it.
        for item in self {
            item.state_hash(state);
        }
    }
}

impl<T: StateHash> StateHash for Vec<T> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().state_hash(state);
    }
}

impl<K, V> StateHash for enum_map::EnumMap<K, V>
where
    K: enum_map::EnumArray<V>,
    V: StateHash,
{
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().state_hash(state);
    }
}

impl<T: StateHash> StateHash for std::collections::VecDeque<T> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.len() as u64);
        for item in self {
            item.state_hash(state);
        }
    }
}

// ─── Maps & sets ──────────────────────────────────────────────────

impl<K: StateHash, V: StateHash> StateHash for std::collections::BTreeMap<K, V> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.len() as u64);
        for (k, v) in self {
            k.state_hash(state);
            v.state_hash(state);
        }
    }
}

/// `IndexMap` iteration order is deterministic insertion order and can be
/// authoritative simulation state, so hash entries without key sorting.
impl<K: StateHash, V: StateHash> StateHash for indexmap::IndexMap<K, V> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.len() as u64);
        for (key, value) in self {
            key.state_hash(state);
            value.state_hash(state);
        }
    }
}

impl<T: StateHash> StateHash for std::collections::BTreeSet<T> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.len() as u64);
        for item in self {
            item.state_hash(state);
        }
    }
}

/// `HashMap` iteration order isn't deterministic. To be safe we
/// require keys to be `Ord` and hash entries in sorted order.
impl<K: StateHash + Ord, V: StateHash> StateHash for std::collections::HashMap<K, V> {
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        let mut entries: Vec<(&K, &V)> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        state.write_u64(entries.len() as u64);
        for (k, v) in entries {
            k.state_hash(state);
            v.state_hash(state);
        }
    }
}

impl<T: StateHash + Ord> StateHash for std::collections::HashSet<T> {
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        let mut items: Vec<&T> = self.iter().collect();
        items.sort();
        state.write_u64(items.len() as u64);
        for item in items {
            item.state_hash(state);
        }
    }
}

// ─── Tuples ───────────────────────────────────────────────────────

macro_rules! impl_state_hash_tuple {
    ($($name:ident),+) => {
        impl<$($name: StateHash),+> StateHash for ($($name,)+) {
            #[inline]
            #[allow(non_snake_case)]
            fn state_hash<H: Hasher>(&self, state: &mut H) {
                let ($(ref $name,)+) = *self;
                $($name.state_hash(state);)+
            }
        }
    };
}

impl StateHash for () {
    #[inline]
    fn state_hash<H: Hasher>(&self, _state: &mut H) {}
}

impl_state_hash_tuple!(A);
impl_state_hash_tuple!(A, B);
impl_state_hash_tuple!(A, B, C);
impl_state_hash_tuple!(A, B, C, D);
impl_state_hash_tuple!(A, B, C, D, E);
impl_state_hash_tuple!(A, B, C, D, E, F);
impl_state_hash_tuple!(A, B, C, D, E, F, G);
impl_state_hash_tuple!(A, B, C, D, E, F, G, H_);

// ─── geo crate types ─────────────────────────────────────────────

impl StateHash for geo::Coord<f32> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.x.state_hash(state);
        self.y.state_hash(state);
    }
}

impl StateHash for geo::Rect<f32> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.min().state_hash(state);
        self.max().state_hash(state);
    }
}

impl StateHash for geo::Line<f32> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.start.state_hash(state);
        self.end.state_hash(state);
    }
}

impl StateHash for geo::LineString<f32> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        let coords = &self.0;
        (coords.len() as u64).state_hash(state);
        for c in coords {
            c.state_hash(state);
        }
    }
}

impl StateHash for geo::Polygon<f32> {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.exterior().state_hash(state);
        let interiors = self.interiors();
        (interiors.len() as u64).state_hash(state);
        for ring in interiors {
            ring.state_hash(state);
        }
    }
}

// ─── fastrand::Rng ───────────────────────────────────────────────

impl StateHash for fastrand::Rng {
    #[inline]
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.get_seed().state_hash(state);
    }
}

// ─── Computing the final hash ─────────────────────────────────────

/// Compute a single u64 hash of `value` using FxHasher — ~5–10× faster
/// than SipHash for the small-integer / byte-stream pattern that
/// `StateHash` produces, and fully deterministic (no random seed).
/// Collision resistance is not required: this is a replay-divergence
/// detector, not a crypto hash.
pub fn compute<T: StateHash + ?Sized>(value: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = rustc_hash::FxHasher::default();
    value.state_hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_canonicalize_zero_and_nan() {
        // -0.0 and 0.0 hash equally.
        assert_eq!(compute(&0.0_f32), compute(&-0.0_f32));
        assert_eq!(compute(&0.0_f64), compute(&-0.0_f64));
        // Different NaN bit-patterns hash equally.
        let nan_a = f32::NAN;
        let nan_b = f32::from_bits(0x7fc00001);
        assert!(nan_a.is_nan() && nan_b.is_nan());
        assert_eq!(compute(&nan_a), compute(&nan_b));
    }

    #[test]
    fn distinct_floats_differ() {
        assert_ne!(compute(&1.0_f32), compute(&2.0_f32));
        assert_ne!(compute(&1.0_f64), compute(&1.0000001_f64));
    }

    #[test]
    fn vec_length_prefix_disambiguates() {
        // [1, 23] and [12, 3] would otherwise hash to the same bytes
        // without a length prefix or per-element separator.
        let a: Vec<u8> = vec![1, 23];
        let b: Vec<u8> = vec![12, 3];
        assert_ne!(compute(&a), compute(&b));
    }

    #[test]
    fn hashmap_order_independent() {
        use std::collections::HashMap;
        let mut a: HashMap<u32, u32> = HashMap::new();
        a.insert(1, 100);
        a.insert(2, 200);
        a.insert(3, 300);
        let mut b: HashMap<u32, u32> = HashMap::new();
        b.insert(3, 300);
        b.insert(1, 100);
        b.insert(2, 200);
        assert_eq!(compute(&a), compute(&b));
    }
}
