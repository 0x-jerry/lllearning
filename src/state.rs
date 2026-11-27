//! The session file: what to restore on the next launch.
//!
//! The layout used to be gpui's own `DockAreaState`, which the dock crate could
//! serialise and rebuild. The dock is gone — the shell lays the window out
//! itself — so the layout is described here, in terms the shell can rebuild:
//! which folder was open, which pane each side dock was showing, whether each
//! dock was open and how wide, which documents were open in tab order, which one
//! was on show, and where the window sat on screen.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The schema version. A file written by a different version is ignored rather
/// than migrated: the layout is a convenience, and a wrong one is worse than a
/// default. Version 2 is the first written by the shell itself; a version 1 file
/// was the dock's layout and is not rebuildable.
pub const SESSION_VERSION: u32 = 2;

/// What survives a restart.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionState {
    /// The schema version this file was written by.
    #[serde(default)]
    pub version: u32,
    /// The workspace folder, canonicalised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
    /// The left dock: which pane it shows, whether it is open, how wide.
    #[serde(default)]
    pub left: SideState,
    /// The right dock, the same three answers.
    #[serde(default)]
    pub right: SideState,
    /// The open documents, in tab order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documents: Vec<SavedDocument>,
    /// The index in [`Self::documents`] that was on show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_document: Option<usize>,
    /// The window's saved bounds, if it had any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SavedBounds>,
}

/// One side dock's state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SideState {
    /// The pane id shown in the dock, if one ever was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// Whether the dock is open. A missing field means open, so a session that
    /// predates the flag still shows its panes.
    #[serde(default = "open_by_default")]
    pub open: bool,
    /// The dock's width, if it has been laid out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
}

impl Default for SideState {
    fn default() -> Self {
        Self {
            active: None,
            open: true,
            width: None,
        }
    }
}

fn open_by_default() -> bool {
    true
}

/// A document that was open.
///
/// The path is kept, and only the path decides how it reopens: the viewer is
/// chosen by scoring the file's type again, so a build that gained a better
/// viewer for that type opens it with the better one instead of replaying a
/// stale choice. What the viewer itself remembers — the markdown viewer's
/// layout, today — travels beside it in [`Self::view`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedDocument {
    pub path: PathBuf,
    /// The viewer's own state, opaque here: only the viewer that wrote it knows
    /// what it means (see `documents::ViewState`). Absent for a viewer that has
    /// nothing to keep, and for every tab written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<serde_json::Value>,
}

/// A window position and size, in logical pixels.
///
/// Logical rather than device pixels, so a session moves between a high-DPI
/// display and a normal one without the window changing apparent size.
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

impl SavedBounds {
    /// Whether these numbers can be used as a window position and size at all.
    ///
    /// The session file is plain JSON and hand-editable, so a nonsense value is
    /// a possibility rather than a bug; a window restored at one would be
    /// invisible with no way to bring it back.
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

/// Write the session, creating its directory if needed.
///
/// Errors are logged rather than returned: failing to save a convenience file
/// is not worth interrupting the user for.
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
            },
            right: SideState {
                active: Some("browser".to_owned()),
                open: false,
                width: Some(420.),
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
