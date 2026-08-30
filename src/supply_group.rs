use std::marker::PhantomData;

use crate::app::BuildState;
use crate::error::Result;
use crate::option::ModOption;
use crate::provide::take_once_value_provider;

pub(crate) fn supply_group<T: Clone + Send + Sync + 'static>(value: T) -> Box<dyn ModOption> {
    Box::new(SupplyGroupOption {
        value,
        _marker: PhantomData::<T>,
    })
}

struct SupplyGroupOption<T> {
    value: T,
    _marker: PhantomData<T>,
}

impl<T: Clone + Send + Sync + 'static> ModOption for SupplyGroupOption<T> {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        let scope = app.current_scope;
        let provider = take_once_value_provider(self.value, "<supply_group>");
        app.container
            .insert_group_member_typed::<T>(provider, scope)?;
        crate::trace::supplied_group(
            std::any::type_name::<T>(),
            app.container.scopes().name(scope),
            false,
        );
        Ok(())
    }
}
