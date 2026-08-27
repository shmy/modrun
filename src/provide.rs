use std::any::{TypeId, type_name};
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::error::Result;

use crate::app::BuildState;
use crate::container::{ConstructFuture, Container, Provider, pack};
use crate::error::user_ctor_err;
use crate::option::ModOption;

pub(crate) fn provide<M, F>(ctor: F) -> Box<dyn ModOption>
where
    F: ProviderFn<M> + 'static,
{
    provide_dyn(ctor.into_provider())
}

pub(crate) fn provide_result<M, F>(ctor: F) -> Box<dyn ModOption>
where
    F: FallibleProviderFn<M> + 'static,
{
    provide_dyn(ctor.into_provider())
}

pub(crate) fn provide_async<M, F>(ctor: F) -> Box<dyn ModOption>
where
    F: AsyncProviderFn<M> + 'static,
{
    provide_dyn(ctor.into_provider())
}

pub(crate) fn provide_result_async<M, F>(ctor: F) -> Box<dyn ModOption>
where
    F: FallibleAsyncProviderFn<M> + 'static,
{
    provide_dyn(ctor.into_provider())
}

pub(crate) fn provide_dyn(provider: DynProvider) -> Box<dyn ModOption> {
    Box::new(ProvideOption {
        provider: Arc::new(provider),
    })
}

struct ProvideOption {
    provider: Arc<dyn Provider>,
}

impl ModOption for ProvideOption {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        let type_name = self.provider.result_name();
        let private = app.private_mode;
        let scope = app.current_scope;
        app.container
            .insert_provider(self.provider, scope, private)?;
        crate::trace::provided(type_name, app.container.scopes().name(scope), private);
        Ok(())
    }
}

/// Bound for constructors accepted by [`ModrunBuilder::provide`](crate::ModrunBuilder::provide).
///
/// Implemented for any `Fn(A, B, ..) -> T` of up to **eight** `Clone` arguments.
/// The result type itself need not be `Clone`; inject `Arc<T>` to avoid a copy.
/// `Marker` distinguishes those arities and is always inferred.
pub trait ProviderFn<Marker>: Sized {
    /// Erase the constructor's signature into a provider the container can call.
    fn into_provider(self) -> DynProvider;
}

/// Bound for fallible constructors accepted by
/// [`ModrunBuilder::provide_result`](crate::ModrunBuilder::provide_result).
pub trait FallibleProviderFn<Marker>: Sized {
    /// Erase the constructor's signature into a provider the container can call.
    fn into_provider(self) -> DynProvider;
}

/// Bound for async constructors accepted by
/// [`ModrunBuilder::provide_async`](crate::ModrunBuilder::provide_async).
pub trait AsyncProviderFn<Marker>: Sized {
    /// Erase the constructor's signature into a provider the container can call.
    fn into_provider(self) -> DynProvider;
}

/// Bound for fallible async constructors accepted by
/// [`ModrunBuilder::provide_result_async`](crate::ModrunBuilder::provide_result_async).
pub trait FallibleAsyncProviderFn<Marker>: Sized {
    /// Erase the constructor's signature into a provider the container can call.
    fn into_provider(self) -> DynProvider;
}

type ConstructFn = Box<dyn Fn(&Container) -> Result<ConstructFuture> + Send + Sync>;

