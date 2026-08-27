use std::any::TypeId;

use crate::error::Result;
use crate::future::try_join_all;

use super::Container;
use super::types::{ConstructFuture, ConstructOut, Constructed, ProviderKey, TypeIdSet};

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
    pub(crate) async fn ensure_built(&mut self, roots: &[(TypeId, &'static str)]) -> Result<()> {
        let from = self.active_scope;
        let mut pending = TypeIdSet::default();
        for &(id, name) in roots {
            self.collect_pending(id, name, from, &mut pending)?;
        }

        self.build_pending(&mut pending).await
    }

    async fn build_pending(&mut self, pending: &mut TypeIdSet<ProviderKey>) -> Result<()> {
        let n = self.layers.len();
        for i in 0..n {
            if pending.is_empty() {
                break;
            }
            self.wave_scratch.clear();
            self.wave_scratch.extend(
                self.layers[i]
                    .iter()
                    .copied()
                    .filter(|key| pending.contains(key)),
            );
            if self.wave_scratch.is_empty() {
                continue;
            }
            let ready = std::mem::take(&mut self.wave_scratch);
            self.run_wave(ready, pending).await?;
        }

        if !pending.is_empty() {
            let name = pending
                .iter()
                .next()
                .map(|k| self.key_name(*k))
                .unwrap_or("<unknown>");
            return Err(crate::error::Error::Cycle(name.to_owned()));
        }
        Ok(())
    }

    async fn run_wave(
        &mut self,
        ready: Vec<ProviderKey>,
        pending: &mut TypeIdSet<ProviderKey>,
    ) -> Result<()> {
        for &key in &ready {
            if !self.constructing.insert(key) {
                return Err(crate::error::Error::Cycle(self.key_name(key).to_owned()));
            }
        }

        let guard = WaveGuard {
            container: self,
            keys: ready,
        };

        let mut futs = Vec::with_capacity(guard.keys.len());
        let mut readies = Vec::with_capacity(guard.keys.len());
        for &key in &guard.keys {
            let previous = guard.container.enter_scope(key.scope);
            let (constructor, module) = {
                let container = &*guard.container;
                let provider = container
                    .provider_at(key)
                    .expect("pending key missing provider");
                (
                    provider.constructor_name(),
                    container.scopes.name(key.scope),
                )
            };
            crate::trace::before_run(constructor, module);
            let timed = crate::trace::start_timer();
            let out = guard
                .container
                .provider_at(key)
                .expect("pending key missing provider")
                .construct(guard.container);
            guard.container.leave_scope(previous);
            match out {
                Err(err) => {
                    crate::trace::run_err(constructor, module, &err);
                    return Err(err);
                }
                Ok(ConstructOut::Ready(built)) => {
                    let elapsed = crate::trace::elapsed(timed);
                    readies.push((key, constructor, module, built, elapsed));
                }
                Ok(ConstructOut::Fut(fut)) => {
                    futs.push((key, TracedConstruct::new(constructor, module, fut, timed)));
                }
            }
        }

        for (key, name, module, built, elapsed) in readies {
            finish_ready(name, module, elapsed);
            guard
                .container
                .store_constructed(key.type_id, built, key.scope, key.private);
            pending.remove(&key);
        }

        let results = join_constructs(futs).await?;
        for (key, built) in results {
            guard
                .container
                .store_constructed(key.type_id, built, key.scope, key.private);
            pending.remove(&key);
        }
        drop(guard);
        Ok(())
    }
}

async fn join_constructs(
    futs: Vec<(ProviderKey, TracedConstruct)>,
) -> Result<Vec<(ProviderKey, Constructed)>> {
    match futs.len() {
        0 => Ok(Vec::new()),
        1 => {
            let (key, fut) = futs.into_iter().next().expect("len checked");
            Ok(vec![(key, fut.await?)])
        }
        _ => try_join_all(futs).await,
    }
}

fn finish_ready(name: &'static str, module: &'static str, elapsed: std::time::Duration) {
    crate::trace::run_ok(name, module, elapsed);
}

struct TracedConstruct {
    name: &'static str,
    module: &'static str,
    fut: ConstructFuture,
    timed: Option<std::time::Instant>,
    finished: bool,
}

impl TracedConstruct {
    fn new(
        name: &'static str,
        module: &'static str,
        fut: ConstructFuture,
        timed: Option<std::time::Instant>,
    ) -> Self {
        Self {
            name,
            module,
            fut,
            timed,
            finished: false,
        }
    }
}

impl std::future::Future for TracedConstruct {
    type Output = Result<Constructed>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        match this.fut.as_mut().poll(cx) {
            std::task::Poll::Ready(Ok(built)) => {
                this.finished = true;
                crate::trace::run_ok(this.name, this.module, crate::trace::elapsed(this.timed));
                std::task::Poll::Ready(Ok(built))
            }
            std::task::Poll::Ready(Err(err)) => {
                this.finished = true;
                crate::trace::run_err(this.name, this.module, &err);
                std::task::Poll::Ready(Err(err))
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl Drop for TracedConstruct {
    fn drop(&mut self) {
        if !self.finished {
            crate::trace::run_cancelled(self.name, self.module);
        }
    }
}
