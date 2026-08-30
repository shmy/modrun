use std::any::TypeId;

use crate::error::{Error, Result};
use crate::provide::DynProvider;
use crate::scope::ScopeId;

use super::types::{GroupElementKey, ProviderKey, TypeIdMap, TypeIdSet};
use super::{Container, DynAny};

impl Container {
    /// Resolve a cached value using the same priority as providers: nearest
    /// private binding up the scope chain, else the public one.
    pub(crate) fn lookup_value_ref_from(&self, id: TypeId, from: ScopeId) -> Option<&DynAny> {
        for scope in self.scopes.ancestors_from(from) {
            if let Some(v) = self.values_private.get(&(id, scope)) {
                return Some(v);
            }
            if self
                .providers
                .contains_key(&ProviderKey::singleton(id, scope, true))
            {
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
            let key = ProviderKey::singleton(id, scope, true);
            if let Some(p) = self.providers.get(&key) {
                return Some((key, p));
            }
            if let Some(&canon) = self.private_alias.get(&(id, scope)) {
                return self.providers.get(&canon).map(|p| (canon, p));
            }
        }

        if self.is_group_type(id) {
            let key = self.group_virtual_key(id)?;
            return self.providers.get(&key).map(|p| (key, p));
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

        if self.group_virtual_key(id) == Some(key) {
            return self.collect_group_pending(id, name, pending);
        }

        self.collect_provider_pending(key, provider, pending)
    }

    fn collect_group_pending(
        &self,
        group_type: TypeId,
        name: &'static str,
        pending: &mut TypeIdSet<ProviderKey>,
    ) -> Result<()> {
        let virtual_key = self
            .group_virtual_key(group_type)
            .ok_or_else(|| self.missing_provider(name, ScopeId::ROOT))?;
        if !pending.insert(virtual_key) {
            return Ok(());
        }
        let element = self
            .group_element_type(group_type)
            .expect("group type missing element mapping");
        for &member_key in self
            .group_members
            .get(&GroupElementKey { element })
            .into_iter()
            .flatten()
        {
            let provider = self
                .provider_at(member_key)
                .expect("group member missing provider");
            self.collect_provider_pending(member_key, provider, pending)?;
        }
        Ok(())
    }

    fn collect_provider_pending(
        &self,
        key: ProviderKey,
        provider: &DynProvider,
        pending: &mut TypeIdSet<ProviderKey>,
    ) -> Result<()> {
        if self.constructing.contains(&key) {
            return Err(Error::Cycle(self.key_name(key).to_owned()));
        }

        if !pending.insert(key) {
            return Ok(());
        }

        for &(dep_id, dep_name) in provider.dep_types() {
            self.collect_pending(dep_id, dep_name, key.scope, pending)?;
        }
        Ok(())
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

        self.validate_required_groups()?;
        self.detect_cycles()?;
        self.freeze_layers()
    }

    fn validate_required_groups(&self) -> Result<()> {
        for (&element, &type_name) in &self.required_groups {
            let members = self
                .group_members
                .get(&GroupElementKey { element })
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if members.is_empty() {
                return Err(Error::EmptyGroup { type_name });
            }
        }
        Ok(())
    }

    fn freeze_layers(&mut self) -> Result<()> {
        let mut indegree = TypeIdMap::default();
        let mut dependents: TypeIdMap<ProviderKey, Vec<ProviderKey>> = TypeIdMap::default();
        for &key in &self.provider_order {
            let Some(provider) = self.provider_at(key) else {
                continue;
            };
            let mut degree = 0usize;
            for &(dep_id, _) in provider.dep_types() {
                if self.value_satisfies(dep_id, key.scope) {
                    continue;
                }
                if let Some((dep_key, _)) = self.resolve_provider(dep_id, key.scope) {
                    degree += 1;
                    dependents.entry(dep_key).or_default().push(key);
                }
            }
            indegree.insert(key, degree);
        }

        self.add_group_virtual_edges(&mut indegree, &mut dependents);

        let mut ready: Vec<_> = self
            .provider_order
            .iter()
            .copied()
            .filter(|key| indegree.get(key) == Some(&0))
            .collect();
        ready.sort_by_key(|key| self.order_index(*key));
        let mut layers = Vec::new();
        let mut built = 0usize;
        while !ready.is_empty() {
            built += ready.len();
            let mut next = Vec::new();
            for key in &ready {
                for dependent in dependents.get(key).into_iter().flatten() {
                    let degree = indegree
                        .get_mut(dependent)
                        .expect("dependent missing indegree");
                    *degree -= 1;
                    if *degree == 0 {
                        next.push(*dependent);
                    }
                }
            }
            layers.push(ready);
            next.sort_by_key(|key| self.order_index(*key));
            ready = next;
        }

        if built != self.provider_order.len() {
            let key = self
                .provider_order
                .iter()
                .copied()
                .find(|key| indegree.get(key).is_some_and(|degree| *degree > 0))
                .expect("unbuilt provider missing positive indegree");
            return Err(self.cycle_error(key, &[]));
        }
        self.layers = layers;
        Ok(())
    }

    fn add_group_virtual_edges(
        &self,
        indegree: &mut TypeIdMap<ProviderKey, usize>,
        dependents: &mut TypeIdMap<ProviderKey, Vec<ProviderKey>>,
    ) {
        for reg in self.group_registrations.values() {
            let virtual_key = reg.virtual_key;
            let members = self
                .group_members
                .get(&GroupElementKey {
                    element: reg.element,
                })
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for &member_key in members {
                *indegree.entry(virtual_key).or_insert(0) += 1;
                dependents.entry(member_key).or_default().push(virtual_key);
            }
        }
    }

    fn can_resolve(&self, id: TypeId, from: ScopeId) -> bool {
        for scope in self.scopes.ancestors_from(from) {
            if self.values_private.contains_key(&(id, scope)) {
                return true;
            }
        }
        self.values_public.contains_key(&id) || self.resolve_provider(id, from).is_some()
    }

    fn value_satisfies(&self, id: TypeId, from: ScopeId) -> bool {
        for scope in self.scopes.ancestors_from(from) {
            if self.values_private.contains_key(&(id, scope)) {
                return true;
            }
            if self
                .providers
                .contains_key(&ProviderKey::singleton(id, scope, true))
                || self.private_alias.contains_key(&(id, scope))
            {
                return false;
            }
        }
        self.values_public.contains_key(&id)
    }

    fn detect_cycles(&self) -> Result<()> {
        let mut done = TypeIdSet::default();
        let mut on_path = TypeIdSet::default();
        let mut path = Vec::new();
        for &key in &self.provider_order {
            if key.is_group_member() {
                continue;
            }
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

        if let Some(&element) = self.group_virtual_to_element.get(&key) {
            for &member_key in self
                .group_members
                .get(&GroupElementKey { element })
                .into_iter()
                .flatten()
            {
                self.visit(member_key, done, on_path, path)?;
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