/// A constructor with its signature erased, ready to be registered via
/// [`ModrunBuilder::provide_dyn`](crate::ModrunBuilder::provide_dyn).
pub struct DynProvider {
    result_type: TypeId,
    result_name: &'static str,
    alias_types: Vec<TypeId>,
    deps: Vec<(TypeId, &'static str)>,
    construct: ConstructFn,
}

impl std::fmt::Debug for DynProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynProvider")
            .field("result", &self.result_name)
            .field(
                "deps",
                &self.deps.iter().map(|(_, n)| n).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl DynProvider {
    /// Type produced by this provider.
    #[must_use]
    pub fn result_type(&self) -> TypeId {
        self.result_type
    }

    /// Rust type name of the value produced by this provider.
    #[must_use]
    pub fn result_name(&self) -> &'static str {
        self.result_name
    }

    /// Dependency types this provider will resolve.
    #[must_use]
    pub fn dep_types(&self) -> &[(TypeId, &'static str)] {
        &self.deps
    }
}

impl Provider for DynProvider {
    fn result_type(&self) -> TypeId {
        self.result_type
    }

    fn result_name(&self) -> &'static str {
        self.result_name
    }

    fn alias_types(&self) -> &[TypeId] {
        &self.alias_types
    }

    fn dep_types(&self) -> &[(TypeId, &'static str)] {
        &self.deps
    }

    fn construct(&self, container: &Container) -> Result<ConstructFuture> {
        (self.construct)(container)
    }
}

fn dyn_for<T: Send + Sync + 'static>(
    deps: Vec<(TypeId, &'static str)>,
    construct: ConstructFn,
) -> DynProvider {
    DynProvider {
        result_type: TypeId::of::<T>(),
        result_name: type_name::<T>(),
        alias_types: vec![TypeId::of::<Arc<T>>()],
        deps,
        construct,
    }
}

fn ready_packed<T: Send + Sync + 'static>(value: T) -> ConstructFuture {
    Box::pin(std::future::ready(Ok(pack(value))))
}

fn ctor_failed<T: ?Sized>(err: impl Into<crate::error::BoxError>) -> crate::error::Error {
    user_ctor_err::<T>(err)
}

macro_rules! impl_provider_fn_zero {
    ($marker:ident, $fallible:ident, $async_marker:ident, $async_fallible:ident) => {
        #[doc(hidden)]
        pub struct $marker<Out>(PhantomData<fn() -> Out>);
        #[doc(hidden)]
        pub struct $fallible<T, ErrTy>(PhantomData<fn() -> (T, ErrTy)>);
        #[doc(hidden)]
        pub struct $async_marker<Out>(PhantomData<fn() -> Out>);
        #[doc(hidden)]
        pub struct $async_fallible<T, ErrTy>(PhantomData<fn() -> (T, ErrTy)>);

        impl<Func, Out> ProviderFn<$marker<Out>> for Func
        where
            Func: Fn() -> Out + Send + Sync + 'static,
            Out: Send + Sync + 'static,
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<Out>(
                    vec![],
                    Box::new(move |_container: &Container| Ok(ready_packed((self)()))),
                )
            }
        }

        impl<Func, T, ErrTy> FallibleProviderFn<$fallible<T, ErrTy>> for Func
        where
            Func: Fn() -> std::result::Result<T, ErrTy> + Send + Sync + 'static,
            T: Send + Sync + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<T>(
                    vec![],
                    Box::new(move |_container: &Container| {
                        let value = (self)().map_err(ctor_failed::<T>)?;
                        Ok(ready_packed(value))
                    }),
                )
            }
        }

        impl<Func, Fut, Out> AsyncProviderFn<$async_marker<Out>> for Func
        where
            Func: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Out> + Send + 'static,
            Out: Send + Sync + 'static,
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<Out>(
                    vec![],
                    Box::new(move |_container: &Container| {
                        let future = (self)();
                        Ok(Box::pin(async move { Ok(pack(future.await)) }))
                    }),
                )
            }
        }

        impl<Func, Fut, T, ErrTy> FallibleAsyncProviderFn<$async_fallible<T, ErrTy>> for Func
        where
            Func: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = std::result::Result<T, ErrTy>> + Send + 'static,
            T: Send + Sync + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<T>(
                    vec![],
                    Box::new(move |_container: &Container| {
                        let future = (self)();
                        Ok(Box::pin(async move {
                            let value = future.await.map_err(ctor_failed::<T>)?;
                            Ok(pack(value))
                        }))
                    }),
                )
            }
        }
    };
}

