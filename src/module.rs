use crate::error::Result;

use crate::app::BuildState;
use crate::option::ModOption;
use crate::wiring::{impl_private_wiring_methods, impl_wiring_methods};

/// A named domain module.
///
/// ```
/// # use modrun::{Modrun, Module};
/// #[derive(Clone)]
/// struct Config;
/// #[derive(Clone)]
/// struct Repo;
/// #[derive(Clone)]
/// struct Greeter;
///
/// fn new_repo(_cfg: Config) -> Repo { Repo }
/// fn new_greeter(_repo: Repo) -> Greeter { Greeter }
/// fn register(_greeter: Greeter) {}
///
/// fn greeter_domain() -> Module {
///     Module::new("greeter")
///         .provide_private(new_repo)
///         .provide(new_greeter)
///         .invoke(register)
/// }
///
/// # #[tokio::main]
/// # async fn main() -> modrun::Result<()> {
/// Modrun::builder()
///     .supply(Config)
///     .module(greeter_domain())
///     .start()
///     .await?
///     .stop()
///     .await
/// # }
/// ```
pub struct Module {
    name: &'static str,
    options: Vec<Box<dyn ModOption>>,
}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module")
            .field("name", &self.name)
            .field("options", &self.options.len())
            .finish()
    }
}

impl Module {
    /// An empty module. `name` should be unique; it is used only in diagnostics.
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            options: Vec::new(),
        }
    }

    fn push_option(&mut self, option: Box<dyn ModOption>) {
        self.options.push(option);
    }

    /// Nest another domain module.
    #[must_use]
    pub fn module(mut self, child: Module) -> Self {
        self.module_mut(child);
        self
    }

    /// [`module`](Self::module) for `&mut self`.
    pub fn module_mut(&mut self, child: Module) -> &mut Self {
        self.push_option(child.into_option());
        self
    }

    pub(crate) fn into_option(self) -> Box<dyn ModOption> {
        Box::new(ModuleOption {
            name: self.name,
            options: self.options,
        })
    }
}

impl_wiring_methods!(Module);
impl_private_wiring_methods!(Module);

struct ModuleOption {
    name: &'static str,
    options: Vec<Box<dyn ModOption>>,
}

impl ModOption for ModuleOption {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        let parent = app.current_scope;
        let child = app.container.scopes_mut().child(parent, self.name);
        app.current_scope = child;
        // On `Err` we restore; on panic the whole `BuildState` is discarded.
        let result = (|| {
            for opt in self.options {
                opt.apply(app)?;
            }
            Ok(())
        })();
        app.current_scope = parent;
        result
    }
}
