# Implementation plan — workspace shell for `lllearning`

Status: proposed (nothing implemented yet)
Scope: the shell, the extensibility seams, the three basic features (explorer, editor, browser),
persistence, and file watching. Based on the mockup at
`%TEMP%\pi-clipboard-48d4246b-b2d8-45f5-aafe-857643b49628.png`.

---

## 1. Goal

Turn today's single-view webview demo into a **three-column workspace app**:

| Region | Behaviour |
| --- | --- |
| Title bar | app name + icon, window buttons (mockup top strip) |
| Left | activity strip (icons) + **workspace explorer**: a lazily loaded folder tree of the current workspace folder; a button/command changes the folder |
| Center | document tab bar + **editor area**; clicking a tree file opens a tab; the *kind* of viewer is chosen by file type (text/code, markdown, image, video) |
| Right | **browser** pane: back / forward / reload + address bar + webview (today's `WebViewDemo`), plus its own activity strip |
| Footer | status bar: workspace path, active file, language, dirty state |

Success = the mockup is reproducible, the left and right panes are **extensible without touching
the shell**, the center viewer set is extensible per file type, and the session survives a restart.

---

## 2. Decisions already taken (from the design Q&A)

1. **Extensibility = compile-time Rust registry** (trait + factory), not dylibs, not JS.
2. **Center is extensible too**, dispatched **by file type** (markdown editor, image/video viewer,
   code/text editor). File type, not user choice, selects the provider.
3. **Shell layout built on `gpui_component::dock`** (`DockArea` + left/right docks + center),
   **with a custom skin** so the chrome matches the mockup.
4. **Persistence**: workspace folder, open tabs, active tab, pane sizes — restored on launch.
5. **Explorer**: lazy per-folder reads **+ `notify` file watching**.
6. **Icons**: `lucide_icons` (font glyphs), replacing ad-hoc `IconName` usage in new code.

---

## 3. Current state (what the plan builds on)

```
src/main.rs     WebViewDemo: wry webview + address bar + error/loading views, rsx! markup
src/tray.rs     tray icon, QuitMode::Explicit, keep-window-on-close
src/window.rs   Win32 hide/show for the tray
statics/        start.html, url-reporter.js (embedded with include_str!)
Cargo.toml      gpui (aliased gpui-kit 0.11, gpui-fast), gpui-webview 0.11, wry, tray-icon, png
```

Reusable as-is: `resolve_address`, `WebViewEvent` plumbing, `LoadError` view, URL reporter script,
`statics/start.html`, tray + window hiding, icon asset pipeline (`bun run build:icon`).
`WebViewDemo` becomes the right-hand **browser pane**; its `render` is split into a toolbar +
content so a panel can own it.

Relevant framework facts confirmed in the vendored crates:

* `gpui_component::dock::DockArea` renders `left dock | center column | right dock`, with
  `ResizablePanelGroup` splits inside each region.
* Panels are `Arc<dyn gpui_base::dock::PanelView>`; the presentation half is recovered through
  `gpui_component::dock::panel_handle(entity)` + `PanelHandle::of(..)`.
* Persistence is `DockArea::dump(cx) -> DockAreaState` / `DockArea::load(state, window, cx)`; the
  rebuild path goes through `register_panel(cx, name, builder)` and `PanelBuildContext`.
* Chrome is replaceable: `DockAreaRenderer` (`frame`, `split_frame`, `render_split_handle`,
  `closed_dock_extent`, `render_dock`) + `TabGroupRenderer` (`frame`, `content_frame`,
  `render_tab_bar`, …).
* Explorer: `gpui_component::tree::tree(&TreeState, render_item)` with
  `TreeEvent::{Expanded, Collapsed}` and `TreeItem { id, label, children, expanded }`.
* Editor: `component::input::{EditorState, Editor}` — `EditorState::new(window, cx)
  .default_value(text).language("rust")`, `InputEvent::Change` for dirty tracking.
* Markdown: `gpui::base::markdown(source)` (`TextView`). Images: `img(path: PathBuf)` works
  (`impl From<PathBuf> for ImageSource`).
* Folder picker: `cx.prompt_for_paths(PathPromptOptions { directories: true, files: false, .. })`
  → `oneshot::Receiver<Result<Option<Vec<PathBuf>>>>`.
* Icons: `lucide-icons` ships `Icon` (PUA chars) + `LUCIDE_FONT_BYTES`; the TTF family name is
  **`lucide`** (verified from its `name` table), installed with
  `cx.text_system().add_fonts(vec![Cow::Borrowed(LUCIDE_FONT_BYTES)])`.

---

## 4. Architecture

### 4.1 Layers

```
┌──────────────────────────── AppShell (src/app/shell.rs) ───────────────────────────┐
│  TitleBar · ActivityBar(left) · DockArea · ActivityBar(right) · StatusBar           │
│                                                                                     │
│   DockArea (gpui_component::dock)  ── with ──▶  ShellSkin (src/app/skin.rs)         │
│     ├ left dock   : PaneHost(Left)  → PaneRegistry providers                        │
│     ├ center      : PaneHost(Center)→ DocumentRegistry providers (per file type)     │
│     └ right dock  : PaneHost(Right) → PaneRegistry providers                        │
└─────────────────────────────────────────────────────────────────────────────────────┘
             │                        │                              │
      PaneRegistry              DocumentRegistry                 Workspace
      (src/panes)               (src/documents)                 (src/workspace)
```

The shell knows **nothing** about files, editors or webviews. It knows three seams:

* `PaneRegistry` — who may occupy a side slot, and how to build it.
* `DocumentRegistry` — who can display a given file.
* `Workspace` — the folder root, its tree, and its change notifications.

### 4.2 Module map (target)

```
src/
  main.rs                 bootstrap: fonts, registries, window options, open window
  app/
    mod.rs                re-exports, `init(cx)` (registers skin + builtin panes)
    shell.rs              AppShell entity: layout, activity bars, status bar, close flow
    skin.rs               ShellSkin: DockAreaRenderer + TabGroupRenderer (mockup chrome)
    activity_bar.rs       vertical icon strip widget (one per side)
    status_bar.rs         footer
    title_bar.rs          TitleBar wrapper with the app mark
  workspace/
    mod.rs                Workspace entity: root, pick/change folder, open/refresh, events
    tree.rs               DirTree: lazy children cache → Vec<TreeItem>
    watcher.rs            notify watcher, debounce, path → affected-node mapping
    fs_util.rs            read_dir sorted (dirs first), binary sniff, size caps, path normalization
  panes/
    mod.rs                PaneProvider trait, PaneSlot, PaneRegistry (Global), register helpers
    explorer.rs           left: file tree pane (tree + toolbar: change folder, refresh, reveal)
    browser.rs            right: wry webview pane (today's WebViewDemo, extracted)
    outline.rs            left: second pane stub proving the seam (optional, step 13)
  documents/
    mod.rs                DocumentProvider trait, DocumentRegistry, Document, DocumentPanel
    text.rs               text/code viewer (Editor + language by extension)
    markdown.rs           markdown editor with preview toggle
    image.rs              image viewer (img element + zoom/fit)
    video.rs              video viewer (see risk R4: no native video in gpui)
    sniff.rs              extension table + binary/size detection
  icons.rs                lucide glyph helper (`icon(Icon) -> impl IntoElement`)
  state.rs                SessionState (serde) load/save, autosave hook
  tray.rs, window.rs      unchanged
```

---

## 5. Key types (contracts to implement)

### 5.1 Pane seam — `src/panes/mod.rs`

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneSlot { Left, Right }

pub struct PaneContext<'a> {
    pub slot: PaneSlot,
    pub workspace: Entity<Workspace>,
    pub shell: WeakEntity<AppShell>,
    pub window: &'a mut Window,
    pub cx: &'a mut App,
}

