//! Fluent wiring methods shared by [`ModrunBuilder`](crate::ModrunBuilder) and [`Module`](crate::Module).

/// Implements `.provide` / `.supply` / `.invoke` (and private variants on [`Module`](crate::Module)).
///
/// The type must provide `fn push_option(&mut self, Box<dyn ModOption>)`.
macro_rules! impl_wiring_methods {
    ($ty:ty) => {
        impl $ty {
            /// Register a constructor. Dependencies are injected by cloning singletons
            /// (or by cloning an `Arc<T>` if the parameter is `Arc<T>`).
            ///
            /// Constructors that return `Result<T, E>` must use
            /// [`provide_result`](Self::provide_result) instead; passing one here
            /// is a compile error.
            ///
            /// ```compile_fail
            /// # use modrun::Modrun;
            /// #[derive(Clone)]
            /// struct Config;
            ///
            /// fn new_config() -> Result<Config, std::io::Error> {
            ///     Ok(Config)
            /// }
            ///
            /// Modrun::builder().provide(new_config);
            /// ```
            #[must_use]
            pub fn provide<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::ProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide(ctor));
                self
            }

            /// Register a fallible constructor (`Result<T, E>`).
            #[must_use]
            pub fn provide_result<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::FallibleProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_result(ctor));
                self
            }

            /// Register an `async` constructor, awaited while the graph is built.
            #[must_use]
            pub fn provide_async<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::AsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_async(ctor));
                self
            }

            /// Register a fallible `async` constructor (`Result<T, E>`).
            #[must_use]
            pub fn provide_result_async<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::FallibleAsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_result_async(ctor));
                self
            }

            /// Register an already-erased constructor, from
            /// [`ProviderFn::into_provider`](crate::__wiring::ProviderFn::into_provider).
            #[doc(hidden)]
            #[must_use]
            pub fn provide_dyn(mut self, provider: $crate::__wiring::DynProvider) -> Self {
                self.push_option($crate::provide::provide_dyn(provider));
                self
            }

            /// Supply a pre-built value. Inject by value requires `T: Clone`; inject
            /// `Arc<T>` does not.
            #[must_use]
            pub fn supply<T: Send + Sync + 'static>(mut self, value: T) -> Self {
                self.push_option($crate::supply::supply(value));
                self
            }

            /// Register an invoker that pulls the dependency graph.
            #[must_use]
            pub fn invoke<M, F>(mut self, func: F) -> Self
            where
                F: $crate::invoke::InvokeFn<M> + 'static,
            {
                self.push_option($crate::invoke::invoke(func));
                self
            }

            /// Register an `async` invoker that pulls the dependency graph.
            #[must_use]
            pub fn invoke_async<M, F>(mut self, func: F) -> Self
            where
                F: $crate::invoke::AsyncInvokeFn<M> + 'static,
            {
                self.push_option($crate::invoke::invoke_async(func));
                self
            }

            /// Register an already-erased invoker, from
            /// [`InvokeFn::into_invoke`](crate::__wiring::InvokeFn::into_invoke).
            #[doc(hidden)]
            #[must_use]
            pub fn invoke_dyn(mut self, invoker: $crate::__wiring::DynInvoker) -> Self {
                self.push_option($crate::invoke::invoke_dyn(invoker));
                self
            }
        }
    };
}

pub(crate) use impl_wiring_methods;

