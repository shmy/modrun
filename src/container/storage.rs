use std::any::{TypeId, type_name};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::scope::ScopeId;

use super::types::{ArcBox, Constructed};
use super::{ArcResolveFn, Container, DynAny, TypeIdMap};

pub(crate) fn pack<T: Send + Sync + 'static>(value: T) -> Constructed {
    let arc = Arc::new(value);
    Constructed {
        value: Arc::clone(&arc) as DynAny,
        arc_alias: Some((
            TypeId::of::<Arc<T>>(),
            Arc::new(ArcBox(Arc::clone(&arc))) as DynAny,
        )),
        register_arc: Some(|map| register_arc_resolver::<T>(map)),
    }
}

pub(crate) fn register_arc_resolver<T: Send + Sync + 'static>(
    resolvers: &mut TypeIdMap<TypeId, ArcResolveFn>,
) {
    fn resolve<T: Send + Sync + 'static>(
        value: &DynAny,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>> {
        let arc = value
            .downcast_ref::<ArcBox<T>>()
            .ok_or_else(|| Error::Downcast(type_name::<Arc<T>>()))?
            .0
            .clone();
        Ok(Box::new(arc))
    }
    resolvers
        .entry(TypeId::of::<Arc<T>>())
        .or_insert(resolve::<T>);
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
        Ok(())
    }

    pub(crate) fn get<T: Clone + Send + Sync + 'static>(&self) -> Result<T> {
        let id = TypeId::of::<T>();
        if let Some(&resolve) = self.arc_resolvers.get(&id) {
            let value = self
                .lookup_value_ref_from(id, self.active_scope)
                .ok_or_else(|| Error::NotConstructed(type_name::<T>()))?;
            let boxed = resolve(value)?;
            let typed = *boxed
                .downcast::<T>()
                .map_err(|_| Error::Downcast(type_name::<T>()))?;
            return Ok(typed);
        }

        let value = self
            .lookup_value_ref_from(id, self.active_scope)
            .ok_or_else(|| Error::NotConstructed(type_name::<T>()))?;
        downcast_clone::<T>(value)
    }

    pub(crate) fn store_constructed(
        &mut self,
        id: TypeId,
        built: Constructed,
        scope: ScopeId,
        private: bool,
    ) {
        if let Some(register) = built.register_arc {
            register(&mut self.arc_resolvers);
        }
        if let Some((alias_id, alias_value)) = built.arc_alias {
            self.store_value(alias_id, alias_value, scope, private);
        }
        self.store_value(id, built.value, scope, private);
    }

    pub(crate) fn store_value(&mut self, id: TypeId, value: DynAny, scope: ScopeId, private: bool) {
        if private {
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
    lifecycle: crate::lifecycle::Lifecycle,
    shutdowner: crate::shutdown::Shutdowner,
) -> Result<()> {
    container.insert_value(lifecycle, ScopeId::ROOT, false)?;
    container.insert_value(shutdowner, ScopeId::ROOT, false)?;
    Ok(())
}
