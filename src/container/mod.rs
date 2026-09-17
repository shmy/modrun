use std::any::{Any, TypeId};
use std::sync::Arc;

use crate::error::Result;
use crate::provide::DynProvider;
use crate::scope::{ScopeId, ScopeTree};
use std::mem::replace;

mod build;
mod graph;
mod group;
mod storage;
mod types;

pub(crate) use group::Groups;
pub(crate) use storage::{pack, seed_builtins};
pub(crate) use types::ConstructOut;
pub(crate) use types::{GroupElementKey, ProviderKey, TypeIdMap, TypeIdSet, ValueNode};

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
    pub(crate) groups: Groups,
    pub(crate) value_nodes: Vec<ValueNode>,
    /// `private_scopes[scope]` is `true` when at least one private binding
    /// (provider, alias, or supplied value) is registered in that scope.
    /// Lets ancestor-chain lookups skip the per-scope private probes for the
    /// common case of purely organizational nesting.
    pub(crate) private_scopes: Vec<bool>,
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
            groups: Groups::default(),
            value_nodes: Vec::new(),
            private_scopes: Vec::new(),
        }
    }

    pub(crate) fn scopes(&self) -> &ScopeTree {
        &self.scopes
    }

    pub(crate) fn scopes_mut(&mut self) -> &mut ScopeTree {
        &mut self.scopes
    }

    pub(crate) fn enter_scope(&mut self, scope: ScopeId) -> ScopeId {
        replace(&mut self.active_scope, scope)
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
        let key = ProviderKey::singleton(id, scope, private);
        if private {
            self.mark_private_scope(scope);
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
        self.provider_order_index
            .insert(key, self.provider_order.len());
        self.provider_order.push(key);
        Ok(())
    }

    pub(crate) fn provider_at(&self, key: ProviderKey) -> Option<&DynProvider> {
        self.providers.get(&key)
    }

    /// Record that `scope` holds at least one private binding.
    pub(crate) fn mark_private_scope(&mut self, scope: ScopeId) {
        let idx = scope.discriminant() as usize;
        if idx >= self.private_scopes.len() {
            self.private_scopes.resize(idx + 1, false);
        }
        self.private_scopes[idx] = true;
    }

    /// Whether `scope` has any private binding. Chains of purely organizational
    /// modules return `false` and skip the private probes during resolution.
    pub(crate) fn scope_has_private(&self, scope: ScopeId) -> bool {
        self.private_scopes
            .get(scope.discriminant() as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Remove a provider so its constructor can borrow the container mutably.
    /// The caller must return it via [`put_provider`](Self::put_provider).
    pub(crate) fn take_provider(&mut self, key: ProviderKey) -> Option<DynProvider> {
        self.providers.remove(&key)
    }

    /// Reinsert a provider taken by [`take_provider`](Self::take_provider).
    pub(crate) fn put_provider(&mut self, key: ProviderKey, provider: DynProvider) {
        self.providers.insert(key, provider);
    }

    pub(crate) fn key_name(&self, key: ProviderKey) -> &'static str {
        self.provider_at(key)
            .map(|p| {
                if key.is_group_member() {
                    p.constructor_name()
                } else {
                    p.result_name()
                }
            })
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