pub trait PaneProvider: 'static {
    /// Stable id; written to `PanelState::panel_name`, so never rename it.
    fn id(&self) -> &'static str;
    fn title(&self) -> SharedString;
    fn icon(&self) -> lucide_icons::Icon;
    fn slot(&self) -> PaneSlot;
    fn order(&self) -> i32 { 0 }
    /// Build the panel. `restore` gets the persisted payload.
    fn build(&self, ctx: PaneContext<'_>, saved: Option<&serde_json::Value>)
        -> Arc<dyn gpui_base::dock::PanelView>;
}

pub struct PaneRegistry { providers: Vec<Rc<dyn PaneProvider>> }
impl Global for PaneRegistry {}

pub fn register_pane(cx: &mut App, provider: Rc<dyn PaneProvider>);
pub fn panes_for(cx: &App, slot: PaneSlot) -> Vec<Rc<dyn PaneProvider>>; // sorted by order
pub fn pane_by_id(cx: &App, id: &str) -> Option<Rc<dyn PaneProvider>>;
```

`register_pane` **also** calls `gpui_base::dock::register_panel(cx, provider.id(), …)` so
`DockArea::load` can rebuild it. The builder closure pulls `Entity<Workspace>` and
`WeakEntity<AppShell>` out of a `Global` (`ShellHandles`) because `PanelBuildContext` only carries
the dock area + `PanelState`.

A side slot renders as: `ActivityBar` (one button per provider for that slot) + the dock, whose
tab group holds one panel per provider that has been mounted. Clicking a strip button calls
`DockArea::select_panel(panel_id, …)` (opening the dock if closed). Adding a new side pane =
one new module + one `register_pane` line in `app::init`. **No shell edits.**

### 5.2 Document seam — `src/documents/mod.rs`

```rust
/// Everything needed to decide and to open.
pub struct Sniff { pub ext: Option<String>, pub len: u64, pub is_binary: bool }

