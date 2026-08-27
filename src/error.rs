//! Public error type for modrun.

/// Owned user error retained as [`std::error::Error::source`].
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Result alias used throughout the crate and by hooks / fallible constructors.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while wiring, starting, or stopping an application.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A public type was registered more than once.
    #[error("type already provided: {0}")]
    AlreadyProvided(&'static str),

    /// A private type was registered more than once in the same module.
    #[error("type already provided privately in module '{module}': {type_name}")]
    AlreadyProvidedPrivate {
        /// Module name shown in diagnostics.
        module: &'static str,
        /// Type name that collided.
        type_name: &'static str,
    },

    /// The dependency graph contains a cycle.
    #[error("dependency cycle detected involving: {0}")]
    Cycle(String),

    /// A value was requested before it had been constructed.
    #[error("dependency not constructed yet: {0}")]
    NotConstructed(&'static str),

    /// No provider exists for a requested type.
    #[error("missing provider for type: {type_name} (from module '{module}')")]
    MissingProvider {
        /// Missing type.
        type_name: &'static str,
        /// Scope that requested it.
        module: &'static str,
    },

    /// A provider depends on something nothing registers.
    #[error(
        "provider for {provider} in module '{module}' needs a dependency nothing provides: {dependency}"
    )]
    ProviderMissingDep {
        /// Provider result type.
        provider: &'static str,
        /// Module that owns the provider.
        module: &'static str,
        /// Missing dependency type.
        dependency: &'static str,
    },

    /// An invoker depends on something nothing registers.
    #[error("invoker in module '{module}' needs a dependency nothing provides: {dependency}")]
    InvokerMissingDep {
        /// Module that owns the invoker.
        module: &'static str,
        /// Missing dependency type.
        dependency: &'static str,
    },

    /// Internal downcast of a cached value failed.
    #[error("failed to downcast value to {0}")]
    Downcast(&'static str),

    /// A fallible constructor returned an error.
    #[error("constructor for {type_name} failed: {source}")]
    ConstructorFailed {
        /// Type being constructed.
        type_name: &'static str,
        /// The user error from the constructor.
        #[source]
        source: BoxError,
    },

    /// A fallible invoker returned an error.
    #[error("invoker failed: {source}")]
    InvokerFailed {
        /// The user error from the invoker.
        #[source]
        source: BoxError,
    },

    /// Hook registration after start completed.
    #[error("cannot append lifecycle hook after start has finished")]
    AppendAfterStart,

    /// Hook registration while stopping.
    #[error("cannot append lifecycle hook while stopping")]
    AppendWhileStopping,

    /// Graph construction exceeded its budget.
    #[error("application build timed out after {0:?}")]
    BuildTimeout(std::time::Duration),

    /// Start phase exceeded its budget.
    #[error("application start timed out after {0:?}")]
    StartTimeout(std::time::Duration),

    /// Stop phase exceeded its budget.
    #[error("application stop timed out after {0:?}")]
    StopTimeout(std::time::Duration),

    /// Unwind after a failed/cancelled start exceeded its budget.
    #[error("application stop timed out after {0:?} while unwinding")]
    UnwindTimeout(std::time::Duration),

    /// Failed to install a SIGINT listener.
    #[error("failed to listen for SIGINT: {0}")]
    SigintListen(#[source] std::io::Error),

    /// Failed to install a SIGTERM listener.
    #[error("failed to listen for SIGTERM: {0}")]
    SigtermListen(#[source] std::io::Error),

    /// Failed to install a process-signal listener (non-Unix platforms).
    #[error("failed to listen for process signal: {0}")]
    SignalListen(#[source] std::io::Error),

    /// Several OnStop hooks failed; see [`MultipleStopError::errors`].
    #[error(transparent)]
    MultipleStop(MultipleStopError),

    /// Cleanup failed after an earlier phase error; both are retained.
    #[error("cleanup failed after an earlier phase error: {cleanup}; earlier: {earlier}")]
    CleanupAfterFailure {
        /// Error from stop/unwind.
        #[source]
        cleanup: Box<Error>,
        /// Error from the earlier phase.
        earlier: Box<Error>,
    },

    /// An application hook returned this failure.
    ///
    /// Prefer [`Error::hook`] rather than constructing this variant directly.
    #[error("hook failed: {0}")]
    Hook(#[source] BoxError),

    /// I/O failure with a short context label (e.g. bind / serve).
    ///
    /// Prefer [`Error::io`] rather than constructing this variant directly.
    #[error("{context}: {source}")]
    Io {
        /// What was being done when I/O failed.
        context: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Several OnStop hook failures aggregated into one error.
#[derive(Debug)]
pub struct MultipleStopError {
    /// Number of failed hooks.
    pub count: usize,
    /// Semicolon-joined displays for easy logging.
    pub summary: String,
    /// Individual failures in stop order.
    pub errors: Vec<Error>,
}

impl std::fmt::Display for MultipleStopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} OnStop hooks failed: {}", self.count, self.summary)
    }
}

impl std::error::Error for MultipleStopError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.errors
            .first()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl Error {
    /// Wrap a hook failure, keeping the original error as [`std::error::Error::source`].
    ///
    /// Accepts anything convertible to [`BoxError`], including `&str`, `String`,
    /// [`std::io::Error`], and types that implement [`std::error::Error`].
    pub fn hook(err: impl Into<BoxError>) -> Self {
        Self::Hook(err.into())
    }

    /// Wrap an I/O failure with a short context label (e.g. `"bind 127.0.0.1:3000"`).
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn constructor_failed<T: ?Sized>(err: impl Into<BoxError>) -> Self {
        Self::ConstructorFailed {
            type_name: std::any::type_name::<T>(),
            source: err.into(),
        }
    }

    pub(crate) fn invoker_failed(err: impl Into<BoxError>) -> Self {
        Self::InvokerFailed { source: err.into() }
    }

    pub(crate) fn multiple_stop(errors: Vec<Error>) -> Self {
        let count = errors.len();
        let summary = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        Self::MultipleStop(MultipleStopError {
            count,
            summary,
            errors,
        })
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Self::io("io", source)
    }
}

/// Wrap a user-facing fallible value with constructor/invoker context.
pub(crate) fn user_ctor_err<T: ?Sized>(err: impl Into<BoxError>) -> Error {
    Error::constructor_failed::<T>(err)
}

pub(crate) fn user_invoke_err(err: impl Into<BoxError>) -> Error {
    Error::invoker_failed(err)
}

/// Aggregate multiple hook failures into a single error.
pub(crate) fn aggregate_errors(errors: Vec<Error>) -> Result<()> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.into_iter().next().expect("len checked")),
        _ => Err(Error::multiple_stop(errors)),
    }
}

