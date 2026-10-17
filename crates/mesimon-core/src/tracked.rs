//! `Tracked<T>`: a value that knows it may have changed (T-839).
//!
//! A state file written "when it changed" learned that by serialising the
//! whole value and comparing the text with the last write — 22 ms a tick on
//! a 655-record `sessions.json` in a debug build, to learn nothing moved.
//! A `Tracked` is marked the moment anyone takes `&mut` to it, through
//! `DerefMut`, so no mutator has to remember it and none can be missed:
//! the compiler routes every write through the mark. The mark
//! over-approximates (a `get_mut` that changes nothing still marks), which
//! is why a marked value still goes through the text comparison; an
//! unmarked one skips it.
//!
//! A new or cloned value starts marked, so a save that follows it is never
//! skipped. It serialises as the value alone: nothing on disk or on the
//! wire changes.

use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub struct Tracked<T> {
    value: T,
    dirty: bool,
}

impl<T> Tracked<T> {
    pub fn new(value: T) -> Self {
        Self { value, dirty: true }
    }

    /// Whether anything took `&mut` since the last `mark_clean`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The value as it stands is the one written.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.dirty = true;
        &mut self.value
    }
}

impl<T> From<T> for Tracked<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<A, T: FromIterator<A>> FromIterator<A> for Tracked<T> {
    fn from_iter<I: IntoIterator<Item = A>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<T: IntoIterator> IntoIterator for Tracked<T> {
    type Item = T::Item;
    type IntoIter = T::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.value.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Tracked<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.value.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Tracked<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().into_iter()
    }
}

impl<T: Default> Default for Tracked<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone> Clone for Tracked<T> {
    fn clone(&self) -> Self {
        Self::new(self.value.clone())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Tracked<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: PartialEq> PartialEq for Tracked<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Serialize> Serialize for Tracked<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(s)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Tracked<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_marks_a_read_does_not_and_a_clone_starts_marked() {
        let mut v = Tracked::new(vec![1, 2]);
        assert!(v.is_dirty(), "new is marked: its first save is never skipped");
        v.mark_clean();
        assert_eq!(v.len(), 2);
        assert!(v.iter().any(|n| *n == 2));
        assert!(!v.is_dirty());
        v.push(3);
        assert!(v.is_dirty());
        v.mark_clean();
        assert!(v.clone().is_dirty());
        assert!(!v.is_dirty());
    }

    #[test]
    fn it_serialises_as_the_value_alone() {
        let v = Tracked::new(vec![1, 2]);
        assert_eq!(serde_json::to_string(&v).unwrap(), "[1,2]");
        let back: Tracked<Vec<u8>> = serde_json::from_str("[3]").unwrap();
        assert!(back.is_dirty());
        assert_eq!(*back, vec![3]);
    }
}
