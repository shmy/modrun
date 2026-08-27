use std::any::{TypeId, type_name};
use std::future::Future;
use std::marker::PhantomData;

use crate::error::Result;

use crate::app::BuildState;
use crate::container::Container;
use crate::deps::DepList;
use crate::error::user_invoke_err;
use crate::future::BoxFuture;
use crate::option::ModOption;
use crate::scope::ScopeId;

pub(crate) enum InvokeOut {
    Done(Result<()>),
    Fut(BoxFuture<'static, Result<()>>),
}

pub(crate) fn invoke<M, F>(func: F) -> Box<dyn ModOption>
where
    F: InvokeFn<M> + 'static,
{
    invoke_dyn(func.into_invoke())
}

pub(crate) fn invoke_dyn(invoker: DynInvoker) -> Box<dyn ModOption> {
    Box::new(InvokeOption { invoker })
}

pub(crate) trait Invoker: Send {
    fn name(&self) -> &'static str;
    fn dep_types(&self) -> &[(TypeId, &'static str)];
    fn dep_list(&self) -> DepList;
    /// Run after dependencies have been built. Sync invokers return [`InvokeOut::Done`].
    fn call(self: Box<Self>, container: &Container) -> InvokeOut;
}

/// Invoker bound to the module scope where it was registered.
pub(crate) struct ScopedInvoker {
    pub(crate) scope: ScopeId,
    pub(crate) invoker: DynInvoker,
}

struct InvokeOption {
    invoker: DynInvoker,
}

impl ModOption for InvokeOption {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        app.invokers.push(ScopedInvoker {
            scope: app.current_scope,
            invoker: self.invoker,
        });
        Ok(())
    }
}

/// Bound for functions accepted by [`ModrunBuilder::invoke`](crate::ModrunBuilder::invoke).
///
/// Implemented for any `FnOnce(A, B, ..)` of up to **eight** `Clone` arguments, returning
/// either `()` or `Result<(), E>`. `Marker` distinguishes those shapes and is
/// always inferred.
pub trait InvokeFn<Marker>: Sized {
    /// Erase the function's signature into an invoker the container can call.
    fn into_invoke(self) -> DynInvoker;
}

/// An invoker with its signature erased, ready to be registered via
/// [`ModrunBuilder::invoke_dyn`](crate::ModrunBuilder::invoke_dyn).
pub struct DynInvoker {
    inner: Box<dyn Invoker>,
}

impl std::fmt::Debug for DynInvoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynInvoker")
            .field("name", &self.inner.name())
            .field(
                "deps",
                &self
                    .inner
                    .dep_types()
                    .iter()
                    .map(|(_, n)| *n)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl DynInvoker {
    /// Rust type name of the registered invoker function.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// Dependency types this invoker will resolve.
    #[must_use]
    pub fn dep_types(&self) -> &[(TypeId, &'static str)] {
        self.inner.dep_types()
    }

    pub(crate) fn dep_list(&self) -> DepList {
        self.inner.dep_list()
    }

    pub(crate) fn call(self, container: &Container) -> InvokeOut {
        self.inner.call(container)
    }
}

macro_rules! impl_invoke_zero {
    ($marker:ident, $ok:ident, $fallible:ident) => {
        #[doc(hidden)]
        pub struct $marker<Out>(PhantomData<fn() -> Out>);

        struct $ok<F> {
            func: Option<F>,
            name: &'static str,
        }

        impl<F> Invoker for $ok<F>
        where
            F: FnOnce() + Send + 'static,
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                &[]
            }

            fn dep_list(&self) -> DepList {
                DepList::empty()
            }

            fn call(mut self: Box<Self>, _container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                InvokeOut::Done({
                    func();
                    Ok(())
                })
            }
        }

        struct $fallible<F, ErrTy> {
            func: Option<F>,
            name: &'static str,
            _err: PhantomData<ErrTy>,
        }

        impl<F, ErrTy> Invoker for $fallible<F, ErrTy>
        where
            F: FnOnce() -> std::result::Result<(), ErrTy> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                &[]
            }

            fn dep_list(&self) -> DepList {
                DepList::empty()
            }

            fn call(mut self: Box<Self>, _container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                InvokeOut::Done(func().map_err(|err| user_invoke_err(self.name, err)))
            }
        }