pub trait DocumentProvider: 'static {
    fn id(&self) -> &'static str;          // "text" | "markdown" | "image" | "video"
    fn title(&self) -> &'static str;
    /// Higher wins; `None` = cannot open. Called in descending registration order.
    fn score(&self, path: &Path, sniff: &Sniff) -> Option<u8>;
    fn open(&self, path: PathBuf, ctx: OpenContext<'_>) -> DocumentContent;
}

/// Object-safe handle over an entity, so the shell can drive any viewer uniformly.
pub trait DocumentController {
    fn is_dirty(&self, cx: &App) -> bool;
    fn save(&self, cx: &mut App) -> Result<(), String>;
    fn focus(&self, window: &mut Window, cx: &mut App);
    fn reload_from_disk(&self, cx: &mut App);
}

pub struct DocumentContent {
    pub view: AnyView,
    pub controller: Rc<dyn DocumentController>,
    pub toolbar: Option<fn(&mut Window, &mut App) -> AnyElement>,
}

pub struct DocumentRegistry { providers: Vec<Rc<dyn DocumentProvider>> }
impl Global for DocumentRegistry {}

pub fn open_document(path: PathBuf, shell: WeakEntity<AppShell>, workspace: Entity<Workspace>,
                     window: &mut Window, cx: &mut App);
```

`open_document` does: stat + sniff (background) → first provider with the best score →
`Document` entity (path, provider id, content, dirty) → `DocumentPanel` (a dock panel) →
`DockArea::add_panel_view(panel_handle(panel), DockPlacement::Center, None, window, cx)` →
`select_panel`. If a tab for the same canonical path exists, select it instead of duplicating.

A blanket `impl DocumentController for Entity<T> where T: DocumentView` keeps each provider's
implementation small:

```rust
pub trait DocumentView: Render + Focusable + 'static {
    fn is_dirty(&self, cx: &App) -> bool { false }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String>;
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {}
    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>);
}
```

`DocumentPanel` implements `gpui_base::dock::Panel` + `gpui_component::dock::Panel`
(`title` = file name + `•` when dirty, `title_bar(false)` because the center shows its own tab bar
via the skin), plus `dump()` returning `PanelInfo::Panel(json!({ "path": … , "provider": … }))`.

Adding a new viewer = one module + one `register_document_provider` line. **No shell edits.**

### 5.3 Workspace — `src/workspace/mod.rs`

```rust
pub struct Workspace {
    root: Option<PathBuf>,
    tree: DirTree,                 // id -> PathBuf, id -> loaded children
    tree_state: Entity<TreeState>, // gpui-component tree state
    watcher: Option<FileWatcher>,
    truncated: HashSet<PathBuf>,
}

