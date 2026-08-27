//! Fluent wiring methods shared by [`ModrunBuilder`](crate::ModrunBuilder) and [`Module`](crate::Module).

/// Implements `.provide` / `.supply` / `.invoke` (and private / `_mut` variants).
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
            #[must_use]
            pub fn provide<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::ProviderFn<M> + 'static,
            {
                self.provide_mut(ctor);
                self
            }

            /// [`provide`](Self::provide) for `&mut self`.
            pub fn provide_mut<M, F>(&mut self, ctor: F) -> &mut Self
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
                self.provide_result_mut(ctor);
                self
            }

            /// [`provide_result`](Self::provide_result) for `&mut self`.
            pub fn provide_result_mut<M, F>(&mut self, ctor: F) -> &mut Self
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
                self.provide_async_mut(ctor);
                self
            }

            /// [`provide_async`](Self::provide_async) for `&mut self`.
            pub fn provide_async_mut<M, F>(&mut self, ctor: F) -> &mut Self
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
                self.provide_result_async_mut(ctor);
                self
            }

            /// [`provide_result_async`](Self::provide_result_async) for `&mut self`.
            pub fn provide_result_async_mut<M, F>(&mut self, ctor: F) -> &mut Self
            where
                F: $crate::provide::FallibleAsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::provide::provide_result_async(ctor));
                self
            }

            /// Register an already-erased constructor, from
            /// [`ProviderFn::into_provider`](crate::ProviderFn::into_provider).
            #[must_use]
            pub fn provide_dyn(mut self, provider: $crate::DynProvider) -> Self {
                self.provide_dyn_mut(provider);
                self
            }

            /// [`provide_dyn`](Self::provide_dyn) for `&mut self`.
            pub fn provide_dyn_mut(&mut self, provider: $crate::DynProvider) -> &mut Self {
                self.push_option($crate::provide::provide_dyn(provider));
                self
            }

            /// Supply a pre-built value. Inject by value requires `T: Clone`; inject
            /// `Arc<T>` does not.
            #[must_use]
            pub fn supply<T: Send + Sync + 'static>(mut self, value: T) -> Self {
                self.supply_mut(value);
                self
            }

            /// [`supply`](Self::supply) for `&mut self`.
            pub fn supply_mut<T: Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
                self.push_option($crate::supply::supply(value));
                self
            }

            /// Register an invoker that pulls the dependency graph.
            #[must_use]
            pub fn invoke<M, F>(mut self, func: F) -> Self
            where
                F: $crate::invoke::InvokeFn<M> + 'static,
            {
                self.invoke_mut(func);
                self
            }

            /// [`invoke`](Self::invoke) for `&mut self`.
            pub fn invoke_mut<M, F>(&mut self, func: F) -> &mut Self
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
                self.invoke_async_mut(func);
                self
            }

            /// [`invoke_async`](Self::invoke_async) for `&mut self`.
            pub fn invoke_async_mut<M, F>(&mut self, func: F) -> &mut Self
            where
                F: $crate::invoke::AsyncInvokeFn<M> + 'static,
            {
                self.push_option($crate::invoke::invoke_async(func));
                self
            }

            /// Register an already-erased invoker, from
            /// [`InvokeFn::into_invoke`](crate::InvokeFn::into_invoke).
            #[must_use]
            pub fn invoke_dyn(mut self, invoker: $crate::DynInvoker) -> Self {
                self.invoke_dyn_mut(invoker);
                self
            }

            /// [`invoke_dyn`](Self::invoke_dyn) for `&mut self`.
            pub fn invoke_dyn_mut(&mut self, invoker: $crate::DynInvoker) -> &mut Self {
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
                self.provide_private_mut(ctor);
                self
            }

            /// [`provide_private`](Self::provide_private) for `&mut self`.
            pub fn provide_private_mut<M, F>(&mut self, ctor: F) -> &mut Self
            where
                F: $crate::provide::ProviderFn<M> + 'static,
            {
                self.push_option($crate::module::private($crate::provide::provide(ctor)));
                self
            }

            /// Like [`provide_result`](Self::provide_result), scoped privately to the module.
            #[must_use]
            pub fn provide_result_private<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::FallibleProviderFn<M> + 'static,
            {
                self.provide_result_private_mut(ctor);
                self
            }

            /// [`provide_result_private`](Self::provide_result_private) for `&mut self`.
            pub fn provide_result_private_mut<M, F>(&mut self, ctor: F) -> &mut Self
            where
                F: $crate::provide::FallibleProviderFn<M> + 'static,
            {
                self.push_option($crate::module::private($crate::provide::provide_result(
                    ctor,
                )));
                self
            }

            /// Like [`provide_async`](Self::provide_async), scoped privately to the module.
            #[must_use]
            pub fn provide_async_private<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::AsyncProviderFn<M> + 'static,
            {
                self.provide_async_private_mut(ctor);
                self
            }

            /// [`provide_async_private`](Self::provide_async_private) for `&mut self`.
            pub fn provide_async_private_mut<M, F>(&mut self, ctor: F) -> &mut Self
            where
                F: $crate::provide::AsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::module::private($crate::provide::provide_async(
                    ctor,
                )));
                self
            }

            /// Like [`provide_result_async`](Self::provide_result_async), scoped privately.
            #[must_use]
            pub fn provide_result_async_private<M, F>(mut self, ctor: F) -> Self
            where
                F: $crate::provide::FallibleAsyncProviderFn<M> + 'static,
            {
                self.provide_result_async_private_mut(ctor);
                self
            }

            /// [`provide_result_async_private`](Self::provide_result_async_private) for `&mut self`.
            pub fn provide_result_async_private_mut<M, F>(&mut self, ctor: F) -> &mut Self
            where
                F: $crate::provide::FallibleAsyncProviderFn<M> + 'static,
            {
                self.push_option($crate::module::private(
                    $crate::provide::provide_result_async(ctor),
                ));
                self
            }

            /// Like [`provide_dyn`](Self::provide_dyn), scoped privately to the module.
            #[must_use]
            pub fn provide_dyn_private(mut self, provider: $crate::DynProvider) -> Self {
                self.provide_dyn_private_mut(provider);
                self
            }

            /// [`provide_dyn_private`](Self::provide_dyn_private) for `&mut self`.
            pub fn provide_dyn_private_mut(&mut self, provider: $crate::DynProvider) -> &mut Self {
                self.push_option($crate::module::private($crate::provide::provide_dyn(
                    provider,
                )));
                self
            }

            /// Supply a value private to this module.
            #[must_use]
            pub fn supply_private<T: Send + Sync + 'static>(mut self, value: T) -> Self {
                self.supply_private_mut(value);
                self
            }

            /// [`supply_private`](Self::supply_private) for `&mut self`.
            pub fn supply_private_mut<T: Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
                self.push_option($crate::module::private($crate::supply::supply(value)));
                self
            }
        }
    };
}

pub(crate) use impl_private_wiring_methods;