        impl<Func> InvokeFn<$marker<()>> for Func
        where
            Func: FnOnce() + Send + 'static,
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($ok {
                        func: Some(self),
                        name: type_name::<Func>(),
                    }),
                }
            }
        }

        impl<Func, ErrTy> InvokeFn<$marker<std::result::Result<(), ErrTy>>> for Func
        where
            Func: FnOnce() -> std::result::Result<(), ErrTy> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($fallible {
                        func: Some(self),
                        name: type_name::<Func>(),
                        _err: PhantomData,
                    }),
                }
            }
        }
    };
}

macro_rules! impl_invoke_fn {
    ($marker:ident, $ok:ident, $fallible:ident, $($A:ident),+) => {
        #[doc(hidden)]
        pub struct $marker <Out, $($A),+>(PhantomData<fn() -> (Out, $($A,)+)>);

        struct $ok<FuncTy, $($A,)+> {
            deps: DepList,
            func: Option<FuncTy>,
            name: &'static str,
            _args: PhantomData<fn($($A),+)>,
        }

        impl<FuncTy, $($A,)+> Invoker for $ok<FuncTy, $($A,)+>
        where
            FuncTy: FnOnce($($A),+) + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                self.deps.as_slice()
            }

            fn dep_list(&self) -> DepList {
                self.deps
            }

            fn call(mut self: Box<Self>, container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                InvokeOut::Done((|| {
                    func(
                        $(container.get::<$A>()?,)+
                    );
                    Ok(())
                })())
            }
        }

        struct $fallible<FuncTy, ErrTy, $($A,)+> {
            deps: DepList,
            func: Option<FuncTy>,
            name: &'static str,
            _args: PhantomData<(ErrTy, fn($($A),+))>,
        }

        impl<FuncTy, ErrTy, $($A,)+> Invoker for $fallible<FuncTy, ErrTy, $($A,)+>
        where
            FuncTy: FnOnce($($A),+) -> std::result::Result<(), ErrTy> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                self.deps.as_slice()
            }

            fn dep_list(&self) -> DepList {
                self.deps
            }

            fn call(mut self: Box<Self>, container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                InvokeOut::Done((|| {
                    func(
                        $(container.get::<$A>()?,)+
                    )
                    .map_err(|err| user_invoke_err(self.name, err))
                })())
            }
        }

        impl<Func, $($A),+> InvokeFn<$marker<(), $($A),+>> for Func
        where
            Func: FnOnce($($A),+) + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($ok {
                        deps: DepList::from_array([$(crate::deps::dep::<$A>(),)+]),
                        func: Some(self),
                        name: type_name::<Func>(),
                        _args: PhantomData,
                    }),
                }
            }
        }

        impl<Func, ErrTy, $($A),+> InvokeFn<$marker<std::result::Result<(), ErrTy>, $($A),+>> for Func
        where
            Func: FnOnce($($A),+) -> std::result::Result<(), ErrTy> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($fallible {
                        deps: DepList::from_array([$(crate::deps::dep::<$A>(),)+]),
                        func: Some(self),
                        name: type_name::<Func>(),
                        _args: PhantomData,
                    }),
                }
            }
        }
    };
}

impl_invoke_zero!(Invoke0, Invoke0Ok, Invoke0Fallible);
impl_invoke_fn!(Invoke1, Invoke1Ok, Invoke1Fallible, A);
impl_invoke_fn!(Invoke2, Invoke2Ok, Invoke2Fallible, A, B);
impl_invoke_fn!(Invoke3, Invoke3Ok, Invoke3Fallible, A, B, C);
impl_invoke_fn!(Invoke4, Invoke4Ok, Invoke4Fallible, A, B, C, D);
impl_invoke_fn!(Invoke5, Invoke5Ok, Invoke5Fallible, A, B, C, D, E);
impl_invoke_fn!(Invoke6, Invoke6Ok, Invoke6Fallible, A, B, C, D, E, F);
impl_invoke_fn!(Invoke7, Invoke7Ok, Invoke7Fallible, A, B, C, D, E, F, G);
impl_invoke_fn!(Invoke8, Invoke8Ok, Invoke8Fallible, A, B, C, D, E, F, G, H);

