//! The document seam: which viewer opens a file, and how it is driven.
//!
//! A viewer is chosen by scoring every registered provider against what was
//! sniffed from the disk. Adding one is a module plus a `register_document_provider`
//! call.
//!
//! Some members carry `#[allow(dead_code)]`: they are seam surface a test-drives
//! or a future caller uses, which the lint cannot see.

pub mod image;
pub mod markdown;
pub mod markdown_outline;
pub mod sniff;
pub mod status_bar_helper;
pub mod text;

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::{
    AnyView, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, WeakEntity,
    Window, div,
};
use gpui_rsx::rsx;

use crate::{
    app::shell::AppShell,
    workspace::{Workspace, fs_util},
};

/// What was learned about a file before deciding who opens it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sniff {
    /// The lowercased extension, if the name has one.
    pub ext: Option<String>,
    /// The file's size in bytes.
    pub len: u64,
    /// Whether the head of the file looks binary.
    pub is_binary: bool,
}

impl Sniff {
    /// Sniff `path` from the disk.
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            ext: fs_util::extension_of(path),
            len: metadata.len(),
            is_binary: fs_util::is_binary_file(path).unwrap_or(false),
        })
    }

    /// Whether the file is too large for the text viewer.
    pub fn is_oversized(&self) -> bool {
        self.len > fs_util::MAX_TEXT_BYTES
    }

    /// The extension, or `""`.
    #[allow(dead_code)]
    pub fn ext_str(&self) -> &str {
        self.ext.as_deref().unwrap_or("")
    }
}

/// One line of a document's outline: `line` is 1-based, `depth` is indentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry {
    pub label: String,
    pub depth: usize,
    pub line: u32,
}

/// What a viewer is handed when it opens a file.
pub struct OpenContext {
    /// The path, canonicalised.
    pub path: PathBuf,
    pub sniff: Sniff,
    /// For a viewer that drives the rest of the app, e.g. asks for a session write.
    pub shell: WeakEntity<AppShell>,
    #[allow(dead_code)]
    pub workspace: Entity<Workspace>,
}

/// What a viewer remembers about itself between runs, opaque to the shell.
///
/// A value this build cannot decode means the viewer falls back to its defaults,
/// never that the open fails.
pub type ViewState = serde_json::Value;

/// Object-safe handle over a document viewer, so the shell can drive any of
/// them without knowing its type.
pub trait DocumentController {
    /// Whether the buffer has unsaved changes.
    fn is_dirty(&self, cx: &App) -> bool;

    /// Write the buffer back to its file.
    fn save(&self, window: &mut Window, cx: &mut App) -> Result<(), String>;

    /// Give the viewer keyboard focus.
    fn focus(&self, window: &mut Window, cx: &mut App);

    /// Re-read the file from disk, discarding the buffer.
    fn reload_from_disk(&self, window: &mut Window, cx: &mut App);

    /// The focus handle the shell focuses when this tab is selected.
    fn focus_handle(&self, cx: &App) -> FocusHandle;

    /// Whether the file changed on disk under an unmodified buffer.
    #[allow(dead_code)]
    fn is_stale(&self, cx: &App) -> bool;

    /// Accept the on-disk version, clearing the stale marker.
    #[allow(dead_code)]
    fn accept_disk_change(&self, window: &mut Window, cx: &mut App);

    /// Note that the file changed on disk while the buffer had unsaved edits.
    /// The buffer is not touched: the panel shows a banner and the user decides.
    fn mark_stale(&self, cx: &mut App) {
        let _ = cx;
    }

    /// The language the file is in, if the viewer has one. Nothing draws it yet.
    #[allow(dead_code)]
    fn language(&self, cx: &App) -> Option<SharedString> {
        let _ = cx;
        None
    }

    /// The caret's 1-based line and column, when the viewer reports one.
    #[allow(dead_code)]
    fn cursor(&self, cx: &App) -> Option<(u32, u32)> {
        let _ = cx;
        None
    }

    /// The state the viewer wants restored next time, or `None` when the file is
    /// all it needs.
    fn view_state(&self, cx: &App) -> Option<ViewState> {
        let _ = cx;
        None
    }

