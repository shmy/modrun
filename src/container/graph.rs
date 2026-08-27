use std::any::TypeId;

use crate::error::{Error, Result};
use crate::provide::DynProvider;
use crate::scope::ScopeId;

use super::types::{ProviderKey, TypeIdSet};
use super::{Container, DynAny};

impl Container {
    /// Resolve a cached value using the same priority as providers: nearest
    /// private binding up the scope chain, else the public one.
    pub(crate) fn lookup_value_ref_from(&self, id: TypeId, from: ScopeId) -> Option<&DynAny> {
        for scope in self.scopes.ancestors_from(from) {
            if let Some(v) = self.values_private.get(&(id, scope)) {
                return Some(v);
            }
            if self.providers.contains_key(&ProviderKey {
                type_id: id,
                scope,
                private: true,
            }) {
                return None;
            }
            if self.private_alias.contains_key(&(id, scope)) {
                return None;
            }
        }
        self.values_public.get(&id)
    }

    pub(crate) fn resolve_provider(
        &self,
        id: TypeId,
        from: ScopeId,
    ) -> Option<(ProviderKey, &DynProvider)> {
        for scope in self.scopes.ancestors_from(from) {
            let key = ProviderKey {
                type_id: id,
                scope,
                private: true,
            };
            if let Some(p) = self.providers.get(&key) {
                return Some((key, p));
            }
            if let Some(&canon) = self.private_alias.get(&(id, scope)) {
                return self.providers.get(&canon).map(|p| (canon, p));
            }
        }

        let key = *self.public_index.get(&id)?;
        self.providers.get(&key).map(|p| (key, p))
    }

    pub(crate) fn missing_provider(&self, name: &'static str, from: ScopeId) -> Error {
        Error::MissingProvider {
            type_name: name,
            module: self.scopes.name(from),
        }
    }

    pub(crate) fn collect_pending(
        &self,
        id: TypeId,
        name: &'static str,
        from: ScopeId,
        pending: &mut TypeIdSet<ProviderKey>,
    ) -> Result<()> {
        if self.lookup_value_ref_from(id, from).is_some() {
            return Ok(());
        }

        let (key, provider) = self
            .resolve_provider(id, from)
            .ok_or_else(|| self.missing_provider(name, from))?;

        if self.constructing.contains(&key) {
            return Err(Error::Cycle(name.to_owned()));
        }

        if !pending.insert(key) {
            return Ok(());
        }

        for &(dep_id, dep_name) in provider.dep_types() {
            self.collect_pending(dep_id, dep_name, key.scope, pending)?;
        }
        Ok(())
    }

    pub(crate) fn ready_wave(&self, pending: &TypeIdSet<ProviderKey>) -> Vec<ProviderKey> {
        let mut ready: Vec<_> = pending
            .iter()
            .copied()
            .filter(|&key| {
                let Some(provider) = self.provider_at(key) else {
                    return false;
                };
                provider.dep_types().iter().all(|&(dep_id, _)| {
                    if self.value_satisfies(dep_id, key.scope) {
                        return true;
                    }
                    match self.resolve_provider(dep_id, key.scope) {
                        Some((dep_key, _)) => !pending.contains(&dep_key),
                        None => false,
                    }
                })
            })
            .collect();
        ready.sort_by_key(|k| self.order_index(*k));
        ready
    }

    pub(crate) fn validate(
        &mut self,
        invoker_deps: &[(ScopeId, &[(TypeId, &'static str)])],
    ) -> Result<()> {
        for &key in &self.provider_order {
            let Some(provider) = self.provider_at(key) else {
                continue;
            };
            for &(dep_id, dep_name) in provider.dep_types() {
                if self.can_resolve(dep_id, key.scope) {
                    continue;
                }
                return Err(Error::ProviderMissingDep {
                    provider: provider.result_name(),
                    module: self.scopes.name(key.scope),
                    dependency: dep_name,
                });
            }
        }

        for &(scope, deps) in invoker_deps {
            for &(dep_id, dep_name) in deps {
                if self.can_resolve(dep_id, scope) {
                    continue;
                }
                return Err(Error::InvokerMissingDep {
                    module: self.scopes.name(scope),
                    dependency: dep_name,
                });
            }
        }

        self.detect_cycles()?;
        self.freeze_layers()
    }

    fn freeze_layers(&mut self) -> Result<()> {
        let mut pending: TypeIdSet<ProviderKey> = self.provider_order.iter().copied().collect();
        let mut layers = Vec::new();
        while !pending.is_empty() {
            let ready = self.ready_wave(&pending);
            if ready.is_empty() {
                let name = pending
                    .iter()
                    .next()
                    .map(|k| self.key_name(*k))
                    .unwrap_or("<unknown>");
                return Err(Error::Cycle(name.to_owned()));
            }
            for key in &ready {
                pending.remove(key);
            }
            layers.push(ready);
        }
        self.layers = layers;
        Ok(())
    }

    fn can_resolve(&self, id: TypeId, from: ScopeId) -> bool {
        for scope in self.scopes.ancestors_from(from) {
            if self.values_private.contains_key(&(id, scope)) {
                return true;
            }
        }
        self.values_public.contains_key(&id)
            || self.arc_resolvers.contains_key(&id)
            || self.resolve_provider(id, from).is_some()
    }

    fn value_satisfies(&self, id: TypeId, from: ScopeId) -> bool {
        for scope in self.scopes.ancestors_from(from) {
            if self.values_private.contains_key(&(id, scope)) {
                return true;
            }
            if self.providers.contains_key(&ProviderKey {
                type_id: id,
                scope,
                private: true,
            }) || self.private_alias.contains_key(&(id, scope))
            {
                return false;
            }
        }
        self.values_public.contains_key(&id) || self.arc_resolvers.contains_key(&id)
    }

    fn detect_cycles(&self) -> Result<()> {
        let mut done = TypeIdSet::default();
        let mut on_path = TypeIdSet::default();
        let mut path = Vec::new();
        for &key in &self.provider_order {
            self.visit(key, &mut done, &mut on_path, &mut path)?;
        }
        Ok(())
    }

    fn visit(
        &self,
        key: ProviderKey,
        done: &mut TypeIdSet<ProviderKey>,
        on_path: &mut TypeIdSet<ProviderKey>,
        path: &mut Vec<ProviderKey>,
    ) -> Result<()> {
        if done.contains(&key) {
            return Ok(());
        }
        if !on_path.insert(key) {
            return Err(self.cycle_error(key, path));
        }

        let Some(provider) = self.provider_at(key) else {
            on_path.remove(&key);
            return Ok(());
        };

        path.push(key);
        let scope = key.scope;
        for &(dep_id, _) in provider.dep_types() {
            if self.value_satisfies(dep_id, scope) {
                continue;
            }
            if let Some((dep_key, _)) = self.resolve_provider(dep_id, scope) {
                self.visit(dep_key, done, on_path, path)?;
            }
        }
        path.pop();
        on_path.remove(&key);
        done.insert(key);
        Ok(())
    }

    fn cycle_error(&self, key: ProviderKey, path: &[ProviderKey]) -> Error {
        let start = path.iter().position(|k| *k == key).unwrap_or(0);
        let mut names: Vec<&str> = path[start..].iter().map(|k| self.key_name(*k)).collect();
        names.push(self.key_name(key));
        Error::Cycle(names.join(" -> "))
    }
}