pub(crate) fn invoke_async<M, F>(func: F) -> Box<dyn ModOption>
where
    F: AsyncInvokeFn<M> + 'static,
{
    invoke_dyn(func.into_invoke())
}

/// Bound for `async` functions accepted by
/// [`ModrunBuilder::invoke_async`](crate::ModrunBuilder::invoke_async).
///
/// Implemented for any `async fn(A, B, ..)` of up to **eight** `Clone` arguments,
/// returning either `()` or `Result<(), E>`.
pub trait AsyncInvokeFn<Marker>: Sized {
    /// Erase the function's signature into an invoker the container can call.
    fn into_invoke(self) -> DynInvoker;
}

macro_rules! impl_async_invoke_zero {
    ($marker:ident, $ok:ident, $fallible:ident) => {
        #[doc(hidden)]
        pub struct $marker<Out>(PhantomData<fn() -> Out>);

        struct $ok<F> {
            func: Option<F>,
            name: &'static str,
        }

        impl<F, Fut> Invoker for $ok<F>
        where
            F: FnOnce() -> Fut + Send + 'static,
            Fut: Future<Output = ()> + Send + 'static,
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                &[]
            }

            fn dep_list(&self) -> DepList {
                DepList::empty()
            }

            fn call(mut self: Box<Self>, _container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                InvokeOut::Fut(Box::pin(async move {
                    func().await;
                    Ok(())
                }))
            }
        }

        struct $fallible<F, ErrTy> {
            func: Option<F>,
            name: &'static str,
            _err: PhantomData<ErrTy>,
        }

        impl<F, Fut, ErrTy> Invoker for $fallible<F, ErrTy>
        where
            F: FnOnce() -> Fut + Send + 'static,
            Fut: Future<Output = std::result::Result<(), ErrTy>> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                &[]
            }

            fn dep_list(&self) -> DepList {
                DepList::empty()
            }

            fn call(mut self: Box<Self>, _container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                let name = self.name;
                InvokeOut::Fut(Box::pin(async move {
                    func().await.map_err(|err| user_invoke_err(name, err))
                }))
            }
        }

        impl<Func, Fut> AsyncInvokeFn<$marker<()>> for Func
        where
            Func: FnOnce() -> Fut + Send + 'static,
            Fut: Future<Output = ()> + Send + 'static,
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($ok {
                        func: Some(self),
                        name: type_name::<Func>(),
                    }),
                }
            }
        }

        impl<Func, Fut, ErrTy> AsyncInvokeFn<$marker<std::result::Result<(), ErrTy>>> for Func
        where
            Func: FnOnce() -> Fut + Send + 'static,
            Fut: Future<Output = std::result::Result<(), ErrTy>> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($fallible {
                        func: Some(self),
                        name: type_name::<Func>(),
                        _err: PhantomData,
                    }),
                }
            }
        }
    };
}

