use std::any::TypeId;
use std::sync::Arc;

use crate::future::BoxFuture;
use crate::scope::ScopeId;

use super::DynAny;

/// Hasher for keys built out of a [`TypeId`].
///
/// `TypeId` is already a well-distributed hash, so feeding it through SipHash a
/// second time only costs cycles. Compound keys — `(TypeId, ScopeId)` and
/// [`ProviderKey`] — fold each field into the accumulator so the extra
/// discriminants still contribute.
#[derive(Default)]
pub(crate) struct TypeIdHasher(u64);

impl TypeIdHasher {
    fn fold(&mut self, value: u64) {
        self.0 = self.0.rotate_left(11) ^ value;
    }
}

impl std::hash::Hasher for TypeIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.fold(u64::from(n));
    }

    fn write_u16(&mut self, n: u16) {
        self.fold(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.fold(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.fold(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.fold(n as u64);
    }

    fn write_u128(&mut self, n: u128) {
        self.fold(n as u64);
        self.fold((n >> 64) as u64);
    }
}

pub(crate) type TypeIdMap<K, V> =
    std::collections::HashMap<K, V, std::hash::BuildHasherDefault<TypeIdHasher>>;
pub(crate) type TypeIdSet<K> =
    std::collections::HashSet<K, std::hash::BuildHasherDefault<TypeIdHasher>>;

/// Stored under [`TypeId::of::<Arc<T>>()`] so `get::<Arc<T>>()` can recover the
/// handle without an extra `Arc<Arc<T>>` allocation.
pub(crate) struct ArcBox<T: Send + Sync + ?Sized>(pub Arc<T>);

pub(crate) type ArcRegisterFn = fn(&mut TypeIdMap<TypeId, crate::container::ArcResolveFn>);

/// Value produced by a constructor, plus an optional `Arc<T>` alias.
pub(crate) struct Constructed {
    pub value: DynAny,
    pub arc_alias: Option<(TypeId, DynAny)>,
    pub register_arc: Option<ArcRegisterFn>,
}

pub(crate) type ConstructFuture = BoxFuture<'static, crate::error::Result<Constructed>>;

pub(crate) enum ConstructOut {
    Ready(Constructed),
    Fut(ConstructFuture),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ProviderKey {
    pub(crate) type_id: TypeId,
    pub(crate) scope: ScopeId,
    pub(crate) private: bool,
    /// `0` for singleton and group-virtual providers; `1+` for group members.
    pub(crate) ordinal: u32,
}

impl ProviderKey {
    pub(crate) fn singleton(type_id: TypeId, scope: ScopeId, private: bool) -> Self {
        Self {
            type_id,
            scope,
            private,
            ordinal: 0,
        }
    }

    pub(crate) fn is_group_member(self) -> bool {
        self.ordinal > 0
    }
}

/// Element type `T` for [`crate::Group<T>`] registrations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GroupElementKey {
    pub(crate) element: TypeId,
}

pub(crate) struct GroupRegistration {
    pub(crate) element: TypeId,
    pub(crate) virtual_key: ProviderKey,
}

/// A value inserted via [`super::storage::insert_value`] (supply / builtins).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValueNode {
    pub(crate) type_id: TypeId,
    pub(crate) type_name: &'static str,
    pub(crate) scope: ScopeId,
    pub(crate) private: bool,
}
