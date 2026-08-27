use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Owned, `Send` future used for erased constructors and lifecycle hooks.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Poll independent futures on the current task. The first error drops the rest
/// (they cancel at their next `.await`) instead of aborting spawned tasks.
pub(crate) fn try_join_all<K, T, E>(
    items: Vec<(K, BoxFuture<'static, Result<T, E>>)>,
) -> TryJoinAll<K, T, E>
where
    K: Unpin,
    T: Unpin,
    E: Unpin,
{
    TryJoinAll {
        inflight: items,
        done: Vec::new(),
    }
}

/// See [`try_join_all`].
pub(crate) struct TryJoinAll<K, T, E> {
    inflight: Vec<(K, BoxFuture<'static, Result<T, E>>)>,
    done: Vec<(K, T)>,
}

impl<K, T, E> Future for TryJoinAll<K, T, E>
where
    K: Unpin,
    T: Unpin,
    E: Unpin,
{
    type Output = Result<Vec<(K, T)>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut i = 0;
        while i < this.inflight.len() {
            match this.inflight[i].1.as_mut().poll(cx) {
                Poll::Ready(Ok(value)) => {
                    let (key, _) = this.inflight.swap_remove(i);
                    this.done.push((key, value));
                }
                Poll::Ready(Err(err)) => {
                    this.inflight.clear();
                    return Poll::Ready(Err(err));
                }
                Poll::Pending => i += 1,
            }
        }
        if this.inflight.is_empty() {
            Poll::Ready(Ok(std::mem::take(&mut this.done)))
        } else {
            Poll::Pending
        }
    }
}
