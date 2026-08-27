use crate::error::Result;

/// Opaque option applied while building a [`crate::ModrunBuilder`].
pub(crate) trait ModOption: Send {
    fn apply(self: Box<Self>, app: &mut crate::app::BuildState) -> Result<()>;
}
