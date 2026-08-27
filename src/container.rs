use std::any::{Any, TypeId, type_name};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::{Error, Result};

use crate::future::{BoxFuture, try_join_all};
use crate::lifecycle::Lifecycle;
use crate::scope::{ScopeId, ScopeTree};
use crate::shutdown::Shutdowner;

pub(crate) type DynAny = Arc<dyn Any + Send + Sync>;

/// Value produced by a constructor, plus `Arc<T>` aliases stored under their
/// own [`TypeId`] so invokers can take `Arc<T>` without `T: Clone`.
pub(crate) struct Constructed {
    pub value: DynAny,
    pub aliases: Vec<(TypeId, DynAny)>,
}

pub(crate) fn pack<T: Send + Sync + 'static>(value: T) -> Constructed {
    let arc = Arc::new(value);
    Constructed {
        value: Arc::clone(&arc) as DynAny,
        aliases: vec![(TypeId::of::<Arc<T>>(), Arc::new(arc) as DynAny)],
    }
}

/// Future produced by a provider once its dependencies have been injected.
///
/// Constructors take their dependencies synchronously from the container, then
/// hand back an owned future. That split keeps the borrow of the container short
/// so independent constructors in the same DAG layer can run concurrently.
pub(crate) type ConstructFuture = BoxFuture<'static, Result<Constructed>>;

pub(crate) trait Provider: Send + Sync {
    fn result_type(&self) -> TypeId;
    fn result_name(&self) -> &'static str;
    fn alias_types(&self) -> &[TypeId];
    fn dep_types(&self) -> &[(TypeId, &'static str)];
    fn construct(&self, container: &Container) -> Result<ConstructFuture>;
}

/// Identifies one provider: type, registration scope, and visibility.
///
/// Sibling modules may each hold a private provider for the same type, and a
/// single module may hold both a private and a public provider for the same
/// type (e.g. a decorator), so none of the three alone is enough.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ProviderKey {
    type_id: TypeId,
    scope: ScopeId,
    private: bool,
}

pub(crate) struct Container {
    scopes: ScopeTree,
    values_public: HashMap<TypeId, DynAny>,
    values_private: HashMap<(TypeId, ScopeId), DynAny>,
    providers: HashMap<ProviderKey, Arc<dyn Provider>>,
    /// Public provider keys keyed by result type *and* `Arc<T>` aliases.
    public_index: HashMap<TypeId, ProviderKey>,
    /// Private `Arc<T>` aliases → canonical private provider key.
    private_alias: HashMap<(TypeId, ScopeId), ProviderKey>,
    /// Registration order, used for stable wave ordering and cycle walks.
    provider_order: Vec<ProviderKey>,
    provider_order_index: HashMap<ProviderKey, usize>,
    constructing: HashSet<ProviderKey>,
    active_scope: ScopeId,
}

/// Clears in-flight constructing keys if a wave is cancelled or fails.
struct WaveGuard<'a> {
    container: &'a mut Container,
    keys: Vec<ProviderKey>,
}

impl Drop for WaveGuard<'_> {
    fn drop(&mut self) {
        for key in self.keys.drain(..) {
            self.container.constructing.remove(&key);
        }
    }
}

impl Container {
    pub(crate) fn new() -> Self {
        Self {
            scopes: ScopeTree::new(),
            values_public: HashMap::new(),
            values_private: HashMap::new(),
            providers: HashMap::new(),
            public_index: HashMap::new(),
            private_alias: HashMap::new(),
            provider_order: Vec::new(),
            provider_order_index: HashMap::new(),
            constructing: HashSet::new(),
            active_scope: ScopeId::ROOT,
        }
    }

    pub(crate) fn scopes(&self) -> &ScopeTree {
        &self.scopes
    }

    pub(crate) fn scopes_mut(&mut self) -> &mut ScopeTree {
        &mut self.scopes
    }

    /// Enter a scope, returning the previous one. Callers must pair this with
    /// [`Container::leave_scope`] on the error path.
    pub(crate) fn enter_scope(&mut self, scope: ScopeId) -> ScopeId {
        std::mem::replace(&mut self.active_scope, scope)
    }

    pub(crate) fn leave_scope(&mut self, previous: ScopeId) {
        self.active_scope = previous;
    }