macro_rules! impl_async_invoke_fn {
    ($marker:ident, $ok:ident, $fallible:ident, $($A:ident),+) => {
        #[doc(hidden)]
        pub struct $marker <Out, $($A),+>(PhantomData<fn() -> (Out, $($A,)+)>);

        struct $ok<FuncTy, $($A,)+> {
            deps: DepList,
            func: Option<FuncTy>,
            name: &'static str,
            _args: PhantomData<fn($($A),+)>,
        }

        impl<FuncTy, Fut, $($A,)+> Invoker for $ok<FuncTy, $($A,)+>
        where
            FuncTy: FnOnce($($A),+) -> Fut + Send + 'static,
            Fut: Future<Output = ()> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                self.deps.as_slice()
            }

            fn dep_list(&self) -> DepList {
                self.deps
            }

            fn call(mut self: Box<Self>, container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                match (|| {
                    Ok::<_, crate::error::Error>(func(
                        $(container.get::<$A>()?,)+
                    ))
                })() {
                    Ok(future) => InvokeOut::Fut(Box::pin(async move {
                        future.await;
                        Ok(())
                    })),
                    Err(err) => InvokeOut::Done(Err(err)),
                }
            }
        }

        struct $fallible<FuncTy, ErrTy, $($A,)+> {
            deps: DepList,
            func: Option<FuncTy>,
            name: &'static str,
            _args: PhantomData<(ErrTy, fn($($A),+))>,
        }

        impl<FuncTy, Fut, ErrTy, $($A,)+> Invoker for $fallible<FuncTy, ErrTy, $($A,)+>
        where
            FuncTy: FnOnce($($A),+) -> Fut + Send + 'static,
            Fut: Future<Output = std::result::Result<(), ErrTy>> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn name(&self) -> &'static str {
                self.name
            }

            fn dep_types(&self) -> &[(TypeId, &'static str)] {
                self.deps.as_slice()
            }

            fn dep_list(&self) -> DepList {
                self.deps
            }

            fn call(mut self: Box<Self>, container: &Container) -> InvokeOut {
                let func = self.func.take().expect("invoker called more than once");
                match (|| {
                    Ok::<_, crate::error::Error>(func(
                        $(container.get::<$A>()?,)+
                    ))
                })() {
                    Ok(future) => {
                        let name = self.name;
                        InvokeOut::Fut(Box::pin(async move {
                            future
                                .await
                                .map_err(|err| user_invoke_err(name, err))
                        }))
                    }
                    Err(err) => InvokeOut::Done(Err(err)),
                }
            }
        }

        impl<Func, Fut, $($A),+> AsyncInvokeFn<$marker<(), $($A),+>> for Func
        where
            Func: FnOnce($($A),+) -> Fut + Send + 'static,
            Fut: Future<Output = ()> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($ok {
                        deps: DepList::from_array([$(crate::deps::dep::<$A>(),)+]),
                        func: Some(self),
                        name: type_name::<Func>(),
                        _args: PhantomData,
                    }),
                }
            }
        }

        impl<Func, Fut, ErrTy, $($A),+> AsyncInvokeFn<$marker<std::result::Result<(), ErrTy>, $($A),+>> for Func
        where
            Func: FnOnce($($A),+) -> Fut + Send + 'static,
            Fut: Future<Output = std::result::Result<(), ErrTy>> + Send + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_invoke(self) -> DynInvoker {
                DynInvoker {
                    inner: Box::new($fallible {
                        deps: DepList::from_array([$(crate::deps::dep::<$A>(),)+]),
                        func: Some(self),
                        name: type_name::<Func>(),
                        _args: PhantomData,
                    }),
                }
            }
        }
    };
}

impl_async_invoke_zero!(AsyncInvoke0, AsyncInvoke0Ok, AsyncInvoke0Fallible);
impl_async_invoke_fn!(AsyncInvoke1, AsyncInvoke1Ok, AsyncInvoke1Fallible, A);
impl_async_invoke_fn!(AsyncInvoke2, AsyncInvoke2Ok, AsyncInvoke2Fallible, A, B);
impl_async_invoke_fn!(AsyncInvoke3, AsyncInvoke3Ok, AsyncInvoke3Fallible, A, B, C);
impl_async_invoke_fn!(
    AsyncInvoke4,
    AsyncInvoke4Ok,
    AsyncInvoke4Fallible,
    A,
    B,
    C,
    D
);
impl_async_invoke_fn!(
    AsyncInvoke5,
    AsyncInvoke5Ok,
    AsyncInvoke5Fallible,
    A,
    B,
    C,
    D,
    E
);
impl_async_invoke_fn!(
    AsyncInvoke6,
    AsyncInvoke6Ok,
    AsyncInvoke6Fallible,
    A,
    B,
    C,
    D,
    E,
    F
);
impl_async_invoke_fn!(
    AsyncInvoke7,
    AsyncInvoke7Ok,
    AsyncInvoke7Fallible,
    A,
    B,
    C,
    D,
    E,
    F,
    G
);
impl_async_invoke_fn!(
    AsyncInvoke8,
    AsyncInvoke8Ok,
    AsyncInvoke8Fallible,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H
);