macro_rules! impl_provider_fn {
    ($marker:ident, $fallible:ident, $async_marker:ident, $async_fallible:ident, $($A:ident),+) => {
        #[doc(hidden)]
        pub struct $marker <Out, $($A),+>(PhantomData<fn() -> (Out, $($A,)+)>);
        #[doc(hidden)]
        pub struct $fallible <T, ErrTy, $($A),+>(PhantomData<fn() -> (T, ErrTy, $($A,)+)>);
        #[doc(hidden)]
        pub struct $async_marker <Out, $($A),+>(PhantomData<fn() -> (Out, $($A,)+)>);
        #[doc(hidden)]
        pub struct $async_fallible <T, ErrTy, $($A),+>(PhantomData<fn() -> (T, ErrTy, $($A,)+)>);

        impl<Func, Out, $($A),+> ProviderFn<$marker<Out, $($A),+>> for Func
        where
            Func: Fn($($A),+) -> Out + Send + Sync + 'static,
            Out: Send + Sync + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<Out>(
                    vec![$((TypeId::of::<$A>(), type_name::<$A>()),)+],
                    Box::new(move |container: &Container| {
                        let value = (self)(
                            $(container.get::<$A>()?,)+
                        );
                        Ok(ready_packed(value))
                    }),
                )
            }
        }

        impl<Func, T, ErrTy, $($A),+> FallibleProviderFn<$fallible<T, ErrTy, $($A),+>> for Func
        where
            Func: Fn($($A),+) -> std::result::Result<T, ErrTy> + Send + Sync + 'static,
            T: Send + Sync + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<T>(
                    vec![$((TypeId::of::<$A>(), type_name::<$A>()),)+],
                    Box::new(move |container: &Container| {
                        let value = (self)(
                            $(container.get::<$A>()?,)+
                        )
                        .map_err(ctor_failed::<T>)?;
                        Ok(ready_packed(value))
                    }),
                )
            }
        }

        impl<Func, Fut, Out, $($A),+> AsyncProviderFn<$async_marker<Out, $($A),+>> for Func
        where
            Func: Fn($($A),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Out> + Send + 'static,
            Out: Send + Sync + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<Out>(
                    vec![$((TypeId::of::<$A>(), type_name::<$A>()),)+],
                    Box::new(move |container: &Container| {
                        let future = (self)(
                            $(container.get::<$A>()?,)+
                        );
                        Ok(Box::pin(async move { Ok(pack(future.await)) }))
                    }),
                )
            }
        }

        impl<Func, Fut, T, ErrTy, $($A),+>
            FallibleAsyncProviderFn<$async_fallible<T, ErrTy, $($A),+>> for Func
        where
            Func: Fn($($A),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = std::result::Result<T, ErrTy>> + Send + 'static,
            T: Send + Sync + 'static,
            ErrTy: Into<crate::error::BoxError> + Send + 'static,
            $($A: Clone + Send + Sync + 'static,)+
        {
            fn into_provider(self) -> DynProvider {
                dyn_for::<T>(
                    vec![$((TypeId::of::<$A>(), type_name::<$A>()),)+],
                    Box::new(move |container: &Container| {
                        let future = (self)(
                            $(container.get::<$A>()?,)+
                        );
                        Ok(Box::pin(async move {
                            let value = future.await.map_err(ctor_failed::<T>)?;
                            Ok(pack(value))
                        }))
                    }),
                )
            }
        }
    };
}

impl_provider_fn_zero!(Provider0, Fallible0, AsyncProvider0, AsyncFallible0);
impl_provider_fn!(Provider1, Fallible1, AsyncProvider1, AsyncFallible1, A);
impl_provider_fn!(Provider2, Fallible2, AsyncProvider2, AsyncFallible2, A, B);
impl_provider_fn!(
    Provider3,
    Fallible3,
    AsyncProvider3,
    AsyncFallible3,
    A,
    B,
    C
);
impl_provider_fn!(
    Provider4,
    Fallible4,
    AsyncProvider4,
    AsyncFallible4,
    A,
    B,
    C,
    D
);
impl_provider_fn!(
    Provider5,
    Fallible5,
    AsyncProvider5,
    AsyncFallible5,
    A,
    B,
    C,
    D,
    E
);
impl_provider_fn!(
    Provider6,
    Fallible6,
    AsyncProvider6,
    AsyncFallible6,
    A,
    B,
    C,
    D,
    E,
    F
);
impl_provider_fn!(
    Provider7,
    Fallible7,
    AsyncProvider7,
    AsyncFallible7,
    A,
    B,
    C,
    D,
    E,
    F,
    G
);
impl_provider_fn!(
    Provider8,
    Fallible8,
    AsyncProvider8,
    AsyncFallible8,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H
);
