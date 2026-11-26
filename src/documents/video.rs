//! The video viewer.
//!
//! GPUI has no native video element (risk R4), so this is a placeholder that
//! exercises the extension point honestly: it claims video extensions, explains
//! why it cannot play them, and offers to open the file in the right-hand
//! browser pane, which can.
//!
//! When GPUI grows a video element, or the browser pane is taught to load a
//! `file://` URL, this module is the only one that changes.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::Button,
    empty::{
        Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant,
        EmptyTitle,
    },
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use super::{DocumentContent, DocumentController, DocumentProvider, OpenContext, Sniff, sniff};

/// Video loses to image (they never collide) and beats markdown.
const SCORE: u8 = 80;

/// The video viewer.
pub struct VideoProvider;

impl DocumentProvider for VideoProvider {
    fn id(&self) -> &'static str {
        "video"
    }

    fn title(&self) -> &'static str {
        "Video"
    }

    fn score(&self, _path: &Path, sniff: &Sniff) -> Option<u8> {
        (sniff::kind_of(sniff.ext.as_deref()) == Some(sniff::FileKind::Video)).then_some(SCORE)
    }

    fn open(&self, ctx: OpenContext, window: &mut Window, cx: &mut App) -> DocumentContent {
        let _ = window;
        let view = cx.new(|cx| VideoView::new(ctx.path, ctx.shell, cx));
        let controller: Rc<dyn DocumentController> =
            Rc::new(VideoController { view: view.clone() });
        DocumentContent::static_content(view.into(), controller)
    }
}

struct VideoController {
    view: Entity<VideoView>,
}

impl DocumentController for VideoController {
    fn is_dirty(&self, _cx: &App) -> bool {
        false
    }

    fn save(&self, _window: &mut Window, _cx: &mut App) -> Result<(), String> {
        Ok(())
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| view.focus(window, cx));
    }

    fn reload_from_disk(&self, _window: &mut Window, _cx: &mut App) {}

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle(cx)
    }

    fn is_stale(&self, _cx: &App) -> bool {
        false
    }

    fn accept_disk_change(&self, _window: &mut Window, _cx: &mut App) {}

    fn language(&self, _cx: &App) -> Option<SharedString> {
        Some(SharedString::from("Video"))
    }
}

/// The placeholder view.
pub struct VideoView {
    path: PathBuf,
    focus_handle: FocusHandle,
    shell: gpui::WeakEntity<crate::app::shell::AppShell>,
}

impl VideoView {
    fn new(
        path: PathBuf,
        shell: gpui::WeakEntity<crate::app::shell::AppShell>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            path,
            focus_handle: cx.focus_handle(),
            shell,
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    /// Show the file in the browser pane, which has a real video engine.
    fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        let url = file_url(&self.path);
        let shell = self.shell.clone();
        // The browser pane lives in the shell, so the request goes through it
        // rather than reaching into the pane directly.
        let _ = shell.update(cx, |shell, cx| shell.navigate_browser(url, cx));
    }
}

impl Focusable for VideoView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for VideoView {}

impl Render for VideoView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = crate::workspace::fs_util::file_name_of(&self.path);

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                Empty::new()
                    .header(
                        EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .child(Icon::new(gpui::assets::IconName::Video)),
                            )
                            .title(EmptyTitle::new().child(SharedString::from(name)))
                            .description(EmptyDescription::new().child(
                                "GPUI has no video element yet, so this file cannot be \
                                 played here.",
                            )),
                    )
                    .content(
                        EmptyContent::new().child(
                            Button::new("video-open-browser")
                                .label("Open in browser pane")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.open_in_browser(cx))),
                        ),
                    ),
            )
            .when(self.path.exists(), |this| this)
            .text_color(cx.theme().foreground)
    }
}

/// A `file://` URL for `path`.
///
/// Windows paths need the drive letter and forward slashes: `C:\a\b.mp4` becomes
/// `file:///C:/a/b.mp4`, with the leading slash that the URL grammar requires.
pub fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_posix_path_becomes_a_file_url() {
        assert_eq!(file_url(Path::new("/tmp/a.mp4")), "file:///tmp/a.mp4");
    }

    #[test]
    fn a_windows_path_keeps_its_drive_letter() {
        assert_eq!(
            file_url(Path::new(r"C:\clips\a.mp4")),
            "file:///C:/clips/a.mp4"
        );
    }
}
