//! Icons and other files bundled into the app.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Path of the git icon, for [`gpui_component::Icon::path`].
pub const GIT_ICON: &str = "icons/git.svg";

/// The app's own icons, plus the icon set of the UI component library.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            GIT_ICON => Ok(Some(Cow::Borrowed(include_bytes!("../assets/icons/git.svg")))),
            _ => gpui_component_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        if GIT_ICON.starts_with(path) {
            paths.push(GIT_ICON.into());
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_app_and_library_icons() {
        let git = AppAssets.load(GIT_ICON).expect("load").expect("present");
        assert!(git.starts_with(b"<svg"));
        assert!(AppAssets.load("icons/file.svg").expect("load").is_some());
        assert!(AppAssets.list("icons/").expect("list").iter().any(|p| p == GIT_ICON));
    }
}
