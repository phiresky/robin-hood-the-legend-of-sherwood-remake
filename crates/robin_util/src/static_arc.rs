//! Explicit marker for immutable shared attachments.
//!
//! Use this when an `Arc<T>` points at load-time/static data that may be
//! serialized as part of a snapshot but is not simulation state for rollback
//! hashing. Raw `Arc<T>` still hashes through to `T`; this wrapper is the
//! semantic opt-out.

use crate::state_hash::StateHash;
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct StaticArc<T>(Arc<T>);

impl<T> StaticArc<T> {
    #[inline]
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }

    #[inline]
    pub fn from_arc(value: Arc<T>) -> Self {
        Self(value)
    }

    #[inline]
    pub fn make_mut(this: &mut Self) -> &mut T
    where
        T: Clone,
    {
        Arc::make_mut(&mut this.0)
    }
}

impl<T> Clone for StaticArc<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Default> Default for StaticArc<T> {
    #[inline]
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for StaticArc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> From<T> for StaticArc<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> From<Arc<T>> for StaticArc<T> {
    #[inline]
    fn from(value: Arc<T>) -> Self {
        Self::from_arc(value)
    }
}

impl<T> Deref for StaticArc<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> StateHash for StaticArc<T> {
    #[inline]
    fn state_hash<H: std::hash::Hasher>(&self, _state: &mut H) {}
}

#[cfg(test)]
mod tests {
    use super::StaticArc;
    use crate::state_hash;

    #[test]
    fn static_arc_is_not_state_hashed() {
        let a = StaticArc::new(vec![1_u32, 2, 3]);
        let b = StaticArc::new(vec![9_u32, 8, 7]);
        assert_eq!(state_hash::compute(&a), state_hash::compute(&b));
    }
}
