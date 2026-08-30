//! Value groups: many providers contribute one element each; consumers receive [`Group`].
//!
//! Register members with [`ModrunBuilder::provide_group`](crate::ModrunBuilder::provide_group)
//! (and async / fallible variants). Inject the collection as `Group<T>` in an invoker
//! or constructor parameter. If nothing contributes, call
//! [`ModrunBuilder::init_group`](crate::ModrunBuilder::init_group) (or
//! [`require_group`](crate::ModrunBuilder::require_group), which also registers
//! the group).
//!
//! Container wiring requires `T: Clone` for group members and injected `Group<T>`;
//! use `Arc<Group<T>>` or `Group<Arc<T>>` when cloning would be expensive.

/// Many values of the same type, aggregated from [`provide_group`](crate::ModrunBuilder::provide_group)
/// registrations across modules.
///
/// Injecting `Group<T>` (or `Arc<Group<T>>`) requires the group to be registered:
/// at least one [`provide_group`](crate::ModrunBuilder::provide_group), or
/// [`init_group`](crate::ModrunBuilder::init_group) /
/// [`require_group`](crate::ModrunBuilder::require_group). With no members the
/// group is empty. `require_group` rejects an empty group at build time.
///
/// ```
/// # use modrun::Group;
/// let routes = Group::from_vec(vec![1, 2, 3]);
/// assert_eq!(routes.len(), 3);
/// for n in routes {
///     assert!(n >= 1);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Group<T> {
    items: Vec<T>,
}

impl<T> Group<T> {
    /// An empty [`Group`] value. In the container, register the group with
    /// [`ModrunBuilder::init_group`](crate::ModrunBuilder::init_group),
    /// [`ModrunBuilder::provide_group`](crate::ModrunBuilder::provide_group), or
    /// [`ModrunBuilder::require_group`](crate::ModrunBuilder::require_group) before
    /// injecting `Group<T>`.
    #[must_use]
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Wrap an existing vector.
    #[must_use]
    pub fn from_vec(items: Vec<T>) -> Self {
        Self { items }
    }

    /// Number of contributed values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the group has no members.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Borrow the inner slice.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    /// Iterate over members.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    /// Consume the group and return the inner vector.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.items
    }
}

impl<T> Default for Group<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> IntoIterator for Group<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Group<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<T> AsRef<[T]> for Group<T> {
    fn as_ref(&self) -> &[T] {
        &self.items
    }
}

impl<T> std::ops::Deref for Group<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.items
    }
}

impl<T> From<Vec<T>> for Group<T> {
    fn from(items: Vec<T>) -> Self {
        Self::from_vec(items)
    }
}

impl<T> FromIterator<T> for Group<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from_vec(iter.into_iter().collect())
    }
}
