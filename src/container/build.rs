use std::any::TypeId;

use crate::error::Result;
use crate::future::try_join_all;

use super::Container;
use super::types::{ConstructFuture, ConstructOut, Constructed, ProviderKey, TypeIdSet};
use crate::error::Error;
use crate::trace;
use crate::trace::before_run;
use crate::trace::emit_unfinished;
use crate::trace::info_enabled;
use crate::trace::run_cancelled;
use crate::trace::run_err;
use crate::trace::run_ok;
use crate::trace::run_panicked;
use std::future::Future;
use std::mem::take;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

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
            let ready = take(&mut self.wave_scratch);
            self.run_wave(ready, pending).await?;
        }

        if !pending.is_empty() {
            let name = pending
                .iter()
                .next()
                .map(|k| self.key_name(*k))
                .unwrap_or("<unknown>");
            return Err(Error::Cycle(name.to_owned()));
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
                return Err(Error::Cycle(self.key_name(key).to_owned()));
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
            let trace_info = info_enabled();
            if trace_info {
                before_run(constructor, module);
            }
            let timed = trace_info.then(Instant::now);
            let mut call = ConstructCallGuard::new(constructor, module);
            let out = guard.container.construct_at(key);
            guard.container.leave_scope(previous);
            match out {
                Err(err) => {
                    call.finish();
                    run_err(constructor, module, &err);
                    return Err(err);
                }
                Ok(ConstructOut::Ready(built)) => {
                    call.finish();
                    let elapsed = trace::elapsed(timed);
                    readies.push((key, constructor, module, built, elapsed));
                }
                Ok(ConstructOut::Fut(fut)) => {
                    call.finish();
                    futs.push((key, TracedConstruct::new(constructor, module, fut, timed)));
                }
            }
        }

        for (key, name, module, built, elapsed) in readies {
            finish_ready(name, module, elapsed);
            if key.is_group_member() {
                guard.container.store_group_member(key, built.value);
            } else {
                guard
                    .container
                    .store_constructed(key.type_id, built, key.scope, key.private);
            }
            pending.remove(&key);
        }

        let results = join_constructs(futs).await?;
        for (key, built) in results {
            if key.is_group_member() {
                guard.container.store_group_member(key, built.value);
            } else {
                guard
                    .container
                    .store_constructed(key.type_id, built, key.scope, key.private);
            }
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

fn finish_ready(name: &'static str, module: &'static str, elapsed: Duration) {
    run_ok(name, module, elapsed);
}

struct ConstructCallGuard {
    name: &'static str,
    module: &'static str,
    finished: bool,
}

impl ConstructCallGuard {
    fn new(name: &'static str, module: &'static str) -> Self {
        Self {
            name,
            module,
            finished: false,
        }
    }

    fn finish(&mut self) {
        self.finished = true;
    }
}

impl Drop for ConstructCallGuard {
    fn drop(&mut self) {
        emit_unfinished(
            self.finished,
            || run_panicked(self.name, self.module),
            || run_cancelled(self.name, self.module),
        );
    }
}

struct TracedConstruct {
    name: &'static str,
    module: &'static str,
    fut: ConstructFuture,
    timed: Option<Instant>,
    finished: bool,
}

impl TracedConstruct {
    fn new(
        name: &'static str,
        module: &'static str,
        fut: ConstructFuture,
        timed: Option<Instant>,
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

impl Future for TracedConstruct {
    type Output = Result<Constructed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this.fut.as_mut().poll(cx) {
            Poll::Ready(Ok(built)) => {
                this.finished = true;
                run_ok(this.name, this.module, trace::elapsed(this.timed));
                Poll::Ready(Ok(built))
            }
            Poll::Ready(Err(err)) => {
                this.finished = true;
                run_err(this.name, this.module, &err);
                Poll::Ready(Err(err))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for TracedConstruct {
    fn drop(&mut self) {
        emit_unfinished(
            self.finished,
            || run_panicked(self.name, self.module),
            || run_cancelled(self.name, self.module),
        );
    }
}