    /// Apply what [`Self::view_state`] returned on the previous run, once, after
    /// the viewer is built. A value this build cannot read is ignored.
    fn restore_view_state(&self, state: &ViewState, window: &mut Window, cx: &mut App) {
        let _ = (state, window, cx);
    }

    /// The outline of the document, if this viewer has one: `None` when it cannot
    /// outline itself, `Some(vec![])` when it can and the file has no entries.
    ///
    /// It lives on the controller because only the controller holds the live
    /// buffer.
    fn outline(&self, cx: &App) -> Option<Vec<OutlineEntry>> {
        let _ = cx;
        None
    }

    /// Move the caret to `line` (1-based), for an outline jump.
    fn goto_line(&self, line: u32, window: &mut Window, cx: &mut App) {
        let _ = (line, window, cx);
    }
}

/// What a provider returns: the view and its controller.
pub struct DocumentContent {
    pub view: AnyView,
    pub controller: Rc<dyn DocumentController>,
    /// Subscribes the panel to its viewer's notifications, re-broadcast as
    /// [`DocumentEvent`]. `None` for a viewer whose contents never change under
    /// the user.
    ///
    /// The handle has to be typed because GPUI has no observe-by-id, and only the
    /// provider knows the concrete type.
    pub watch: Option<WatchView>,
}

/// A provider's way to subscribe the panel to its own view.
pub type WatchView = Box<dyn Fn(&mut Context<DocumentPanel>) -> Subscription>;

impl DocumentContent {
    /// Content with no live updates: the view is drawn and nothing watches it.
    pub fn static_content(view: AnyView, controller: Rc<dyn DocumentController>) -> Self {
        Self {
            view,
            controller,
            watch: None,
        }
    }
}

/// A kind of document viewer.
pub trait DocumentProvider: 'static {
    /// Stable id, written into the persisted layout. Never rename it.
    fn id(&self) -> &'static str;

    /// The name the viewer goes by, where something names it rather than the
    /// file.
    #[allow(dead_code)]
    fn title(&self) -> &'static str;

    /// How well this provider can show `path`. `None` means it cannot.
    ///
    /// Higher wins. The text provider scores low and matches almost anything,
    /// so a more specific provider always beats it.
    fn score(&self, path: &Path, sniff: &Sniff) -> Option<u8>;

    /// Build the viewer for `ctx.path`.
    fn open(&self, ctx: OpenContext, window: &mut Window, cx: &mut App) -> DocumentContent;
}

/// Every registered viewer.
#[derive(Default)]
pub struct DocumentRegistry {
    providers: Vec<Rc<dyn DocumentProvider>>,
}

impl Global for DocumentRegistry {}

impl DocumentRegistry {
    /// Every provider, in registration order.
    #[allow(dead_code)]
    pub fn providers(&self) -> &[Rc<dyn DocumentProvider>] {
        &self.providers
    }

    /// The best provider for a file, by score.
    ///
    /// Ties go to the provider registered first, so the built-in set's order is
    /// meaningful: image before markdown before text.
    pub fn best_for(&self, path: &Path, sniff: &Sniff) -> Option<Rc<dyn DocumentProvider>> {
        let mut best: Option<(u8, &Rc<dyn DocumentProvider>)> = None;
        for provider in &self.providers {
            if let Some(score) = provider.score(path, sniff)
                && best.is_none_or(|(best_score, _)| score > best_score)
            {
                best = Some((score, provider));
            }
        }
        best.map(|(_, provider)| provider.clone())
    }

    /// A provider by id.
    #[allow(dead_code)]
    pub fn by_id(&self, id: &str) -> Option<Rc<dyn DocumentProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.id() == id)
            .cloned()
    }
}

/// Add a document viewer.
pub fn register_document_provider(cx: &mut App, provider: Rc<dyn DocumentProvider>) {
    if cx.try_global::<DocumentRegistry>().is_none() {
        cx.set_global(DocumentRegistry::default());
    }
    cx.global_mut::<DocumentRegistry>().providers.push(provider);
}

/// The best provider for `path`, or `None` when nothing can show it.
pub fn provider_for(cx: &App, path: &Path, sniff: &Sniff) -> Option<Rc<dyn DocumentProvider>> {
    cx.try_global::<DocumentRegistry>()
        .and_then(|registry| registry.best_for(path, sniff))
}

