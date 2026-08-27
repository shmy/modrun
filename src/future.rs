use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Owned, `Send` future used for erased constructors and lifecycle hooks.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Poll independent futures on the current task. The first error drops the rest
/// (they cancel at their next `.await`) instead of aborting spawned tasks.
pub(crate) fn try_join_all<K, Fut, T, E>(items: Vec<(K, Fut)>) -> TryJoinAll<K, Fut, T, E>
where
    Fut: Future<Output = Result<T, E>> + Unpin,
{
    TryJoinAll {
        inflight: items,
        done: Vec::new(),
        _ty: std::marker::PhantomData,
    }
}

/// See [`try_join_all`].
pub(crate) struct TryJoinAll<K, Fut, T, E> {
    inflight: Vec<(K, Fut)>,
    done: Vec<(K, T)>,
    _ty: std::marker::PhantomData<E>,
}

impl<K, Fut, T, E> Future for TryJoinAll<K, Fut, T, E>
where
    K: Unpin,
    Fut: Future<Output = Result<T, E>> + Unpin,
    T: Unpin,
    E: Unpin,
{
    type Output = Result<Vec<(K, T)>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut i = 0;
        while i < this.inflight.len() {
            match Pin::new(&mut this.inflight[i].1).poll(cx) {
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
