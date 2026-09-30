use std::any::{Any, TypeId, type_name};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::scope::ScopeId;

use super::types::{Constructed, StoredValue, ValueNode};
use super::{Container, DynAny};
use crate::lifecycle::Lifecycle;
use crate::shutdown::Shutdowner;

pub(crate) fn pack<T: Send + Sync + 'static>(value: T) -> Constructed {
    let arc = Arc::new(value);
    Constructed {
        value: Arc::clone(&arc) as DynAny,
        // Alias the same handle under `Arc<T>`'s id. This is a refcount bump, not
        // a second allocation: the handle already points at `T`, and the stored
        // extractor turns it back into `Arc<T>` for `get::<Arc<T>>()`.
        arc_alias: Some((TypeId::of::<Arc<T>>(), arc as DynAny, extract_arc::<T>)),
    }
}

/// Recover `Arc<T>` out of the stored handle (whose pointee is `T`).
pub(crate) fn extract_arc<T: Send + Sync + 'static>(
    value: &DynAny,
) -> Result<Box<dyn Any + Send + Sync>> {
    let arc = Arc::downcast::<T>(Arc::clone(value))
        .map_err(|_| Error::Downcast(type_name::<Arc<T>>()))?;
    Ok(Box::new(arc))
}

impl Container {
    pub(crate) fn insert_value<T: Send + Sync + 'static>(
        &mut self,
        value: T,
        scope: ScopeId,
        private: bool,
    ) -> Result<()> {
        let packed = pack(value);
        ensure_absent(self, TypeId::of::<T>(), type_name::<T>(), scope, private)?;
        ensure_absent(
            self,
            TypeId::of::<Arc<T>>(),
            type_name::<Arc<T>>(),
            scope,
            private,
        )?;
        self.store_constructed(TypeId::of::<T>(), packed, scope, private);
        self.value_nodes.push(ValueNode {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
            scope,
            private,
        });
        self.value_nodes.push(ValueNode {
            type_id: TypeId::of::<Arc<T>>(),
            type_name: type_name::<Arc<T>>(),
            scope,
            private,
        });
        Ok(())
    }

    pub(crate) fn get<T: Clone + Send + Sync + 'static>(&self) -> Result<T> {
        let id = TypeId::of::<T>();
        let stored = self
            .lookup_value_ref_from(id, self.active_scope)
            .ok_or_else(|| Error::NotConstructed(type_name::<T>()))?;
        match stored.extract {
            Some(extract) => {
                let boxed = extract(&stored.handle)?;
                let typed = *boxed
                    .downcast::<T>()
                    .map_err(|_| Error::Downcast(type_name::<T>()))?;
                Ok(typed)
            }
            None => downcast_clone::<T>(&stored.handle),
        }
    }

    pub(crate) fn store_constructed(
        &mut self,
        id: TypeId,
        built: Constructed,
        scope: ScopeId,
        private: bool,
    ) {
        if let Some((alias_id, alias_handle, extract)) = built.arc_alias {
            self.store_value(
                alias_id,
                StoredValue {
                    handle: alias_handle,
                    extract: Some(extract),
                },
                scope,
                private,
            );
        }
        self.store_value(
            id,
            StoredValue {
                handle: built.value,
                extract: None,
            },
            scope,
            private,
        );
    }

    pub(crate) fn store_value(
        &mut self,
        id: TypeId,
        value: StoredValue,
        scope: ScopeId,
        private: bool,
    ) {
        if private {
            self.mark_private_scope(scope);
            self.values_private.insert((id, scope), value);
        } else {
            self.values_public.insert(id, value);
        }
    }
}

pub(crate) fn ensure_absent(
    container: &Container,
    id: TypeId,
    name: &'static str,
    scope: ScopeId,
    private: bool,
) -> Result<()> {
    use super::types::ProviderKey;

    let conflict = if private {
        container.values_private.contains_key(&(id, scope))
            || container
                .providers
                .contains_key(&ProviderKey::singleton(id, scope, true))
            || container.private_alias.contains_key(&(id, scope))
    } else {
        container.values_public.contains_key(&id) || container.public_index.contains_key(&id)
    };

    if !conflict {
        return Ok(());
    }

    if private {
        Err(Error::AlreadyProvidedPrivate {
            module: container.scopes.name(scope),
            type_name: name,
        })
    } else {
        Err(Error::AlreadyProvided(name))
    }
}

pub(crate) fn downcast_clone<T: Clone + Send + Sync + 'static>(value: &DynAny) -> Result<T> {
    value
        .downcast_ref::<T>()
        .cloned()
        .ok_or_else(|| Error::Downcast(type_name::<T>()))
}

/// Move a packed member out of its `Arc` when this is the last handle.
pub(crate) fn take_packed_member<T: Clone + Send + Sync + 'static>(value: DynAny) -> Result<T> {
    let arc = Arc::downcast::<T>(value).map_err(|_| Error::Downcast(type_name::<T>()))?;
    match Arc::try_unwrap(arc) {
        Ok(value) => Ok(value),
        Err(arc) => Ok((*arc).clone()),
    }
}

pub(crate) fn seed_builtins(
    container: &mut Container,
    lifecycle: Lifecycle,
    shutdowner: Shutdowner,
) -> Result<()> {
    container.insert_value(lifecycle, ScopeId::ROOT, false)?;
    container.insert_value(shutdowner, ScopeId::ROOT, false)?;
    Ok(())
}
