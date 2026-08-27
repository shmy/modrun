use std::time::Duration;

use crate::app::BuildState;
use crate::error::Result;
use crate::option::ModOption;

/// Default build/start/stop timeout (15s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn build_timeout(duration: Option<Duration>) -> Box<dyn ModOption> {
    Box::new(BuildTimeoutOption(duration))
}

pub(crate) fn start_timeout(duration: Option<Duration>) -> Box<dyn ModOption> {
    Box::new(StartTimeoutOption(duration))
}

pub(crate) fn stop_timeout(duration: Option<Duration>) -> Box<dyn ModOption> {
    Box::new(StopTimeoutOption(duration))
}

struct BuildTimeoutOption(Option<Duration>);
struct StartTimeoutOption(Option<Duration>);
struct StopTimeoutOption(Option<Duration>);

impl ModOption for BuildTimeoutOption {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        app.build_timeout = self.0;
        Ok(())
    }
}

impl ModOption for StartTimeoutOption {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        app.start_timeout = self.0;
        Ok(())
    }
}

impl ModOption for StopTimeoutOption {
    fn apply(self: Box<Self>, app: &mut BuildState) -> Result<()> {
        app.stop_timeout = self.0;
        Ok(())
    }
}
