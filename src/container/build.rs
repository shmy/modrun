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

        let mut futs = Vec::new();
        let mut readies = Vec::new();
        for &key in &guard.keys {
            let previous = guard.container.enter_scope(key.scope);
            let timed = crate::trace::start_timer();
            let (name, module, out) = {
                let container = &*guard.container;
                let provider = container
                    .provider_at(key)
                    .expect("pending key missing provider");
                let name = provider.result_name();
                let module = container.scopes.name(key.scope);
                let out = provider.construct(container);
                (name, module, out)
            };
            guard.container.leave_scope(previous);
            match out? {
                ConstructOut::Ready(built) => {
                    let elapsed = crate::trace::elapsed(timed);
                    readies.push((key, name, module, built, elapsed));
                }
                ConstructOut::Fut(fut) => futs.push((key, name, module, fut)),
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
    futs: Vec<(ProviderKey, &'static str, &'static str, ConstructFuture)>,
) -> Result<Vec<(ProviderKey, Constructed)>> {
    match futs.len() {
        0 => Ok(Vec::new()),
        1 => {
            let (key, name, module, fut) = futs.into_iter().next().expect("len checked");
            Ok(vec![(key, run_construct(name, module, fut).await?)])
        }
        _ => {
            let items = futs
                .into_iter()
                .map(|(key, name, module, fut)| (key, TracedConstruct::new(name, module, fut)))
                .collect();
            try_join_all(items).await
        }
    }
}

fn finish_ready(name: &'static str, module: &'static str, elapsed: std::time::Duration) {
    crate::trace::before_run(name, module);
    crate::trace::run_ok(name, module, elapsed);
}

struct TracedConstruct {
    name: &'static str,
    module: &'static str,
    fut: ConstructFuture,
    timed: Option<std::time::Instant>,
    began: bool,
    finished: bool,
}

impl TracedConstruct {
    fn new(name: &'static str, module: &'static str, fut: ConstructFuture) -> Self {
        Self {
            name,
            module,
            fut,
            timed: None,
            began: false,
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
        if !this.began {
            this.began = true;
            crate::trace::before_run(this.name, this.module);
            this.timed = crate::trace::start_timer();
        }
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
        if self.began && !self.finished {
            crate::trace::run_cancelled(self.name, self.module);
        }
    }
}

async fn run_construct(
    name: &'static str,
    module: &'static str,
    fut: ConstructFuture,
) -> Result<Constructed> {
    TracedConstruct::new(name, module, fut).await
}
