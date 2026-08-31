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
        let mut can_resolve = TypeIdMap::<(TypeId, ScopeId), bool>::default();
        let mut value_satisfies = TypeIdMap::<(TypeId, ScopeId), bool>::default();
        let mut provider_keys = TypeIdMap::<(TypeId, ScopeId), Option<ProviderKey>>::default();

        for &key in &self.provider_order {
            let Some(provider) = self.provider_at(key) else {
                continue;
            };
            for &(dep_id, dep_name) in provider.dep_types() {
                if self.can_resolve_cached(dep_id, key.scope, &mut can_resolve) {
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
                if self.can_resolve_cached(dep_id, scope, &mut can_resolve) {
                    continue;
                }
                return Err(Error::InvokerMissingDep {
                    module: self.scopes.name(scope),
                    dependency: dep_name,
                });
            }
        }

        self.validate_required_groups()?;
        match self.freeze_layers(&mut value_satisfies, &mut provider_keys) {
            Ok(()) => Ok(()),
            Err(Error::Cycle(_)) => self.detect_cycles(),
            Err(err) => Err(err),
        }
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

    fn freeze_layers(
        &mut self,
        value_satisfies: &mut TypeIdMap<(TypeId, ScopeId), bool>,
        provider_keys: &mut TypeIdMap<(TypeId, ScopeId), Option<ProviderKey>>,
    ) -> Result<()> {
        let mut indegree = TypeIdMap::default();
        let mut dependents: TypeIdMap<ProviderKey, Vec<ProviderKey>> = TypeIdMap::default();
        for &key in &self.provider_order {
            let Some(provider) = self.provider_at(key) else {
                continue;
            };
            let mut degree = 0usize;
            for &(dep_id, _) in provider.dep_types() {
                if self.value_satisfies_cached(dep_id, key.scope, value_satisfies) {
                    continue;
                }
                if let Some(dep_key) =
                    self.resolve_provider_key_cached(dep_id, key.scope, provider_keys)
                {
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

    fn can_resolve_cached(
        &self,
        id: TypeId,
        from: ScopeId,
        cache: &mut TypeIdMap<(TypeId, ScopeId), bool>,
    ) -> bool {
        if let Some(&hit) = cache.get(&(id, from)) {
            return hit;
        }
        let hit = self.can_resolve(id, from);
        cache.insert((id, from), hit);
        hit
    }

    fn resolve_provider_key(&self, id: TypeId, from: ScopeId) -> Option<ProviderKey> {
        self.resolve_provider(id, from).map(|(key, _)| key)
    }

    fn resolve_provider_key_cached(
        &self,
        id: TypeId,
        from: ScopeId,
        cache: &mut TypeIdMap<(TypeId, ScopeId), Option<ProviderKey>>,
    ) -> Option<ProviderKey> {
        if let Some(hit) = cache.get(&(id, from)) {
            return *hit;
        }
        let hit = self.resolve_provider_key(id, from);
        cache.insert((id, from), hit);
        hit
    }

    fn value_satisfies_cached(
        &self,
        id: TypeId,
        from: ScopeId,
        cache: &mut TypeIdMap<(TypeId, ScopeId), bool>,
    ) -> bool {
        if let Some(&hit) = cache.get(&(id, from)) {
            return hit;
        }
        let hit = self.value_satisfies(id, from);
        cache.insert((id, from), hit);
        hit
    }

    fn value_satisfies(&self, id: TypeId, from: ScopeId) -> bool {
        self.resolve_value_binding(id, from).is_some()
    }

    /// Where a pre-built value would be resolved from, mirroring [`value_satisfies`].
    pub(crate) fn resolve_value_binding(
        &self,
        id: TypeId,
        from: ScopeId,
    ) -> Option<(ScopeId, bool)> {
        for scope in self.scopes.ancestors_from(from) {
            if self.values_private.contains_key(&(id, scope)) {
                return Some((scope, true));
            }
            if self
                .providers
                .contains_key(&ProviderKey::singleton(id, scope, true))
                || self.private_alias.contains_key(&(id, scope))
            {
                return None;
            }
        }
        if self.values_public.contains_key(&id) {
            if let Some(node) = self
                .value_nodes
                .iter()
                .find(|node| node.type_id == id && !node.private)
            {
                return Some((node.scope, false));
            }
            return Some((ScopeId::ROOT, false));
        }
        None
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

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::Container;
    use crate::scope::ScopeId;

    #[test]
    fn resolve_value_binding_uses_public_supply_module_scope() {
        let mut container = Container::new();
        let settings = container.scopes_mut().child(ScopeId::ROOT, "settings");
        container.insert_value(42u32, settings, false).unwrap();

        assert_eq!(
            container.resolve_value_binding(TypeId::of::<u32>(), settings),
            Some((settings, false))
        );
        assert_eq!(
            container.resolve_value_binding(TypeId::of::<u32>(), ScopeId::ROOT),
            Some((settings, false))
        );
    }

    #[test]
    fn resolve_value_binding_falls_back_to_root_without_value_node_metadata() {
        let mut container = Container::new();
        container.insert_value(7u32, ScopeId::ROOT, false).unwrap();
        container.value_nodes.clear();

        assert_eq!(
            container.resolve_value_binding(TypeId::of::<u32>(), ScopeId::ROOT),
            Some((ScopeId::ROOT, false))
        );
    }

    #[test]
    fn resolve_value_binding_uses_private_supply_scope() {
        #[derive(Clone)]
        struct Token;

        let mut container = Container::new();
        let auth = container.scopes_mut().child(ScopeId::ROOT, "auth");
        container.insert_value(Token, auth, true).unwrap();

        assert_eq!(
            container.resolve_value_binding(TypeId::of::<Token>(), auth),
            Some((auth, true))
        );
        assert_eq!(
            container.resolve_value_binding(TypeId::of::<Token>(), ScopeId::ROOT),
            None
        );
    }
}
