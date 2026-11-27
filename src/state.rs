//! The session file: what to restore on the next launch — the folder, each
//! dock's pane and width, the open documents and the window bounds.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The schema version. A file written by a different version is ignored rather
/// than migrated.
pub const SESSION_VERSION: u32 = 2;

/// What survives a restart.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionState {
    #[serde(default)]
    pub version: u32,
    /// The workspace folder, canonicalised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
    #[serde(default)]
    pub left: SideState,
    #[serde(default)]
    pub right: SideState,
    /// The open documents, in tab order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documents: Vec<SavedDocument>,
    /// The index in [`Self::documents`] that was on show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_document: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SavedBounds>,
}

/// One side dock's state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SideState {
    /// The pane id shown in the dock, if one ever was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// A missing field means open, so a session that predates the flag still
    /// shows its panes.
    #[serde(default = "open_by_default")]
    pub open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// The pane's own state, opaque here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<serde_json::Value>,
}

impl Default for SideState {
    fn default() -> Self {
        Self {
            active: None,
            open: true,
            width: None,
            state: None,
        }
    }
}

fn open_by_default() -> bool {
    true
}

/// A document that was open. Only the path decides how it reopens, so a build
/// with a better viewer for that type uses the better one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedDocument {
    pub path: PathBuf,
    /// The viewer's own state, opaque here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<serde_json::Value>,
}

/// A window position and size, in logical pixels, so the window keeps its
/// apparent size across displays of different DPI.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

impl SavedBounds {
    /// Whether these numbers can be used at all: the file is hand-editable, and
    /// a window restored at a nonsense value would be invisible.
    pub fn is_usable(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.
            && self.height > 0.
    }
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

/// Read the session, or `None` when there is nothing usable. Every failure is
/// logged and swallowed, because a corrupt session must not stop the app.
pub fn load() -> Option<SessionState> {
    let path = session_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("could not read {}: {error}", path.display());
            return None;
        }
    };

    match serde_json::from_str::<SessionState>(&text) {
        Ok(session) if session.is_current() => Some(session),
        Ok(session) => {
            log::warn!(
                "ignoring {}: it was written by session version {}, not {SESSION_VERSION}",
                path.display(),
                session.version
            );
            None
        }
        Err(error) => {
            log::warn!(
                "ignoring {}: it is not valid JSON ({error})",
                path.display()
            );
            None
        }
    }
}

/// Write the session, creating its directory if needed. Errors are logged:
/// failing to save a convenience file is not worth interrupting the user.
pub fn save(session: &SessionState) {
    let Some(path) = session_path() else {
        log::warn!("could not locate a config directory for the session file");
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        log::warn!("could not create {}: {error}", parent.display());
        return;
    }

    match serde_json::to_string_pretty(session) {
        Ok(text) => {
            if let Err(error) = std::fs::write(&path, text) {
                log::warn!("could not write {}: {error}", path.display());
            }
        }
        Err(error) => log::warn!("could not encode the session: {error}"),
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
            left: SideState {
                active: Some("explorer".to_owned()),
                open: true,
                width: Some(260.),
                state: None,
            },
            right: SideState {
                active: Some("browser".to_owned()),
                open: false,
                width: Some(420.),
                state: Some(serde_json::json!({ "url": "https://a.b" })),
            },
            documents: vec![SavedDocument {
                path: PathBuf::from("/work/notes.md"),
                view: Some(serde_json::json!({ "layout": "split" })),
            }],
            active_document: Some(0),
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
        assert!(session.documents.is_empty());
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
    fn a_layout_written_by_the_dock_is_not_current() {
        // Version 1 carried the dock's own layout, which this build cannot
        // rebuild; the shell falls back to the default layout instead.
        let v1: SessionState = serde_json::from_str(
            r#"{"version":1,"workspace_root":"/work","active_left":"explorer"}"#,
        )
        .expect("the old shape still parses");
        assert!(!v1.is_current());
    }

    #[test]
    fn a_side_without_the_open_flag_is_open() {
        let side: SideState = serde_json::from_str(r#"{"active":"explorer"}"#).expect("decode");
        assert!(side.open);
        assert_eq!(side.width, None);
        assert_eq!(side.state, None);
    }

    #[test]
    fn a_closed_side_round_trips_as_closed() {
        let session = SessionState {
            version: SESSION_VERSION,
            right: SideState {
                open: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = serde_json::to_string(&session).expect("encode");
        let back: SessionState = serde_json::from_str(&text).expect("decode");
        assert!(!back.right.open);
    }

    #[test]
    fn a_session_without_window_bounds_has_none() {
        // Written before the bounds were saved, or hand-edited without them.
        let session: SessionState = serde_json::from_str(r#"{"version":2}"#).expect("decode");
        assert_eq!(session.window, None);
    }

    #[test]
    fn a_document_without_view_state_keeps_none() {
        // Every tab written before viewers saved anything of their own.
        let session: SessionState =
            serde_json::from_str(r#"{"version":2,"documents":[{"path":"/work/a.rs"}]}"#)
                .expect("decode");
        assert_eq!(session.documents[0].view, None);
    }

    #[test]
    fn a_zero_sized_window_is_not_usable() {
        let empty = SavedBounds {
            x: 10.,
            y: 20.,
            width: 0.,
            height: 0.,
            maximized: false,
        };
        assert!(!empty.is_usable());

        let real = SavedBounds {
            width: 1200.,
            height: 760.,
            ..empty
        };
        assert!(real.is_usable());
    }
}