/// Private provide/supply variants — only on [`Module`](crate::Module), because a
/// private binding on the root builder sits on every ancestor chain.
macro_rules! impl_private_wiring_methods {
    ($ty:ty) => {
        impl $ty {
            /// Like [`provide`](Self::provide), but only visible inside this module.
            #[must_use]
            pub fn provide_private<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::ProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_priv(ctor));
                self
            }

            /// Like [`provide_result`](Self::provide_result), scoped privately to the module.
            #[must_use]
            pub fn provide_result_private<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::FallibleProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_result_priv(ctor));
                self
            }

            /// Like [`provide_async`](Self::provide_async), scoped privately to the module.
            #[must_use]
            pub fn provide_async_private<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::AsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_async_priv(ctor));
                self
            }

            /// Like [`provide_result_async`](Self::provide_result_async), scoped privately.
            #[must_use]
            pub fn provide_result_async_private<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::FallibleAsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_result_async_priv(ctor));
                self
            }

            /// Like [`provide_dyn`](Self::provide_dyn), scoped privately to the module.
            #[doc(hidden)]
            #[must_use]
            pub fn provide_dyn_private(mut self, provider: $crate::__wiring::DynProvider) -> Self {
                self.push_option($crate::provide::provide_dyn_priv(provider));
                self
            }

            /// Supply a value private to this module.
            #[must_use]
            pub fn supply_private<T: Send + Sync + 'static>(mut self, value: T) -> Self {
                self.push_option($crate::supply::supply_priv(value));
                self
            }
        }
    };
}

pub(crate) use impl_private_wiring_methods;

/// Group provide/supply variants shared by [`ModrunBuilder`](crate::ModrunBuilder) and [`Module`](crate::Module).
macro_rules! impl_group_wiring_methods {
    ($ty:ty) => {
        impl $ty {
            /// Register a group member constructor (sync / infallible).
            ///
            /// `T` is inferred from the constructor's return type; there is no
            /// `provide_group::<T>(...)` turbofish. For a trait-object group,
            /// return `Arc<dyn Trait>` from the constructor.
            #[must_use]
            pub fn provide_group<M, F>(mut self, ctor: F) -> Self
            where
                M: $crate::provide::ProviderMarker,
                <M as $crate::provide::ProviderMarker>::Output: Clone + Send + Sync + 'static,
                F: $crate::provide::ProviderFn<M> + 'static,
            {
                self.push_option($crate::provide_group::provide_group::<M, F>(ctor));
                self
            }

            /// Register a fallible group member constructor (`Result<T, E>`).
            #[must_use]
            pub fn provide_group_result<M, F>(mut self, ctor: F) -> Self
            where
                M: $crate::provide::ProviderMarker,
                <M as $crate::provide::ProviderMarker>::Output: Clone + Send + Sync + 'static,
                F: $crate::provide::FallibleProviderFn<M> + 'static,
            {
                self.push_option($crate::provide_group::provide_group_result::<M, F>(ctor));
                self
            }

            /// Register an async group member constructor.
            #[must_use]
            pub fn provide_group_async<M, F>(mut self, ctor: F) -> Self
            where
                M: $crate::provide::ProviderMarker,
                <M as $crate::provide::ProviderMarker>::Output: Clone + Send + Sync + 'static,
                F: $crate::provide::AsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::provide_group::provide_group_async::<M, F>(ctor));
                self
            }

            /// Register a fallible async group member constructor (`Result<T, E>`).
            #[must_use]
            pub fn provide_group_result_async<M, F>(mut self, ctor: F) -> Self
            where
                M: $crate::provide::ProviderMarker,
                <M as $crate::provide::ProviderMarker>::Output: Clone + Send + Sync + 'static,
                F: $crate::provide::FallibleAsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::provide_group::provide_group_result_async::<M, F>(
                    ctor,
                ));
                self
            }

            /// Supply a pre-built group member.
            #[must_use]
            pub fn supply_group<T: Clone + Send + Sync + 'static>(mut self, value: T) -> Self {
                self.push_option($crate::supply_group::supply_group(value));
                self
            }

            /// Register an already-erased group member, from
            /// [`ProviderFn::into_provider`](crate::__wiring::ProviderFn::into_provider).
            #[doc(hidden)]
            #[must_use]
            pub fn provide_group_dyn<T: Clone + Send + Sync + 'static>(
                mut self,
                provider: $crate::__wiring::DynProvider,
            ) -> Self {
                self.push_option($crate::provide_group::provide_group_dyn::<T>(provider));
                self
            }
        }
    };
}

pub(crate) use impl_group_wiring_methods;
