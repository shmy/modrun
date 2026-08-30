use std::any::{TypeId, type_name};

/// Maximum constructor / invoker arity. Matches the `ProviderFn` / `InvokeFn` impls.
pub(crate) const MAX_DEPS: usize = 8;

/// One constructor / invoker parameter.
#[derive(Clone, Copy)]
pub(crate) struct Dep {
    pub id: TypeId,
    pub name: &'static str,
}

/// Stack-allocated dependency list. Arity is known at compile time (0–8).
#[derive(Clone, Copy)]
pub(crate) struct DepList {
    items: [(TypeId, &'static str); MAX_DEPS],
    len: u8,
}

impl DepList {
    pub(crate) fn empty() -> Self {
        Self {
            items: [(TypeId::of::<()>(), ""); MAX_DEPS],
            len: 0,
        }
    }

    pub(crate) fn from_array<const N: usize>(arr: [Dep; N]) -> Self {
        const { assert!(N <= MAX_DEPS, "constructors accept at most 8 parameters") };
        let mut items = [(TypeId::of::<()>(), ""); MAX_DEPS];
        for (i, dep) in arr.into_iter().enumerate() {
            items[i] = (dep.id, dep.name);
        }
        Self {
            items,
            len: N as u8,
        }
    }

    pub(crate) fn as_slice(&self) -> &[(TypeId, &'static str)] {
        &self.items[..self.len as usize]
    }
}

/// Compile-time `(TypeId, type_name)` pair for a dependency parameter.
#[inline]
pub(crate) fn dep<T: 'static>() -> Dep {
    Dep {
        id: TypeId::of::<T>(),
        name: type_name::<T>(),
    }
}
