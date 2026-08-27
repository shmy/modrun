use std::borrow::Cow;
use std::io::IsTerminal;

/// Default ASCII banner printed at startup unless
/// [`crate::ModrunBuilder::no_banner`] is called.
///
/// The default banner is written to stderr and only when stderr is a TTY, so
/// piped or daemon stdout stays clean. Custom text from
/// [`crate::ModrunBuilder::banner`] is always written to stderr.
pub const DEFAULT_BANNER: &str = r"
  __  __           _                  
 |  \/  | ___   __| |_ __ _   _ _ __  
 | |\/| |/ _ \ / _` | '__| | | | '_ \ 
 | |  | | (_) | (_| | |  | |_| | | | |
 |_|  |_|\___/ \__,_|_|   \__,_|_| |_|
";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Banner {
    #[default]
    Default,
    Custom(Cow<'static, str>),
    Off,
}

pub(crate) fn emit(banner: &Banner) {
    match banner {
        Banner::Off => {}
        Banner::Default => print_default(),
        Banner::Custom(text) => print(text),
    }
}

fn print_default() {
    if !std::io::stderr().is_terminal() {
        return;
    }
    print(DEFAULT_BANNER);
    eprintln!(
        ":: modrun :: v{} :: Lightweight wiring for Tokio services",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!();
}

fn print(text: &str) {
    let body = text.trim_end_matches('\n');
    if body.is_empty() {
        return;
    }
    eprintln!("{body}");
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_banner_contains_name() {
        assert!(DEFAULT_BANNER.contains("modrun") || DEFAULT_BANNER.contains('|'));
    }

    #[test]
    fn off_emits_nothing() {
        emit(&Banner::Off);
    }
}
