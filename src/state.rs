//! The session file: what to restore on the next launch.
//!
//! Everything about the layout — open tabs, the active tab, the dock widths and
//! which docks are open — is already carried by the dock's own
//! `DockAreaState`. This adds the three things the dock does not know: which
//! folder was open, which pane each side dock was showing, and the window's
//! size.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The schema version. A file written by a different version is ignored rather
/// than migrated: the layout is a convenience, and a wrong one is worse than a
/// default.
pub const SESSION_VERSION: u32 = 1;

/// What survives a restart.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionState {
    /// The schema version this file was written by.
    #[serde(default)]
    pub version: u32,
    /// The workspace folder, canonicalised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
    /// The dock's own persisted layout: tabs, active tab, sizes, open docks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<gpui::base::dock::DockAreaState>,
    /// The pane id shown in the left dock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_left: Option<String>,
    /// The pane id shown in the right dock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_right: Option<String>,
    /// The window's saved bounds, if it had any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SavedBounds>,
}

/// A window position and size, in the platform's own units.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Whether the window was maximised, so the restore size is what it returns
    /// to when un-maximised.
    #[serde(default)]
    pub maximized: bool,
}

impl SessionState {
    /// Whether this file was written by a version this build understands.
    pub fn is_current(&self) -> bool {
        self.version == SESSION_VERSION
    }
}

/// The session file's path: `dirs::config_dir()/lllearning/session.json`.
pub fn session_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lllearning").join("session.json"))
}

/// Read the session, or `None` when there is nothing usable.
///
/// A missing file, an unreadable one, a malformed one and one from another
/// version all produce `None`, which the shell treats as "start fresh". A
/// corrupt session must never stop the app from starting (risk R12), so every
/// failure is logged and swallowed.
pub fn load() -> Option<SessionState> {
    let path = session_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            eprintln!("could not read {}: {error}", path.display());
            return None;
        }
    };

    match serde_json::from_str::<SessionState>(&text) {
        Ok(session) if session.is_current() => Some(session),
        Ok(session) => {
            eprintln!(
                "ignoring {}: it was written by session version {}, not {SESSION_VERSION}",
                path.display(),
                session.version
            );
            None
        }
        Err(error) => {
            eprintln!(
                "ignoring {}: it is not valid JSON ({error})",
                path.display()
            );
            None
        }
    }
}

/// Write the session, creating its directory if needed.
///
/// Errors are logged rather than returned: failing to save a convenience file
/// is not worth interrupting the user for.
pub fn save(session: &SessionState) {
    let Some(path) = session_path() else {
        eprintln!("could not locate a config directory for the session file");
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        eprintln!("could not create {}: {error}", parent.display());
        return;
    }

    match serde_json::to_string_pretty(session) {
        Ok(text) => {
            if let Err(error) = std::fs::write(&path, text) {
                eprintln!("could not write {}: {error}", path.display());
            }
        }
        Err(error) => eprintln!("could not encode the session: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_session_round_trips() {
        let session = SessionState {
            version: SESSION_VERSION,
            workspace_root: Some(PathBuf::from("/work")),
            dock: None,
            active_left: Some("explorer".to_owned()),
            active_right: Some("browser".to_owned()),
            window: Some(SavedBounds {
                x: 10.,
                y: 20.,
                width: 1200.,
                height: 760.,
                maximized: false,
            }),
        };

        let text = serde_json::to_string(&session).expect("encode");
        let back: SessionState = serde_json::from_str(&text).expect("decode");
        assert_eq!(back, session);
    }

    #[test]
    fn an_empty_object_is_a_fresh_session() {
        let session: SessionState = serde_json::from_str("{}").expect("decode");
        assert_eq!(session.version, 0);
        assert_eq!(session.workspace_root, None);
        assert_eq!(session.dock, None);
        assert!(!session.is_current(), "an unversioned file is not current");
    }

    #[test]
    fn a_malformed_file_is_rejected_rather_than_panicking() {
        let parsed = serde_json::from_str::<SessionState>("{ not json");
        assert!(parsed.is_err());
    }

    #[test]
    fn a_version_mismatch_is_not_current() {
        let session = SessionState {
            version: SESSION_VERSION + 1,
            ..Default::default()
        };
        assert!(!session.is_current());
    }

    #[test]
    fn a_dock_state_round_trips_inside_the_session() {
        // The dock's state is the bulk of the file, so it is worth proving the
        // nesting works rather than only the outer shape.
        let dock = gpui::base::dock::DockAreaState::default();
        let session = SessionState {
            version: SESSION_VERSION,
            dock: Some(dock),
            ..Default::default()
        };
        let text = serde_json::to_string(&session).expect("encode");
        let back: SessionState = serde_json::from_str(&text).expect("decode");
        assert!(back.dock.is_some());
        assert!(back.is_current());
    }
}