/// A provider by id, for a caller that has one recorded rather than a file to
/// score.
#[allow(dead_code)]
pub fn provider_by_id(cx: &App, id: &str) -> Option<Rc<dyn DocumentProvider>> {
    cx.try_global::<DocumentRegistry>()
        .and_then(|registry| registry.by_id(id))
}

/// Every registered provider, in registration order.
#[allow(dead_code)]
pub fn providers(cx: &App) -> Vec<Rc<dyn DocumentProvider>> {
    cx.try_global::<DocumentRegistry>()
        .map(|registry| registry.providers().to_vec())
        .unwrap_or_default()
}

/// Register the viewers this build ships.
///
/// Order matters only for ties, and the scores are chosen so there are none.
pub fn register_builtins(cx: &mut App) {
    register_document_provider(cx, Rc::new(image::ImageProvider));
    register_document_provider(cx, Rc::new(markdown::MarkdownProvider));
    register_document_provider(cx, Rc::new(text::TextProvider));
    // Scores below text and claims only what text refused, so a binary file
    // opens an explanation rather than nothing.
    register_document_provider(cx, Rc::new(refused::RefusedProvider));
}

/// The viewer that stands in for a file no real viewer will open.
pub mod refused {
    use std::{
        path::{Path, PathBuf},
        rc::Rc,
    };

    use gpui::component::{
        Icon,
        empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle},
    };
    use gpui::{
        App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
        ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
    };
    use gpui_rsx::rsx;

    use super::{DocumentContent, DocumentController, DocumentProvider, OpenContext, Sniff, sniff};

    /// Scores just below the text viewer, so it only wins where text declined.
    const SCORE: u8 = 1;

    /// Claims binary and oversized files so they open an explanation.
    pub struct RefusedProvider;

    impl DocumentProvider for RefusedProvider {
        fn id(&self) -> &'static str {
            "refused"
        }

        fn title(&self) -> &'static str {
            "Unsupported"
        }

        fn score(&self, _path: &Path, sniff: &Sniff) -> Option<u8> {
            sniff::text_refusal(sniff).map(|_| SCORE)
        }

        fn open(&self, ctx: OpenContext, window: &mut Window, cx: &mut App) -> DocumentContent {
            let refusal = sniff::text_refusal(&ctx.sniff)
                .map(|refusal| refusal.message())
                .unwrap_or_else(|| "This file cannot be shown.".to_owned());
            let view = cx.new(|cx| RefusedView::new(ctx.path, refusal, cx));
            let _ = window;
            let controller: Rc<dyn DocumentController> =
                Rc::new(RefusedController { view: view.clone() });
            DocumentContent::static_content(view.into(), controller)
        }
    }

    struct RefusedController {
        view: Entity<RefusedView>,
    }

    impl DocumentController for RefusedController {
        fn is_dirty(&self, _cx: &App) -> bool {
            false
        }

        fn save(&self, _window: &mut Window, _cx: &mut App) -> Result<(), String> {
            Ok(())
        }

        fn focus(&self, window: &mut Window, cx: &mut App) {
            self.view.update(cx, |view, cx| {
                view.focus_handle.focus(window, cx);
            });
        }

        fn reload_from_disk(&self, _window: &mut Window, cx: &mut App) {
            self.view.update(cx, |_, cx| cx.notify());
        }

        fn focus_handle(&self, cx: &App) -> FocusHandle {
            self.view.read(cx).focus_handle(cx)
        }

        fn is_stale(&self, _cx: &App) -> bool {
            false
        }

        fn accept_disk_change(&self, _window: &mut Window, _cx: &mut App) {}

        fn language(&self, _cx: &App) -> Option<SharedString> {
            Some(SharedString::from("Binary"))
        }
    }

    /// The explanation panel.
    pub struct RefusedView {
        path: PathBuf,
        message: String,
        focus_handle: FocusHandle,
    }

    impl RefusedView {
        fn new(path: PathBuf, message: String, cx: &mut Context<Self>) -> Self {
            Self {
                path,
                message,
                focus_handle: cx.focus_handle(),
            }
        }
    }

    impl Focusable for RefusedView {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus_handle.clone()
        }
    }

    impl EventEmitter<()> for RefusedView {}

    impl Render for RefusedView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let name = crate::workspace::fs_util::file_name_of(&self.path);
            let message = self.message.clone();

            rsx! {
                <div class="flex size-full items-center justify-center p-6">
                    <Empty
                        base={Empty::new()}
                        header={EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .child(Icon::new(gpui::component::IconName::File)),
                            )
                            .title(EmptyTitle::new().child(SharedString::from(name)))
                            .description(EmptyDescription::new().child(SharedString::from(message)))}
                    />
                </div>
            }
            .into_any_element()
        }
    }
}

