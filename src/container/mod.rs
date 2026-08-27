use std::any::{Any, TypeId};
use std::sync::Arc;

use crate::error::Result;
use crate::provide::DynProvider;
use crate::scope::{ScopeId, ScopeTree};

mod build;
mod graph;
mod storage;
mod types;

pub(crate) use storage::{pack, register_arc_resolver, seed_builtins};
pub(crate) use types::ConstructOut;
pub(crate) use types::{ArcRegisterFn, ProviderKey, TypeIdMap, TypeIdSet};

pub(crate) type DynAny = Arc<dyn Any + Send + Sync>;
pub(crate) type ArcResolveFn = fn(&DynAny) -> Result<Box<dyn Any + Send + Sync>>;

pub(crate) struct Container {
    pub(crate) scopes: ScopeTree,
    pub(crate) values_public: TypeIdMap<TypeId, DynAny>,
    pub(crate) values_private: TypeIdMap<(TypeId, ScopeId), DynAny>,
    pub(crate) providers: TypeIdMap<ProviderKey, DynProvider>,
    pub(crate) public_index: TypeIdMap<TypeId, ProviderKey>,
    pub(crate) private_alias: TypeIdMap<(TypeId, ScopeId), ProviderKey>,
    pub(crate) provider_order: Vec<ProviderKey>,
    pub(crate) provider_order_index: TypeIdMap<ProviderKey, usize>,
    pub(crate) constructing: TypeIdSet<ProviderKey>,
    pub(crate) active_scope: ScopeId,
    pub(crate) layers: Vec<Vec<ProviderKey>>,
    pub(crate) arc_resolvers: TypeIdMap<TypeId, ArcResolveFn>,
    pub(crate) wave_scratch: Vec<ProviderKey>,
}

impl Container {
    pub(crate) fn new() -> Self {
        Self {
            scopes: ScopeTree::new(),
            values_public: TypeIdMap::default(),
            values_private: TypeIdMap::default(),
            providers: TypeIdMap::default(),
            public_index: TypeIdMap::default(),
            private_alias: TypeIdMap::default(),
            provider_order: Vec::new(),
            provider_order_index: TypeIdMap::default(),
            constructing: TypeIdSet::default(),
            active_scope: ScopeId::ROOT,
            layers: Vec::new(),
            arc_resolvers: TypeIdMap::default(),
            wave_scratch: Vec::new(),
        }
    }

    pub(crate) fn scopes(&self) -> &ScopeTree {
        &self.scopes
    }

    pub(crate) fn scopes_mut(&mut self) -> &mut ScopeTree {
        &mut self.scopes
    }

    pub(crate) fn enter_scope(&mut self, scope: ScopeId) -> ScopeId {
        std::mem::replace(&mut self.active_scope, scope)
    }

    pub(crate) fn leave_scope(&mut self, previous: ScopeId) {
        self.active_scope = previous;
    }

    pub(crate) fn insert_provider(
        &mut self,
        provider: DynProvider,
        scope: ScopeId,
        private: bool,
    ) -> Result<()> {
        let id = provider.result_type();
        storage::ensure_absent(self, id, provider.result_name(), scope, private)?;
        for &alias in provider.alias_types() {
            storage::ensure_absent(self, alias, provider.result_name(), scope, private)?;
        }
        let key = ProviderKey {
            type_id: id,
            scope,
            private,
        };
        if private {
            for &alias in provider.alias_types() {
                self.private_alias.insert((alias, scope), key);
            }
        } else {
            self.public_index.insert(id, key);
            for &alias in provider.alias_types() {
                self.public_index.insert(alias, key);
            }
        }
        self.providers.insert(key, provider);
        if let Some(register) = self.providers.get(&key).and_then(|p| p.register_arc()) {
            register(&mut self.arc_resolvers);
        }
        self.provider_order_index
            .insert(key, self.provider_order.len());
        self.provider_order.push(key);
        Ok(())
    }

    pub(crate) fn provider_at(&self, key: ProviderKey) -> Option<&DynProvider> {
        self.providers.get(&key)
    }

    pub(crate) fn key_name(&self, key: ProviderKey) -> &'static str {
        self.provider_at(key)
            .map(|p| p.result_name())
            .unwrap_or("<unknown>")
    }

    pub(crate) fn order_index(&self, key: ProviderKey) -> usize {
        self.provider_order_index
            .get(&key)
            .copied()
            .unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provide::ProviderFn;

    #[test]
    fn public_index_rejects_duplicate() {
        let mut c = Container::new();
        let p = (|| 1u32).into_provider();
        c.insert_provider(p, ScopeId::ROOT, false).unwrap();
        let p2 = (|| 2u32).into_provider();
        let err = c.insert_provider(p2, ScopeId::ROOT, false).unwrap_err();
        assert!(format!("{err}").contains("already provided"));
    }
}
