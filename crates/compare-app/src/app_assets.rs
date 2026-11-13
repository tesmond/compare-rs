//! Icons and other files bundled into the app.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Paths of the Git compare icons, for [`gpui_component::Icon::path`].
pub const GIT_HISTORY_ICON: &str = "icons/git-history.svg";
pub const GIT_BRANCHES_ICON: &str = "icons/git-branches.svg";

/// The app's own icons, plus the icon set of the UI component library.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            GIT_HISTORY_ICON => {
                Ok(Some(Cow::Borrowed(include_bytes!("../assets/icons/git-history.svg"))))
            }
            GIT_BRANCHES_ICON => {
                Ok(Some(Cow::Borrowed(include_bytes!("../assets/icons/git-branches.svg"))))
            }
            _ => gpui_component_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        for icon in [GIT_HISTORY_ICON, GIT_BRANCHES_ICON] {
            if icon.starts_with(path) {
                paths.push(icon.into());
            }
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_app_and_library_icons() {
        let paths = AppAssets.list("icons/").expect("list");
        for icon in [GIT_HISTORY_ICON, GIT_BRANCHES_ICON] {
            let svg = AppAssets.load(icon).expect("load").expect("present");
            assert!(svg.starts_with(b"<svg"));
            assert!(paths.iter().any(|p| p == icon));
        }
        assert!(AppAssets.load("icons/file.svg").expect("load").is_some());
    }
}
