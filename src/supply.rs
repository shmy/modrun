use std::any::type_name;

use crate::app::BuildState;
use crate::error::Result;
use crate::option::ModOption;

pub(crate) fn supply<T: Send + Sync + 'static>(value: T) -> Box<dyn ModOption> {
    Box::new(SupplyOption { value })
}

struct SupplyOption<T> {
    value: T,
}

impl<T: Send + Sync + 'static> ModOption for SupplyOption<T> {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        let private = app.private_mode;
        let scope = app.current_scope;
        app.container.insert_value(self.value, scope, private)?;
        crate::trace::supplied(
            type_name::<T>(),
            app.container.scopes().name(scope),
            private,
        );
        Ok(())
    }
}
