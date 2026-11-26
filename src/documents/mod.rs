//! The document seam: who can display a given file, and how to open it.
//!
//! The center of the shell is a dock tab group whose panels are documents. The
//! *kind* of viewer is chosen by file type, never by the user (plan §2.2), so
//! opening a file means scoring every registered provider against what was
//! sniffed from the disk and taking the best.
//!
//! Adding a viewer is one module plus one [`register_document_provider`] call.

pub mod image;
pub mod markdown;
pub mod sniff;
pub mod text;
pub mod video;

use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    AnyView, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window,
    base::dock::PanelEvent, component::dock::BasePanelView, div,
};
use serde_json::Value;

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
    pub fn ext_str(&self) -> &str {
        self.ext.as_deref().unwrap_or("")
    }
}

/// What a provider is handed when it opens a file.
pub struct OpenContext {
    /// The path, canonicalised.
    pub path: PathBuf,
    /// What was sniffed, so a provider does not repeat the work.
    pub sniff: Sniff,
    /// The shell, for a viewer that needs to drive the rest of the app — the
    /// video placeholder's "open in browser" button is the case this exists for.
    pub shell: WeakEntity<AppShell>,
    /// The workspace, for a viewer that wants to reveal its file.
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

    /// The focus handle the dock focuses when this tab is selected.
    fn focus_handle(&self, cx: &App) -> FocusHandle;

    /// Whether the file changed on disk under an unmodified buffer.
    fn is_stale(&self, cx: &App) -> bool;

    /// Accept the on-disk version, clearing the stale marker.
    fn accept_disk_change(&self, window: &mut Window, cx: &mut App);

    /// The language shown in the status bar, if the viewer has one.
    fn language(&self, cx: &App) -> Option<SharedString> {
        let _ = cx;
        None
    }

    /// The caret's 1-based line and column, for the status bar.
    fn cursor(&self, cx: &App) -> Option<(u32, u32)> {
        let _ = cx;
        None
    }
}

/// What a provider returns: the view and its controller.
pub struct DocumentContent {
    pub view: AnyView,
    pub controller: Rc<dyn DocumentController>,
}

/// A kind of document viewer.
pub trait DocumentProvider: 'static {
    /// Stable id, written into the persisted layout. Never rename it.
    fn id(&self) -> &'static str;

    /// The name shown in menus and the status bar.
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

/// A provider by id.
pub fn provider_by_id(cx: &App, id: &str) -> Option<Rc<dyn DocumentProvider>> {
    cx.try_global::<DocumentRegistry>()
        .and_then(|registry| registry.by_id(id))
}

/// Register the viewers this build ships.
///
/// Order matters only for ties, and the scores are chosen so there are none.
pub fn register_builtins(cx: &mut App) {
    register_document_provider(cx, Rc::new(image::ImageProvider));
    register_document_provider(cx, Rc::new(video::VideoProvider));
    register_document_provider(cx, Rc::new(markdown::MarkdownProvider));
    register_document_provider(cx, Rc::new(text::TextProvider));
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

/// The persisted payload for a document panel.
///
/// Both halves are needed: the path says which file, the provider says which
/// viewer, and a restart rebuilds the panel through the registered provider.
pub fn document_payload(path: &Path, provider_id: &str) -> Value {
    serde_json::json!({ "path": path.to_string_lossy(), "provider": provider_id })
}

/// Read a document payload back into its path and provider id.
pub fn parse_document_payload(value: &Value) -> Option<(PathBuf, String)> {
    let path = value.get("path")?.as_str()?;
    let provider = value.get("provider")?.as_str()?;
    Some((PathBuf::from(path), provider.to_owned()))
}

/// A dock panel that shows one document.
///
/// The viewer inside is type-erased through [`DocumentController`], so the shell
/// can hold a list of these without knowing what each one is.
pub struct DocumentPanel {
    path: PathBuf,
    provider_id: &'static str,
    controller: Rc<dyn DocumentController>,
    view: AnyView,
}

impl EventEmitter<PanelEvent> for DocumentPanel {}

impl Focusable for DocumentPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.controller.focus_handle(cx)
    }
}

impl DocumentPanel {
    /// Open `path` with the best provider, and wrap it as a dock panel.
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
                eprintln!("could not read {}: {error}", path.display());
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

        Some(cx.new(|_| Self {
            path,
            provider_id,
            controller: content.controller,
            view: content.view,
        }))
    }

    /// The file this document shows.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The provider id.
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
        div().size_full().child(self.view.clone())
    }
}

impl gpui::base::dock::Panel for DocumentPanel {
    fn panel_name(&self) -> &'static str {
        "document"
    }

    fn dump(&self, _cx: &App) -> gpui::component::dock::PanelState {
        gpui::component::dock::PanelState {
            panel_name: "document".to_owned(),
            children: Vec::new(),
            info: gpui::component::dock::PanelInfo::panel(document_payload(
                &self.path,
                self.provider_id,
            )),
        }
    }
}

impl gpui::component::dock::Panel for DocumentPanel {
    fn tab_name(&self, cx: &App) -> Option<SharedString> {
        Some(self.tab_title(cx))
    }

    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tab_title(cx)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }

    fn title_bar(&self, _cx: &App) -> bool {
        false
    }
}

/// Rebuild a document panel from its persisted payload.
///
/// This is what makes a restart restore open tabs: the dock's registry calls it
/// through the builder [`register_documents`] installs.
pub fn restore_document(
    payload: &Value,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<dyn BasePanelView>> {
    let (path, _provider_id) = parse_document_payload(payload)?;
    let handles = crate::panes::ShellHandles::get(cx);
    let panel = DocumentPanel::open(path, handles.shell, handles.workspace, window, cx)?;
    Some(gpui::component::dock::panel_handle(panel))
}

/// Register the dock's builder for document panels.
pub fn register_documents(cx: &mut App) {
    gpui::component::dock::register_panel(cx, "document", |context, window, cx| {
        let payload = match context.info() {
            gpui::component::dock::PanelInfo::Panel(value) => value.clone(),
            _ => Value::Null,
        };
        restore_document(&payload, window, cx).unwrap_or_else(|| {
            // An unknown payload means the file or provider is gone. A
            // placeholder keeps the layout intact instead of dropping a tab.
            gpui::component::dock::panel_handle(cx.new(|cx| MissingDocument::new(payload, cx)))
        })
    });
}

/// The stand-in for a document panel that could not be rebuilt.
pub struct MissingDocument {
    payload: Value,
    focus_handle: FocusHandle,
}

impl MissingDocument {
    fn new(payload: Value, cx: &mut Context<Self>) -> Self {
        Self {
            payload,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PanelEvent> for MissingDocument {}

impl Focusable for MissingDocument {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MissingDocument {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let name = parse_document_payload(&self.payload)
            .map(|(path, _)| fs_util::file_name_of(&path))
            .unwrap_or_else(|| "an unknown file".to_owned());
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(SharedString::from(format!(
                "{name} could not be reopened; its viewer is not available."
            )))
    }
}

impl gpui::base::dock::Panel for MissingDocument {
    fn panel_name(&self) -> &'static str {
        "document"
    }

    fn dump(&self, _cx: &App) -> gpui::component::dock::PanelState {
        gpui::component::dock::PanelState {
            panel_name: "document".to_owned(),
            children: Vec::new(),
            info: gpui::component::dock::PanelInfo::panel(self.payload.clone()),
        }
    }
}

impl gpui::component::dock::Panel for MissingDocument {
    fn tab_name(&self, _cx: &App) -> Option<SharedString> {
        Some(SharedString::from("Unavailable"))
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Unavailable")
    }
}