/// The tab title for a document: the file name, with a dot when dirty.
pub fn tab_title(path: &Path, dirty: bool) -> SharedString {
    let name = fs_util::file_name_of(path);
    if dirty {
        SharedString::from(format!("{name} •"))
    } else {
        SharedString::from(name)
    }
}

/// A tab that shows one document. The viewer is type-erased through
/// [`DocumentController`].
pub struct DocumentPanel {
    path: PathBuf,
    provider_id: &'static str,
    controller: Rc<dyn DocumentController>,
    view: AnyView,
    /// Dropped with the panel, so the viewer's notifications stop reaching it.
    _watch: Option<Subscription>,
}

impl EventEmitter<DocumentEvent> for DocumentPanel {}

/// The panel's own event: its contents changed, re-broadcast from its viewer so
/// a pane can watch the active document without naming its type.
#[derive(Clone, Copy, Debug)]
pub struct DocumentEvent;

impl Focusable for DocumentPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.controller.focus_handle(cx)
    }
}

impl DocumentPanel {
    /// Open `path` with the best provider, or `None` when none can show it.
    pub fn open(
        path: PathBuf,
        shell: WeakEntity<AppShell>,
        workspace: Entity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<Self>> {
        let sniff = match Sniff::of(&path) {
            Ok(sniff) => sniff,
            Err(error) => {
                log::error!("could not read {}: {error}", path.display());
                return None;
            }
        };
        let provider = provider_for(cx, &path, &sniff)?;
        let provider_id = provider.id();
        let content = provider.open(
            OpenContext {
                path: path.clone(),
                sniff,
                shell,
                workspace,
            },
            window,
            cx,
        );

        Some(cx.new(|cx| {
            // The provider's hook subscribes to its own view and hands the
            // subscription back to be held here.
            let watch = content.watch.as_ref().map(|watch| watch(cx));
            Self {
                path,
                provider_id,
                controller: content.controller,
                view: content.view,
                _watch: watch,
            }
        }))
    }

    /// The file this document shows.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The provider id.
    #[allow(dead_code)]
    pub fn provider_id(&self) -> &'static str {
        self.provider_id
    }

    /// The viewer's controller.
    pub fn controller(&self) -> &Rc<dyn DocumentController> {
        &self.controller
    }

    /// Whether the buffer has unsaved changes.
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.controller.is_dirty(cx)
    }

    /// The tab title, with a dirty marker.
    pub fn tab_title(&self, cx: &App) -> SharedString {
        tab_title(&self.path, self.is_dirty(cx))
    }
}

impl Render for DocumentPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.view.clone();
        rsx! { <div class="size-full">{view}</div> }
    }
}

/// The stand-in for a document that could not be reopened: its file is gone, or
/// its viewer is. The tab is kept rather than dropped.
pub struct MissingDocument {
    path: PathBuf,
    focus_handle: FocusHandle,
}

impl MissingDocument {
    fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        Self {
            path,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for MissingDocument {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MissingDocument {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let name = fs_util::file_name_of(&self.path);
        let message = SharedString::from(format!(
            "{name} could not be reopened; its viewer is not available."
        ));

        rsx! {
            <div class="flex size-full items-center justify-center">
                {message}
            </div>
        }
    }
}

/// Drives [`MissingDocument`] like any other viewer.
struct MissingController {
    view: Entity<MissingDocument>,
}

impl DocumentController for MissingController {
    fn is_dirty(&self, _cx: &App) -> bool {
        false
    }

    fn save(&self, _window: &mut Window, _cx: &mut App) -> Result<(), String> {
        Ok(())
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.focus_handle.focus(window, cx);
        });
    }

    fn reload_from_disk(&self, _window: &mut Window, cx: &mut App) {
        self.view.update(cx, |_, cx| cx.notify());
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle(cx)
    }