pub enum WorkspaceEvent {
    RootChanged,
    TreeChanged(PathBuf),          // a directory's contents changed
    DiskChanged(Vec<PathBuf>),     // files changed outside the app
    Error(String),
}
```

* `set_root(path, window, cx)` — canonicalise, build the root `TreeItem`, (re)start the watcher,
  emit `RootChanged`, reset the tree state.
* Children are read **on expand** (`TreeEvent::Expanded(id)`) on the background executor, then the
  tree is rebuilt and selection re-applied.
* `pick_root(window, cx)` — `cx.prompt_for_paths(...)`; the async reply is delivered on the
  foreground and calls `set_root`.

### 5.4 Persistence — `src/state.rs`

```rust
#[derive(Serialize, Deserialize, Default)]
pub struct SessionState {
    pub version: u32,                       // 1
    pub workspace_root: Option<PathBuf>,
    pub dock: Option<DockAreaState>,        // tabs, active tab, pane sizes, docks open/closed
    pub active_left: Option<String>,        // pane id shown in the left dock
    pub active_right: Option<String>,
    pub window: Option<SavedBounds>,
}
```

Path: `dirs::config_dir()/lllearning/session.json` (create dirs, ignore write errors with a log
line). Saved on a 500 ms debounce after `DockEvent::LayoutChanged` / `RootChanged`, and once in
`cx.on_app_quit(...)`. Loaded before the first window is opened.

Because `DockAreaState` carries each panel's `PanelState` payload, open tabs, the active tab and
pane sizes all persist **through the dock**; we only add `workspace_root` and the pane selections.

---

## 6. Ordered steps

Each step compiles and is verifiable on its own. Steps 1–4 are pure plumbing and can land first.

### Step 0 — Dependencies and bootstrap (no UI change)

* `Cargo.toml`: add
  `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`,
  `notify = "8"`, `dirs = "6"`, `lucide-icons = "1.80"`,
  `ignore = "0.4"` (optional, step 8), and gpui features
  `tree-sitter`, `tree-sitter-markdown`, `tree-sitter-json`, `tree-sitter-rust`,
  `tree-sitter-javascript`, `tree-sitter-typescript`, `tree-sitter-toml`, `tree-sitter-yaml`,
  `tree-sitter-bash`, `tree-sitter-python`.
* `src/icons.rs`: `pub fn icon(i: lucide_icons::Icon) -> impl IntoElement` rendering
  `div().font_family("lucide").text_size(px(16.)).child(char::from(i).to_string())`, plus
  `pub fn install_font(cx: &mut App)` calling `cx.text_system().add_fonts(...)`.
* `main.rs`: call `icons::install_font(cx)` right after `gpui::init(cx)`.

**Verify:** `cargo check`; a temporary `icon(Icon::Folder)` in the current view renders a folder
glyph (not tofu).

### Step 1 — Skeleton modules

Create the empty modules from §4.2 with `mod` wiring and no behaviour, so later steps have homes.
`app::init(cx)` exists and is called from `main`.

**Verify:** `cargo check`, `cargo clippy -- -D warnings`.

### Step 2 — `Workspace` with a static root and a flat tree

`workspace/tree.rs` + `fs_util.rs`: `read_dir` sorted (dirs first, case-insensitive), skipping
hidden entries and `target`/`node_modules`/`.git`; build `Vec<TreeItem>` where the id is the
absolute path string. Empty directories get a disabled `__empty__` child so `TreeItem::is_folder()`
stays true (see R3).

**Verify:** unit test `read_dir_orders_dirs_first_and_skips_ignored`; unit test
`empty_dir_is_still_a_folder`.

### Step 3 — Explorer pane (`panes/explorer.rs`)

Implements `PaneProvider` (`id: "explorer"`, `slot: Left`, `icon: Icon::Files`) and a
`gpui_base::dock::Panel` wrapper. Content: a toolbar row (`Change folder…`, `Refresh`, root name)
+ `component::tree(&tree_state, …)`.

* Subscribe to `TreeState` `TreeEvent::Expanded` → `Workspace::load_children(dir)`.
* Row render: chevron (dirs), lucide icon by kind, label; depth indent.
* Click on a **file** row → `documents::open_document(path, …)`. Click on a dir row toggles expand
  (base already does that; we only need the file branch — use `on_click` on the label with
  `cx.stop_propagation()` for files, see R6).
* Selection: keep the active document's path highlighted by `TreeState::set_selected_item`.

**Verify:** register the pane, mount it into a left dock with `set_dock`; expanding loads children
on demand; clicking a `.txt` opens a tab (temporary text panel until step 6).

### Step 4 — Shell layout + skin (no real panes yet)

* `app/skin.rs`: `ShellSkin` implementing `DockAreaRenderer` (flat `frame`, hairline
  `render_split_handle` reusing `component::resizable::resize_handle_appearance`, dock chrome with
  a subtle background and a top border) and `TabGroupRenderer` (`render_tab_bar` → `TabBar` with
  `variant=underline`, close button, no zoom control, overflow `menu(true)`).
  Mirror `DockSkin::dock_area(...)`'s construction dance to keep a `WeakEntity<DockArea>`.
* `app/shell.rs`: `AppShell` renders `v_flex` → `TitleBar` → `h_flex`(activity bar, dock area,
  activity bar) → `StatusBar`. Holds `Entity<DockArea>`, `Entity<Workspace>`, and
  `HashMap<PaneSlot, HashMap<String, PanelId>>`.
* `app/activity_bar.rs`: column of ghost buttons; active pane highlighted; click →
  `select_panel` / `toggle_dock`.
* `main.rs`: window options from `TitleBar::window_options()` (transparent titlebar,
  `app_owns_titlebar_drag: true`), size 1200×760, `open_window(..., |w, cx| AppShell::new(...))`.
* Default layout when no session: `DockLayout::h_split()` with left `tabs().panel_view(explorer)`
  and center `tabs()` empty; `set_dock(Left, …, Some(px(240.)))`.

**Verify:** window shows title bar, three regions, draggable dividers, footer; left dock collapses
and reopens from the activity strip.

### Step 5 — Browser pane (`panes/browser.rs`)

Move `WebViewDemo` into a `BrowserPane` panel: state = `Option<Entity<WebView>>`, `address`,
`loading`, `address_edited`, `error`, `start_page`; logic (`resolve_address`, `navigate`,
`go_back/forward`, `reload`, `stop`, event channel) unchanged. `title_bar(false)`,
`inner_padding(false)`, `dump()` → `{ "url": <current> }`.

**Verify:** browser pane loads the start page, navigation/back/reload work, devtools still open in
debug builds; the pane survives a dock resize without losing the page.

### Step 6 — Document providers (center)

* `documents/sniff.rs`: extension → provider hint table; `is_binary` = NUL in first 8 KiB; refuse
  files > 8 MiB (configurable const) with a friendly "too large" panel.
* `documents/text.rs`: `EditorState::new(window, cx).default_value(text).language(lang_for(ext))`;
  `Editor::new(&state)`; `InputEvent::Change` → dirty; `Ctrl/Cmd+S` (`KeyBinding` in the pane's key
  context) → `DocumentView::save` (write on the background executor, then clear dirty and refresh
  the tab title).
* `documents/markdown.rs`: editor on the left, `gpui::base::markdown(&text)` preview on the right,
  toggle in the panel toolbar; save as above.
* `documents/image.rs`: `img(path)` with `object_fit(Contain)`, checkerboard background,
  fit/1:1 zoom buttons, dimensions in the status bar.
* `documents/video.rs`: see R4 — ship a placeholder panel that says video playback needs the
  webview; wire the `score` for `.mp4/.webm/.mov` so the extension point is exercised.
* Register in order: image(90) / video(80) / markdown(70) / text(10, always matches non-binary).

**Verify:** open `.md`, `.png`, `.rs`, `.txt`, a binary file, and a >8 MiB file from the tree;
each opens the expected viewer; edit + save round-trips to disk (`md5sum` before/after).

### Step 7 — Dirty-close flow

Skin's close button must not close directly: route through `AppShell::request_close(panel_id)`,
which for a dirty document opens a `component::dialog::Dialog` with **Save / Discard / Cancel**;
Save runs `controller.save` then closes, Discard closes, Cancel aborts. Clean tabs close
immediately via `DockArea::remove_panel`/`TabGroup::close_panel`. Also intercept app quit
(`cx.on_app_quit`) when any document is dirty (single combined prompt).

**Verify:** edit → close → Save writes and the tab disappears; Discard loses changes; Cancel keeps
the tab and the dirty marker.

### Step 8 — File watching

* `workspace/watcher.rs`: `notify::recommended_watcher` over the root (recursive), events pushed
  into a `smol` channel consumed by a foreground task with a **250 ms debounce** and path dedupe.
* Mapping: a change to `dir/*` → if `dir` is loaded, re-read its children; a change to a path with
  an open document → emit `DiskChanged`; the `DocumentPanel` shows a "changed on disk — Reload"
  banner (never silently overwrite a dirty buffer).
* Ignore events under ignored dirs (same predicate as `read_dir`); drop the watcher when the root
  changes (`notify::Watcher::unwatch`/drop).
* Optional: filter listings with `ignore::gitignore` built at the root (respects `.gitignore`).

**Verify:** create/rename/delete files in Explorer from the shell → tree updates within ~0.5 s;
editing a file in another editor flags the open tab; deleting the workspace root shows an error
state, not a panic.

### Step 9 — Change workspace folder

Explorer toolbar `Change folder…` → `cx.prompt_for_paths`; on reply: `Workspace::set_root`, restart
the watcher, **close all document tabs** (or ask to save dirty ones first), reset the tree, persist.

**Verify:** switching folders replaces the tree, clears tabs (after the save prompt), and the new
root is persisted across a restart.

### Step 10 — Session persistence

`src/state.rs` as in §5.4. Save on debounced `DockEvent::LayoutChanged`, `WorkspaceEvent::RootChanged`,
and quit. Load at startup **before** the window opens; restore order:

1. `Workspace` with the saved root (skip the watcher until the window exists).
2. Create `DockArea` with `ShellSkin`.
3. `DockArea::load(saved.dock)` — registry builders recreate explorer/browser/editor panels.
4. Fall back to the default layout if the file is missing, the version differs, or the root no
   longer exists.

**Verify:** open 3 files, move a divider, switch the left pane, restart → same tabs, active tab,
pane widths, folder. Corrupt the JSON by hand → app starts with the default layout and logs, no
panic.

### Step 11 — Second pane to prove the seam (acceptance test for extensibility)

Add `panes/outline.rs` (`id: "outline"`, `slot: Left`, `icon: Icon::ListTree`, lists headings from
the active markdown document or symbols by regex for code). It must require **only** a new module +
one `register_pane` line + nothing in `app/shell.rs` or `app/skin.rs`.

**Verify:** the left strip gains a third button; switching to it shows the outline; removing the
`register_pane` line removes the button. `git diff --stat` for this step touches only `panes/outline.rs`,
`panes/mod.rs` (one line) — proof the seam is real.

### Step 12 — Status bar + polish

`app/status_bar.rs`: workspace path, active document path + language, `Ln/Col` (from
`EditorState` selection, if cheap to observe), dirty count, watcher state. Footer styling per
mockup (small, muted, 1px top border). Empty center state (no tabs) shows a hint panel
(`component::empty::Empty`).

**Verify:** footer reflects the active tab; empty state appears when the last tab closes.

### Step 13 — Docs and cleanup

Update `README.md` (architecture, how to add a pane, how to add a viewer, persistence file
location, new dependencies, Linux/macOS notes). Delete dead code left in `main.rs`. Keep
`statics/` as-is.

**Verify:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
`cargo run` manual pass.

---

## 7. Risks and tricky parts

| # | Risk | Mitigation |
| --- | --- | --- |
| R1 | **WebView2 covers GPUI overlays** and cannot be clipped to a partial region on Windows (documented in `gpui-webview`). Dialogs/popovers opened over the right dock will be hidden behind the page. | Keep modal dialogs (dirty-close, folder picker is native) outside the webview's bounds by rendering them at the window root (`Root`/`Dialog`), and never anchor a dropdown inside the browser pane. Document the limitation. |
| R2 | WebView2 profile lock: a second instance fails to start the webview (already handled today). | Keep the existing "web view could not start" panel inside `BrowserPane`. |
| R3 | `TreeItem::is_folder()` is `!children.is_empty()`, so an empty directory is not a folder and cannot be expanded/clicked. | Give empty dirs a disabled placeholder child; skip it when rendering and when resolving clicks. |
| R4 | **No native video element in GPUI.** | Ship the `video` provider as a placeholder (message + "open in browser" button that navigates the right pane), and/or render video through a `WebView` pointed at `file://` — decide when implementing step 6; the seam is unaffected. |
| R5 | `TreeState::set_items` clears the selection, so every lazy expand would lose the highlight. | After each rebuild, re-apply `set_selected_item` from the active document path; keep expansion state in `DirTree` (ids), not only in `TreeItem`. |
| R6 | Base tree toggles expand on **any** click, including a file click; `on_click` inside the item runs first but selection changes after. | Handle file clicks with `cx.stop_propagation()` on the label element and open the document there; dirs keep the default behaviour. |
| R7 | `notify` emits duplicates and rename pairs on Windows; watching a large tree is expensive. | 250 ms debounce + path dedupe + per-directory refresh only for *loaded* dirs; watch recursively but ignore the same paths `read_dir` skips. |
| R8 | Blocking `read_dir`/file reads on the UI thread freeze the app on network drives. | All fs work through `cx.background_spawn`; UI shows a per-node spinner; cap directory listings (e.g. 5 000 entries, mark "truncated"). |
| R9 | Path identity on Windows (drive-letter case, `\\?\` prefixes from `canonicalize`, symlinks). | Normalise once: `canonicalize` for the root, `fs::canonicalize` only when comparing; compare case-insensitively on Windows. One helper in `fs_util.rs`, unit-tested. |
| R10 | Editor dirty tracking vs. programmatic `set_value` (restoring a tab would mark it dirty). | Use `EditorState::set_value` (which suppresses events) for restore; subscribe only to `InputEvent::Change` afterwards. |
| R11 | Losing unsaved work on quit / workspace switch. | Single quit hook that walks dirty documents; workspace switch reuses the same prompt. |
| R12 | Persisted layout from a future/older build, or a panel id that no longer exists. | `SessionState.version` + `DockArea::new(id, Some(1), …)`; unknown panels survive as base placeholders; any load error falls back to the default layout. |
| R13 | Custom title bar drag/close on Windows interacts with the tray "hide on close" guard. | Reuse `tray::keep_window_on_close`; verify minimise/maximise/close buttons and that the tray still shows/hides the window. |
| R14 | Lucide font glyphs: baseline/centring, and a 1 MB TTF in the binary. | Render icons in a fixed-size box with `flex items-center justify-center`; if metrics look off, fall back to `gpui_kit::assets::IconName` for that one icon. |
| R15 | Tree-sitter grammar features inflate build time and binary size. | Start with the language list in step 0; drop grammars that are not needed. `text` provider must work with `language("text")` when a grammar is missing. |
| R16 | `DockArea` centers/side docks are `Arc<dyn PanelView>`; presentation (tab title, close button) only reaches the skin through `panel_handle`. | Always wrap panels with `panel_handle(...)`; never hand a bare `Entity<P>` to `add_panel`/`DockLayout::panel`, or tabs lose their titles. |

---

## 8. Verification checklist (definition of done)

Build & static:

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test` — includes new unit tests: `read_dir` ordering/filtering, empty-dir folder flag,
      path normalisation, sniff/score table (`png→image`, `md→markdown`, `rs→text`, binary→none),
      `SessionState` round-trip, `resolve_address` (existing behaviour preserved).

Manual, on Windows (WebView2) and one Linux run if available:

- [ ] App starts, custom title bar renders, minimise/maximise/close work, close hides to tray,
      tray Show/Hide/Quit work.
- [ ] Left strip: explorer button; tree shows the workspace root; expanding a folder loads children
      with no visible freeze; collapsing/re-expanding is instant (cached).
- [ ] Click a text file → tab opens, content editable, `Ctrl+S` writes, tab shows dirty marker
      before save and clears after.
- [ ] Click a `.md` → markdown editor + preview toggle; `.png` → image with fit/zoom;
      binary/oversized file → explanatory panel, no crash.
- [ ] Right strip: browser pane; start page loads; address bar navigation, back, forward, reload,
      stop all behave as before the refactor; devtools open in debug.
- [ ] Dividers resize left/center/right; widths clamp sanely; left dock collapses and reopens.
- [ ] Dirty tab close → Save/Discard/Cancel all behave; quit with a dirty tab prompts once.
- [ ] External change: `echo x >> file.txt` flags the open tab; `touch new.txt` in the workspace
      shows up in the tree within ~0.5 s; deleting a file removes its row.
- [ ] "Change folder…" switches the tree and the tab set; the new folder survives a restart.
- [ ] Restart: same folder, same tabs, same active tab, same pane widths, same pane selection.
- [ ] Corrupt/delete `session.json` → clean default start, one log line, no panic.
- [ ] Extensibility proof: `git diff --stat` for step 11 shows no changes under `src/app/`.

---

## 9. Explicitly out of scope for this pass

Search/replace across the workspace, git status decorations, LSP, tabs split within the center,
drag-and-drop of files into the tree, dylib/JS plugins, multi-window, settings UI.
The seams above are the hooks a later pass would use (a `search` pane is another
`PaneProvider`; a preview pane is another `DocumentProvider`).