    pub(crate) fn insert_value<T: Send + Sync + 'static>(
        &mut self,
        value: T,
        scope: ScopeId,
        private: bool,
    ) -> Result<()> {
        let packed = pack(value);
        self.ensure_absent(TypeId::of::<T>(), type_name::<T>(), scope, private)?;
        self.ensure_absent(
            TypeId::of::<Arc<T>>(),
            type_name::<Arc<T>>(),
            scope,
            private,
        )?;
        self.store_constructed(TypeId::of::<T>(), packed, scope, private);
        Ok(())
    }

    pub(crate) fn insert_provider(
        &mut self,
        provider: Arc<dyn Provider>,
        scope: ScopeId,
        private: bool,
    ) -> Result<()> {
        let id = provider.result_type();
        self.ensure_absent(id, provider.result_name(), scope, private)?;
        for &alias in provider.alias_types() {
            self.ensure_absent(alias, provider.result_name(), scope, private)?;
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
        self.provider_order_index
            .insert(key, self.provider_order.len());
        self.provider_order.push(key);
        Ok(())
    }

    fn store_constructed(&mut self, id: TypeId, built: Constructed, scope: ScopeId, private: bool) {
        self.store_value(id, built.value, scope, private);
        for (alias_id, value) in built.aliases {
            self.store_value(alias_id, value, scope, private);
        }
    }

    fn store_value(&mut self, id: TypeId, value: DynAny, scope: ScopeId, private: bool) {
        if private {
            self.values_private.insert((id, scope), value);
        } else {
            self.values_public.insert(id, value);
        }
    }

    fn ensure_absent(
        &self,
        id: TypeId,
        name: &'static str,
        scope: ScopeId,
        private: bool,
    ) -> Result<()> {
        let conflict = if private {
            self.values_private.contains_key(&(id, scope))
                || self.providers.contains_key(&ProviderKey {
                    type_id: id,
                    scope,
                    private: true,
                })
                || self.private_alias.contains_key(&(id, scope))
        } else {
            self.values_public.contains_key(&id) || self.public_index.contains_key(&id)
        };

        if !conflict {
            return Ok(());
        }

        if private {
            Err(Error::AlreadyProvidedPrivate {
                module: self.scopes.name(scope),
                type_name: name,
            })
        } else {
            Err(Error::AlreadyProvided(name))
        }
    }

    /// Ensure every root type (and its transitive unbuilt providers) is constructed.
    ///
    /// **Async** constructors in the same DAG layer are polled concurrently on this
    /// task. **Sync** constructors run inside [`Provider::construct`](Provider::construct)
    /// while the wave is being assembled, which defers creation of later futures
    /// in the same layer.
    pub(crate) async fn ensure_built(&mut self, roots: &[(TypeId, &'static str)]) -> Result<()> {
        let from = self.active_scope;
        let mut pending = HashSet::new();
        for &(id, name) in roots {
            self.collect_pending(id, name, from, &mut pending)?;
        }

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

            for &key in &ready {
                if !self.constructing.insert(key) {
                    return Err(Error::Cycle(self.key_name(key).to_owned()));
                }
            }

            let guard = WaveGuard {
                container: self,
                keys: ready,
            };

            let mut futs = Vec::with_capacity(guard.keys.len());
            for &key in &guard.keys {
                let provider = Arc::clone(
                    guard
                        .container
                        .provider_at(key)
                        .expect("pending key missing provider"),
                );
                let name = provider.result_name();
                let module = guard.container.scopes.name(key.scope);
                let previous = guard.container.enter_scope(key.scope);
                let fut = provider.construct(guard.container);
                guard.container.leave_scope(previous);
                futs.push((key, name, module, fut?));
            }

            let results = join_constructs(futs).await?;

            for (key, built) in results {
                guard
                    .container
                    .store_constructed(key.type_id, built, key.scope, key.private);
                pending.remove(&key);
            }
            drop(guard);
        }

        Ok(())
    }

    pub(crate) fn get<T: Clone + Send + Sync + 'static>(&self) -> Result<T> {
        let id = TypeId::of::<T>();
        let value = self
            .lookup_value_ref_from(id, self.active_scope)
            .ok_or_else(|| Error::NotConstructed(type_name::<T>()))?;
        downcast_clone::<T>(value)
    }

    /// Resolve a cached value using the same priority as providers: nearest
    /// private binding up the scope chain, else the public one.
    ///
    /// A nearer private *provider* that has not been built yet shadows a public
    /// value, so this returns `None` and forces a build.
    fn lookup_value_ref_from(&self, id: TypeId, from: ScopeId) -> Option<&DynAny> {
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

    /// Resolve a type to the provider that would build it when seen from `from`:
    /// the nearest private provider up the scope chain, else the public one.
    fn resolve_provider(
        &self,
        id: TypeId,
        from: ScopeId,
    ) -> Option<(ProviderKey, &Arc<dyn Provider>)> {
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

    fn provider_at(&self, key: ProviderKey) -> Option<&Arc<dyn Provider>> {
        self.providers.get(&key)
    }

    fn missing_provider(&self, name: &'static str, from: ScopeId) -> Error {
        Error::MissingProvider {
            type_name: name,
            module: self.scopes.name(from),
        }
    }

    fn collect_pending(
        &self,
        id: TypeId,
        name: &'static str,
        from: ScopeId,
        pending: &mut HashSet<ProviderKey>,
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

    fn ready_wave(&self, pending: &HashSet<ProviderKey>) -> Vec<ProviderKey> {
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

    fn order_index(&self, key: ProviderKey) -> usize {
        self.provider_order_index
            .get(&key)
            .copied()
            .unwrap_or(usize::MAX)
    }

    /// Check every registered provider (and invoker dependency list) before
    /// anything is built.
    ///
    /// Construction is lazy, so a provider nobody depends on would otherwise
    /// never be resolved and a typo in its signature would go unnoticed until
    /// some later change happened to pull it into the graph.
    pub(crate) fn validate(
        &self,
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

        self.detect_cycles()
    }

    fn can_resolve(&self, id: TypeId, from: ScopeId) -> bool {
        for scope in self.scopes.ancestors_from(from) {
            if self.values_private.contains_key(&(id, scope)) {
                return true;
            }
        }
        self.values_public.contains_key(&id) || self.resolve_provider(id, from).is_some()
    }

    /// Whether a dependency is already satisfied by a supplied/built value when
    /// resolving from `from` — used to truncate cycle detection and wave ready checks.
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
        self.values_public.contains_key(&id)
    }

    fn detect_cycles(&self) -> Result<()> {
        let mut done = HashSet::new();
        let mut on_path = HashSet::new();
        let mut path = Vec::new();
        for &key in &self.provider_order {
            self.visit(key, &mut done, &mut on_path, &mut path)?;
        }
        Ok(())
    }

    fn visit(
        &self,
        key: ProviderKey,
        done: &mut HashSet<ProviderKey>,
        on_path: &mut HashSet<ProviderKey>,
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

    fn key_name(&self, key: ProviderKey) -> &'static str {
        self.provider_at(key)
            .map(|p| p.result_name())
            .unwrap_or("<unknown>")
    }
}

async fn join_constructs(
    futs: Vec<(ProviderKey, &'static str, &'static str, ConstructFuture)>,
) -> Result<Vec<(ProviderKey, Constructed)>> {
    match futs.len() {
        0 => Ok(Vec::new()),
        1 => {
            let (key, name, module, fut) = futs.into_iter().next().expect("len checked");
            Ok(vec![(key, run_construct(name, module, fut).await?)])
        }
        _ => {
            let boxed = futs
                .into_iter()
                .map(|(key, name, module, fut)| {
                    let traced: ConstructFuture =
                        Box::pin(async move { run_construct(name, module, fut).await });
                    (key, traced)
                })
                .collect();
            try_join_all(boxed).await
        }
    }
}

async fn run_construct(
    name: &'static str,
    module: &'static str,
    fut: ConstructFuture,
) -> Result<Constructed> {
    use std::time::Instant;

    struct CancelGuard {
        name: &'static str,
        module: &'static str,
        finished: bool,
    }

    impl Drop for CancelGuard {
        fn drop(&mut self) {
            if !self.finished {
                crate::trace::run_cancelled(self.name, self.module);
            }
        }
    }

    crate::trace::before_run(name, module);
    let mut guard = CancelGuard {
        name,
        module,
        finished: false,
    };
    let started = Instant::now();
    match fut.await {
        Ok(built) => {
            guard.finished = true;
            crate::trace::run_ok(name, module, started.elapsed());
            Ok(built)
        }
        Err(err) => {
            guard.finished = true;
            crate::trace::run_err(name, module, &err);
            Err(err)
        }
    }
}

fn downcast_clone<T: Clone + Send + Sync + 'static>(value: &DynAny) -> Result<T> {
    // `DynAny` is only constructed by `pack`; a failed downcast means an internal
    // invariant broke — there is no supported way for callers to forge a bad value.
    value
        .downcast_ref::<T>()
        .cloned()
        .ok_or_else(|| Error::Downcast(type_name::<T>()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provide::ProviderFn;

    #[test]
    fn public_index_rejects_duplicate() {
        let mut c = Container::new();
        let p = Arc::new((|| 1u32).into_provider());
        c.insert_provider(p, ScopeId::ROOT, false).unwrap();
        let p2 = Arc::new((|| 2u32).into_provider());
        let err = c.insert_provider(p2, ScopeId::ROOT, false).unwrap_err();
        assert!(format!("{err}").contains("already provided"));
    }
}