/// Prefer a cleanup failure as the primary error, while retaining the earlier
/// phase error when both fail.
pub(crate) fn combine_results(earlier: Result<()>, later: Result<()>) -> Result<()> {
    match (earlier, later) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(e)) | (Err(e), Ok(())) => Err(e),
        (Err(earlier), Err(later)) => Err(Error::CleanupAfterFailure {
            cleanup: Box::new(later),
            earlier: Box::new(earlier),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as StdError;

    #[test]
    fn hook_preserves_io_source() {
        let err = Error::hook(std::io::Error::other("disk full"));
        let src = StdError::source(&err).expect("source");
        assert!(src.to_string().contains("disk full"), "source was {src}");
    }

    #[test]
    fn constructor_failed_preserves_source() {
        let err = Error::constructor_failed::<u32>("nope");
        let src = StdError::source(&err).expect("source");
        assert!(src.to_string().contains("nope"), "source was {src}");
        assert!(
            err.to_string().contains("constructor for"),
            "display was {err}"
        );
    }

    #[test]
    fn io_helper_sets_context_and_source() {
        let err = Error::io("bind", std::io::Error::other("addr in use"));
        match &err {
            Error::Io { context, source } => {
                assert_eq!(context, "bind");
                assert!(source.to_string().contains("addr in use"));
            }
            other => panic!("expected Io, got {other}"),
        }
        assert!(StdError::source(&err).is_some());
    }

    #[test]
    fn from_io_error() {
        let err = Error::from(std::io::Error::other("nope"));
        match &err {
            Error::Io { context, .. } => assert_eq!(context, "io"),
            other => panic!("expected Io, got {other}"),
        }
    }

    #[test]
    fn cleanup_after_failure_source_is_cleanup() {
        let err = Error::CleanupAfterFailure {
            cleanup: Box::new(Error::hook("cleanup")),
            earlier: Box::new(Error::hook("earlier")),
        };
        let src = StdError::source(&err).expect("source");
        assert!(src.to_string().contains("cleanup"), "source was {src}");
    }

    #[test]
    fn multiple_stop_source_is_first_error() {
        let err = Error::multiple_stop(vec![Error::hook("stop-a"), Error::hook("stop-b")]);
        let src = StdError::source(&err).expect("source");
        assert!(src.to_string().contains("stop-a"), "source was {src}");
        match &err {
            Error::MultipleStop(inner) => {
                assert_eq!(inner.errors.len(), 2);
                assert!(inner.errors[1].to_string().contains("stop-b"));
            }
            other => panic!("expected MultipleStop, got {other}"),
        }
    }

    #[test]
    fn hook_display_includes_context() {
        let err = Error::hook("boom");
        let msg = err.to_string();
        assert!(msg.contains("boom"), "unexpected: {msg}");
        assert!(msg.contains("hook failed"), "unexpected: {msg}");
    }
}