    fn is_stale(&self, _cx: &App) -> bool {
        false
    }

    fn accept_disk_change(&self, _window: &mut Window, _cx: &mut App) {}

    fn language(&self, _cx: &App) -> Option<SharedString> {
        Some(SharedString::from("Unavailable"))
    }
}

impl DocumentPanel {
    /// A tab for a file that could not be reopened.
    pub fn missing(path: PathBuf, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| MissingDocument::new(path.clone(), cx));
        cx.new(|_| Self {
            path,
            provider_id: "missing",
            controller: Rc::new(MissingController { view: view.clone() }),
            view: view.into(),
            _watch: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sniff(ext: Option<&str>, len: u64, is_binary: bool) -> Sniff {
        Sniff {
            ext: ext.map(str::to_owned),
            len,
            is_binary,
        }
    }

    fn registry() -> DocumentRegistry {
        let mut registry = DocumentRegistry::default();
        registry.providers.push(Rc::new(image::ImageProvider));
        registry.providers.push(Rc::new(markdown::MarkdownProvider));
        registry.providers.push(Rc::new(text::TextProvider));
        registry.providers.push(Rc::new(refused::RefusedProvider));
        registry
    }

    /// The score table decides what opens a file, so it is asserted directly.
    #[test]
    fn each_file_type_lands_on_the_viewer_it_should() {
        let registry = registry();

        let cases: [(Option<&str>, bool, Option<&str>); 8] = [
            (Some("png"), false, Some("image")),
            (Some("jpg"), false, Some("image")),
            // No video viewer: video files are binary, so the panel explains.
            (Some("mp4"), true, Some("refused")),
            (Some("webm"), true, Some("refused")),
            (Some("md"), false, Some("markdown")),
            (Some("rs"), false, Some("text")),
            (Some("txt"), false, Some("text")),
            // No extension at all is still text.
            (None, false, Some("text")),
        ];

        for (ext, is_binary, expected) in cases {
            let sniff = sniff(ext, 128, is_binary);
            let provider = registry.best_for(Path::new("a"), &sniff);
            assert_eq!(
                provider.map(|provider| provider.id()),
                expected,
                "extension {ext:?} should open {expected:?}"
            );
        }
    }

    #[test]
    fn a_binary_file_opens_the_explanation_panel() {
        let registry = registry();
        let provider = registry
            .best_for(Path::new("a.bin"), &sniff(Some("bin"), 128, true))
            .expect("a binary file still opens something");
        assert_eq!(provider.id(), "refused");
    }

    #[test]
    fn an_oversized_file_opens_the_explanation_panel() {
        let registry = registry();
        let len = crate::workspace::fs_util::MAX_TEXT_BYTES + 1;
        let provider = registry
            .best_for(Path::new("big.txt"), &sniff(Some("txt"), len, false))
            .expect("an oversized file still opens something");
        assert_eq!(provider.id(), "refused");
    }

    #[test]
    fn a_binary_image_extension_still_opens_the_image_viewer() {
        // The image viewer does not read the file as text, so a NUL in the head
        // must not push a PNG to the refusal panel.
        let registry = registry();
        let provider = registry
            .best_for(Path::new("a.png"), &sniff(Some("png"), 128, true))
            .expect("an image opens the image viewer");
        assert_eq!(provider.id(), "image");
    }

    #[test]
    fn a_provider_is_found_by_id() {
        let registry = registry();
        assert!(registry.by_id("text").is_some());
        assert!(registry.by_id("image").is_some());
        assert!(registry.by_id("nope").is_none());
    }

    #[test]
    fn a_dirty_tab_title_carries_a_marker() {
        let clean = tab_title(Path::new("/work/a.rs"), false);
        let dirty = tab_title(Path::new("/work/a.rs"), true);
        assert_eq!(clean.as_ref(), "a.rs");
        assert!(dirty.as_ref().starts_with("a.rs"));
        assert_ne!(clean, dirty);
    }

    /// The sniff extension helper answers the empty case.
    #[test]
    fn the_sniff_extension_helper_answers_the_empty_case() {
        assert_eq!(sniff(None, 1, false).ext_str(), "");
        assert_eq!(sniff(Some("rs"), 1, false).ext_str(), "rs");
    }
}
