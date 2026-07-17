//! Explicit marker for immutable shared attachments.
//!
//! `StaticArc<T>` is for load-time data that is available while a live engine
//! runs but is not simulation state. Its payload is omitted from both snapshot
//! serialization and rollback hashing. A deserialized value is therefore
//! explicitly detached and panics on dereference until the owning engine
//! reattaches the matching level asset.
//!
//! Unlike substituting `T::default()` during deserialization, detachment cannot
//! silently feed fabricated data into gameplay. `make_mut` remains a
//! copy-on-write builder operation for load-time assembly; because the payload
//! is absent from both snapshots and hashes, such changes cannot make those two
//! representations disagree.

use crate::state_hash::{StateHash, hash_skipped_field};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::Deref;
use std::sync::Arc;

pub struct StaticArc<T>(Option<Arc<T>>);

/// Access was attempted before the owning engine reattached level data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("StaticArc is detached; attach level assets first")]
pub struct DetachedStaticArc;

impl<T> StaticArc<T> {
    #[inline]
    pub fn new(value: T) -> Self {
        Self(Some(Arc::new(value)))
    }

    #[inline]
    pub fn from_arc(value: Arc<T>) -> Self {
        Self(Some(value))
    }

    /// Whether the level/static payload has been attached.
    #[inline]
    pub fn is_attached(&self) -> bool {
        self.0.is_some()
    }

    /// Borrow the attachment, returning a typed error while detached.
    #[inline]
    pub fn get(&self) -> Result<&T, DetachedStaticArc> {
        self.0.as_deref().ok_or(DetachedStaticArc)
    }

    /// Replace a detached or stale attachment with the level's canonical Arc.
    #[inline]
    pub fn attach(&mut self, value: Arc<T>) {
        self.0 = Some(value);
    }

    /// Copy-on-write access for load-time builders.
    ///
    /// # Panics
    ///
    /// Panics when called on a deserialized value before [`Self::attach`].
    #[inline]
    pub fn make_mut(this: &mut Self) -> &mut T
    where
        T: Clone,
    {
        let value = this
            .0
            .as_mut()
            .expect("attempted to mutate a detached StaticArc; attach level assets first");
        Arc::make_mut(value)
    }
}

impl<T> Clone for StaticArc<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
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
        match &self.0 {
            Some(value) => value.fmt(f),
            None => f.write_str("StaticArc(<detached>)"),
        }
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
        self.get()
            .expect("attempted to use a detached StaticArc; attach level assets first")
    }
}

impl<T> Serialize for StaticArc<T> {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_unit()
    }
}

impl<'de, T> Deserialize<'de> for StaticArc<T> {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        <()>::deserialize(deserializer)?;
        Ok(Self(None))
    }
}

impl<T> StateHash for StaticArc<T> {
    #[inline]
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        hash_skipped_field(state);
    }
}

#[cfg(test)]
mod tests {
    use super::StaticArc;
    use crate::state_hash;

    #[test]
    fn payload_is_omitted_from_both_snapshot_and_hash() {
        let a = StaticArc::new(vec![1_u32, 2, 3]);
        let b = StaticArc::new(vec![9_u32, 8, 7]);

        assert_eq!(serde_json::to_value(&a).unwrap(), serde_json::Value::Null);
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        assert_eq!(state_hash::compute(&a), state_hash::compute(&b));
    }

    #[test]
    fn deserialize_is_detached_instead_of_fabricating_a_default() {
        let decoded: StaticArc<Vec<u32>> = serde_json::from_str("null").unwrap();
        assert!(!decoded.is_attached());
        assert_eq!(decoded.get(), Err(super::DetachedStaticArc));
    }

    #[test]
    fn attachment_restores_access_after_deserialization() {
        let mut decoded: StaticArc<Vec<u32>> = serde_json::from_str("null").unwrap();
        decoded.attach(std::sync::Arc::new(vec![4, 5, 6]));

        assert!(decoded.is_attached());
        assert_eq!(&*decoded, &[4, 5, 6]);
    }

    #[test]
    fn copy_on_write_mutation_stays_out_of_snapshot_and_hash() {
        let original = StaticArc::new(vec![1_u32]);
        let mut changed = original.clone();
        StaticArc::make_mut(&mut changed).push(2);

        assert_eq!(&*original, &[1]);
        assert_eq!(&*changed, &[1, 2]);
        assert_eq!(
            serde_json::to_value(&original).unwrap(),
            serde_json::to_value(&changed).unwrap()
        );
        assert_eq!(
            state_hash::compute(&original),
            state_hash::compute(&changed)
        );
    }
}
