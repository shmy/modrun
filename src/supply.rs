use std::any::type_name;

use crate::app::BuildState;
use crate::error::Result;
use crate::option::ModOption;

pub(crate) fn supply<T: Send + Sync + 'static>(value: T) -> Box<dyn ModOption> {
    supply_vis(value, false)
}

pub(crate) fn supply_priv<T: Send + Sync + 'static>(value: T) -> Box<dyn ModOption> {
    supply_vis(value, true)
}

fn supply_vis<T: Send + Sync + 'static>(value: T, private: bool) -> Box<dyn ModOption> {
    Box::new(SupplyOption { value, private })
}

struct SupplyOption<T> {
    value: T,
    private: bool,
}

impl<T: Send + Sync + 'static> ModOption for SupplyOption<T> {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        let private = self.private;
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
