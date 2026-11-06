//! Window size and open tabs, saved between launches.

use std::fs;
use std::io;
use std::path::PathBuf;

use gpui::{Bounds, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use crate::app_dirs;

const SESSION_FILE: &str = "session.json";

/// Smallest window dimension accepted when restoring, so a corrupt file
/// cannot produce an unusable window.
const MIN_WINDOW_SIDE: f32 = 300.0;

/// Everything restored on the next launch. Fields missing from older files
/// take their default values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub window: Option<SavedBounds>,
    pub tabs: Vec<SavedTab>,
    pub active_tab: usize,
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub beautify: bool,
    pub word_wrap: bool,
}

/// The main window's placement.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

/// The inputs of one tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    pub merge: bool,
    pub left: Option<PathBuf>,
    pub base: Option<PathBuf>,
    pub right: Option<PathBuf>,
}

impl SavedBounds {
    pub fn from_window_bounds(bounds: WindowBounds) -> Self {
        let (rect, maximized) = match bounds {
            WindowBounds::Windowed(rect) => (rect, false),
            WindowBounds::Maximized(rect) | WindowBounds::Fullscreen(rect) => (rect, true),
        };
        Self {
            x: f32::from(rect.origin.x),
            y: f32::from(rect.origin.y),
            width: f32::from(rect.size.width),
            height: f32::from(rect.size.height),
            maximized,
        }
    }

    /// Converts back to window bounds, or `None` if the saved size is unusable.
    pub fn to_window_bounds(self) -> Option<WindowBounds> {
        if !(self.width >= MIN_WINDOW_SIDE && self.height >= MIN_WINDOW_SIDE) {
            return None;
        }
        let rect = Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        };
        Some(if self.maximized {
            WindowBounds::Maximized(rect)
        } else {
            WindowBounds::Windowed(rect)
        })
    }
}

fn session_path() -> Option<PathBuf> {
    app_dirs::support_dir().map(|dir| dir.join(SESSION_FILE))
}

/// Loads the saved session; a missing or unreadable file yields `None`.
pub fn load() -> Option<SessionState> {
    let text = fs::read_to_string(session_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes the session atomically (write to a temporary file, then rename).
pub fn save(state: &SessionState) -> io::Result<()> {
    let Some(path) = session_path() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json)?;
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let state = SessionState {
            window: Some(SavedBounds {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
                maximized: false,
            }),
            tabs: vec![SavedTab {
                merge: false,
                left: Some("/a".into()),
                base: None,
                right: Some("/b".into()),
            }],
            active_tab: 0,
            ignore_whitespace: true,
            ignore_case: false,
            beautify: true,
            word_wrap: true,
        };
        let json = serde_json::to_string(&state).expect("serialize");
        assert_eq!(serde_json::from_str::<SessionState>(&json).expect("deserialize"), state);
    }

    #[test]
    fn loads_files_written_before_newer_fields() {
        let old = r#"{"window": null, "tabs": [], "active_tab": 0, "ignore_whitespace": true, "ignore_case": false}"#;
        let state: SessionState = serde_json::from_str(old).expect("old sessions still load");
        assert!(state.ignore_whitespace);
        assert!(!state.beautify);
        assert!(!state.word_wrap);
    }

    #[test]
    fn rejects_tiny_windows() {
        let bounds = SavedBounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0, maximized: false };
        assert!(bounds.to_window_bounds().is_none());
    }
}
