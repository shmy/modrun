use std::marker::PhantomData;

use crate::app::BuildState;
use crate::error::Result;
use crate::group::Group;
use crate::option::ModOption;
use crate::provide::{
    AsyncProviderFn, DynProvider, FallibleAsyncProviderFn, FallibleProviderFn, ProviderFn,
    ProviderMarker,
};

macro_rules! provide_group_fn {
    ($name:ident, $trait:ident) => {
        pub(crate) fn $name<M, F>(ctor: F) -> Box<dyn ModOption>
        where
            M: ProviderMarker,
            <M as ProviderMarker>::Output: Clone + Send + Sync + 'static,
            F: $trait<M> + 'static,
        {
            register_group::<M::Output>(ctor.into_provider())
        }
    };
}

provide_group_fn!(provide_group, ProviderFn);
provide_group_fn!(provide_group_result, FallibleProviderFn);
provide_group_fn!(provide_group_async, AsyncProviderFn);
provide_group_fn!(provide_group_result_async, FallibleAsyncProviderFn);

pub(crate) fn provide_group_dyn<T: Clone + Send + Sync + 'static>(
    provider: DynProvider,
) -> Box<dyn ModOption> {
    if provider.result_type() != std::any::TypeId::of::<T>() {
        return Box::new(GroupMemberTypeMismatchOption {
            expected: std::any::type_name::<T>(),
            actual: provider.result_name(),
        });
    }
    register_group::<T>(provider)
}

struct GroupMemberTypeMismatchOption {
    expected: &'static str,
    actual: &'static str,
}

impl ModOption for GroupMemberTypeMismatchOption {
    fn apply(self: Box<Self>, _app: &mut BuildState) -> Result<()> {
        Err(crate::error::Error::GroupMemberTypeMismatch {
            expected: self.expected,
            actual: self.actual,
        })
    }
}

fn register_group<T>(provider: DynProvider) -> Box<dyn ModOption>
where
    T: Clone + Send + Sync + 'static,
{
    Box::new(ProvideGroupOption::<T> {
        provider,
        _marker: PhantomData,
    })
}

struct ProvideGroupOption<T> {
    provider: DynProvider,
    _marker: PhantomData<T>,
}

impl<T: Clone + Send + Sync + 'static> ModOption for ProvideGroupOption<T> {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        let type_name = self.provider.result_name();
        let constructor = self.provider.constructor_name();
        let scope = app.current_scope;
        app.container
            .insert_group_member_typed::<T>(self.provider, scope)?;
        crate::trace::provided_group(
            type_name,
            constructor,
            app.container.scopes().name(scope),
            false,
        );
        Ok(())
    }
}

pub(crate) fn init_group<T: Clone + Send + Sync + 'static>() -> Box<dyn ModOption> {
    Box::new(InitGroupOption(PhantomData::<T>))
}

struct InitGroupOption<T>(PhantomData<T>);

impl<T: Clone + Send + Sync + 'static> ModOption for InitGroupOption<T> {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        app.container.ensure_empty_group::<T>()
    }
}

pub(crate) fn require_group<T: Clone + Send + Sync + 'static>() -> Box<dyn ModOption> {
    Box::new(RequireGroupOption(PhantomData::<T>))
}

struct RequireGroupOption<T>(PhantomData<T>);

impl<T: Clone + Send + Sync + 'static> ModOption for RequireGroupOption<T> {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        app.container.ensure_empty_group::<T>()?;
        app.container.require_group_element(
            std::any::TypeId::of::<T>(),
            std::any::type_name::<Group<T>>(),
        );
        Ok(())
    }
}
