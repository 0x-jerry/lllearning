//! The document seam: who can display a given file, and how to open it.
//!
//! The centre of the shell is a tab strip of documents. The *kind* of viewer is
//! chosen by file type, never by the user, so opening a file means scoring every
//! registered provider against what was sniffed from the disk and taking the
//! best.
//!
//! Adding a viewer is one module plus one [`register_document_provider`] call.
//!
//! # Why some members are `pub` but only exercised by tests
//!
//! A seam is a contract, and its contract is verified by tests rather than by
//! every consumer using every method. [`providers`], [`provider_by_id`] and the
//! `DocumentController` reload pair exist so a viewer can be driven uniformly
//! and so the score table can be asserted; the binary itself reaches only the
//! parts it needs. The dead-code lint cannot tell that difference, so it is
//! allowed on those items rather than the API being trimmed to whatever the
//! current call sites happen to use.

pub mod image;
pub mod markdown;
pub mod markdown_outline;
pub mod sniff;
pub mod text;
pub mod video;

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

/// One line of a document's outline.
///
/// `line` is 1-based and is what a jump needs; `depth` is what indentation
/// draws. The pane owns neither the text nor the parse, so both travel here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry {
    /// The label, without the syntax that produced it.
    pub label: String,
    /// 0 for a top-level entry, 1 for a nested one, and so on.
    pub depth: usize,
    /// The 1-based line the entry sits on.
    pub line: u32,
}

/// What a viewer is handed when it opens a file.
pub struct OpenContext {
    /// The path, canonicalised.
    pub path: PathBuf,
    /// What was sniffed, so a provider does not repeat the work.
    pub sniff: Sniff,
    /// The shell, for a viewer that needs to drive the rest of the app — the
    /// video placeholder's "open in browser" button is the case this exists for.
    pub shell: WeakEntity<AppShell>,
    /// The workspace, for a viewer that wants to reveal its file.
    #[allow(dead_code)]
    pub workspace: Entity<Workspace>,
}

/// Object-safe handle over a document viewer, so the shell can drive any of
/// them uniformly.
///
/// The shell holds an `AnyView` and needs to ask about dirty state and to save;
/// a trait object is what lets it do that without knowing the viewer type.
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
    ///
    /// The buffer is not touched: the panel shows a banner and the user decides
    /// whether to reload.
    fn mark_stale(&self, cx: &mut App) {
        let _ = cx;
    }

    /// The language the file is in, if the viewer has one.
    ///
    /// No chrome shows it since the footer went away, so this is seam surface a
    /// future pane can read rather than something the shell calls today.
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

    /// The outline of the document's contents, if this viewer has one.
    ///
    /// `None` is the answer for every viewer that cannot outline itself, and it
    /// is what keeps the outline pane from claiming a binary file or a video
    /// has headings. `Some(vec![])` means "this viewer outlines, and this file
    /// happens to have no entries" — the pane says so rather than the viewer
    /// declining.
    ///
    /// It lives on the controller rather than the provider because the provider
    /// is stateless: only the controller holds the live buffer, so only the
    /// controller can outline what the user has typed but not yet saved.
    fn outline(&self, cx: &App) -> Option<Vec<OutlineEntry>> {
        let _ = cx;
        None
    }

    /// Move the caret to `line` (1-based), for an outline jump.
    ///
    /// The default does nothing, so a viewer that answers [`Self::outline`] but
    /// does not implement this still shows a usable list.
    fn goto_line(&self, line: u32, window: &mut Window, cx: &mut App) {
        let _ = (line, window, cx);
    }
}

/// What a provider returns: the view and its controller.
pub struct DocumentContent {
    pub view: AnyView,
    pub controller: Rc<dyn DocumentController>,
    /// Subscribes the panel to its viewer's notifications.
    ///
    /// A viewer calls `cx.notify()` on itself when its buffer changes, and the
    /// panes watching the document — the outline, the footer — need to hear
    /// that. They cannot: [`AnyView`] carries an entity id but GPUI has no
    /// observe-by-id, so the handle has to be typed, and the only place the
    /// concrete type is known is the provider that built it.
    ///
    /// `None` for a viewer whose contents never change under the user.
    pub watch: Option<WatchView>,
}

/// A provider's way to subscribe the panel to its own view.
///
/// Called with the panel's context; the hook subscribes to the concrete view
/// and re-broadcasts its `notify` as [`DocumentEvent`] on the panel.
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
    /// meaningful: image before video before markdown before text.
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
    register_document_provider(cx, Rc::new(video::VideoProvider));
    register_document_provider(cx, Rc::new(markdown::MarkdownProvider));
    register_document_provider(cx, Rc::new(text::TextProvider));
    // Last, and scoring below text: it only claims what text refused, so a
    // binary or oversized file opens a panel that explains itself instead of
    // doing nothing at all (risk R4's sibling case).
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

/// A tab that shows one document.
///
/// The viewer inside is type-erased through [`DocumentController`], so the shell
/// can hold a list of these without knowing what each one is.
pub struct DocumentPanel {
    path: PathBuf,
    provider_id: &'static str,
    controller: Rc<dyn DocumentController>,
    view: AnyView,
    /// Dropped with the panel, so the viewer's notifications stop reaching it.
    _watch: Option<Subscription>,
}

impl EventEmitter<DocumentEvent> for DocumentPanel {}

/// The panel's own event: its contents changed.
///
/// This is what lets a pane watch "whatever the user is editing" without knowing
/// which viewer is behind it. The panel re-broadcasts its viewer's `notify`
/// because a subscriber can only subscribe to a type it can name, and the
/// viewer's type is erased by the time the shell holds it.
#[derive(Clone, Copy, Debug)]
pub struct DocumentEvent;

impl Focusable for DocumentPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.controller.focus_handle(cx)
    }
}

impl DocumentPanel {
    /// Open `path` with the best provider.
    ///
    /// Returns `None` when no provider can show the file, which the caller
    /// reports rather than leaving an empty tab behind.
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
            // Built after the panel exists: the provider's hook subscribes to
            // its own view and hands the subscription back to be held here.
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

/// The stand-in for a document that could not be reopened.
///
/// A restored tab whose file is gone, or whose viewer is missing, keeps its tab
/// and says so. Dropping it instead would read as a bug: the session named it,
/// so the user expects to see it.
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

/// Drives [`MissingDocument`] like any other viewer, so the shell holds it the
/// same way it holds a real one.
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
        registry.providers.push(Rc::new(video::VideoProvider));
        registry.providers.push(Rc::new(markdown::MarkdownProvider));
        registry.providers.push(Rc::new(text::TextProvider));
        registry.providers.push(Rc::new(refused::RefusedProvider));
        registry
    }

    /// The score table is the contract that decides what opens a file, so it is
    /// asserted directly rather than through a window.
    #[test]
    fn each_file_type_lands_on_the_viewer_it_should() {
        let registry = registry();

        let cases: [(Option<&str>, Option<&str>); 8] = [
            (Some("png"), Some("image")),
            (Some("jpg"), Some("image")),
            (Some("mp4"), Some("video")),
            (Some("webm"), Some("video")),
            (Some("md"), Some("markdown")),
            (Some("rs"), Some("text")),
            (Some("txt"), Some("text")),
            // No extension at all is still text.
            (None, Some("text")),
        ];

        for (ext, expected) in cases {
            let sniff = sniff(ext, 128, false);
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
