# AGENTS.md

Instructions for coding agents working in this repository. Read this before
touching the code; it records what is expensive to rediscover.

## What this is

A three-column desktop workspace app in Rust on GPUI Kit: file tree on the left,
document tabs in the middle, a real webview browser on the right.

The design document the app was built from has been removed; `README.md` now
carries the product overview and this file carries the internals. The old
document's risk register survived here (§ [Risk register](#risk-register))
because source comments cite those numbers — a bare `risk R11` in a comment is a
lookup, not a typo.

## Commands

```bash
cargo check --all-targets          # fast feedback
cargo build                        # the binary the driver scripts use
cargo test                         # 92 tests, ~0.3s
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

All of these must pass. `cargo test`, `fmt` and `clippy` are the definition of
done, and `-D warnings` means a warning is a failure — do not leave one for
later.

`cargo build --release` is the exception: `[profile.release]` sets fat LTO and
one codegen unit, so a full rebuild is ~9 minutes rather than ~30 s. That is the
trade for a 42 MB binary becoming ~28 MB, and it runs once per version. Do not
reach for it while iterating.

## Releasing

`bun run release <patch|minor|major|x.y.z> [--push]` (`scripts/release.ts`) is
where a release starts: it bumps `Cargo.toml`, refreshes `Cargo.lock` with
`cargo metadata`, commits `chore(release): v<version>`, and makes an annotated
`v<version>` tag — refusing a dirty tree, a tag that already exists and a version
that goes backwards, because each of those publishes something nobody can
reproduce. `--push` sends exactly `HEAD` and the new tag; `--follow-tags` was
wrong here, since it also sends every older reachable tag.

`.github/workflows/release.yml` then runs on that tag: it installs
`cargo-packager` (pinned — `[package.metadata.packager]` is a schema it owns),
runs `cargo packager --release --formats nsis`, which is where the fat-LTO build
actually happens (`before-packaging-command`), then zips the same exe as the
portable copy — the exe is the whole app — and publishes both to a release. The
job fails when the tag is not `v` + `Cargo.toml`'s version, so bumping and
tagging stay one step. `workflow_dispatch` runs the job without publishing;
`dist/` is the portable staging directory and is ignored.

## Windows resources

The exe's own icon is a Windows resource, which cargo knows nothing about:
`build/exe_icon.rs` compiles `assets/lllearning.rc` into the link, so the artwork
`assets/icon.svg` rasterises to (`assets/build-icon.ts`) is also what Explorer
and the taskbar draw. Without it the exe carries Rust's logo while the window
and the tray show the real one — the mismatch is the bug, not the styling. A
toolchain with no `rc.exe` still builds; a `.rc` that fails to compile stops the
build, because the result is that same iconless exe.

**The binary must not be running when you build.** On Windows a running
`lllearning.exe` holds the file and `cargo build` fails with
`Access is denied. (os error 5)`. Kill it first:

```powershell
Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force
```

## Layout

```
src/main.rs           bootstrap only: fonts, registries, session, open_window
src/app/              the shell and its chrome
  shell.rs            AppShell: the arrangement, document lifecycle, close flow, session
  side_view.rs        SideView: one dock — its activity strip and the pane on show
  documents_view.rs   DocumentsView: the document tabs and the document on show
  activity_bar.rs     the icon strip SideView draws
  title_bar.rs        TitleBar wrapper
  popup/mod.rs        the owned-window seam: placement, the answer channel, the registry
  popup/dialog.rs     prompts: a question with footer buttons
  popup/contextmenu.rs a list of commands at the pointer
  mod.rs              init(): the one place a feature is switched on
src/panes/            side-pane seam: explorer, browser, outline
src/documents/        document seam: text, markdown, image, refused, sniff
  status_bar_helper.rs  the items an editor's status bar says: type, caret, size
src/workspace/        folder root, lazy tree, notify watcher, fs helpers
src/state.rs          the session file, the window's bounds included
src/logging.rs        the log sink (stdout) and the startup marks
src/icons.rs          icon glyphs as text: lucide, and the nerd font for file types
src/tray.rs           tray icon; hide-on-close
src/window.rs         Win32 hide/show, the owned handle a build crosses awaits with,
                      and owning a prompt window to its caller
build/                the build script: subsets the icon fonts, one file per font
  main.rs             what it does and when; the two font calls and the icon call
  font.rs             crate lookup, subsetting, staleness
  lucide.rs           which lucide glyphs the app draws, scanned out of src/
  devicons.rs         which nerd-font glyphs the file-type table can return
  exe_icon.rs         the exe's own icon: assets/lllearning.rc compiled into the link
```

There is no dock crate in use. The window is two `SideView`s around a
`DocumentsView`, placed by the shell in one `h_resizable` group of three panels,
and the shell composes it in `render`. `src/app/dock.rs` and the whole
`gpui::base::dock` / `gpui::component::dock` surface are gone: no splits, no
dragging a tab between docks, no zoom.

Each view owns its own chrome and its own state — a `SideView` knows which pane
it shows and whether it is open; a `DocumentsView` knows its tabs and which one
is active. What the shell owns is the *arrangement* and the consequences: the
tree reveal, the `ActiveDocumentChanged` event, the debounced session write. So
the two views take callbacks for what they cannot decide (`on_select`,
`on_close`) and report changes rather than acting on them.

## The three seams

The shell knows **only** these. If a change requires editing `src/app/shell.rs`
to add a pane or a viewer, the seam is being bypassed and that is the bug.

### Adding a side pane

1. New file in `src/panes/`, e.g. `src/panes/my_pane.rs`.
2. Implement `PaneProvider` (`id`, `title`, `icon`, `slot`, optional `order`,
   `build`).
3. Implement `Focusable` and `Render` on the pane's view entity, and return
   `PaneContent::new(view)` from `build`. That is the whole contract — there is
   no dock trait to write.
4. A pane whose contents can leave the screen (the browser's native child
   window) returns `PaneContent { activation: Some(..), .. }` instead; the shell
   calls it whenever the pane is shown or hidden (risk R1b).
5. A pane that remembers something between runs supplies a `PaneController`
   beside its view (`PaneContent { controller, .. }`): the dock asks
   `view_state` for what to write into the session and calls
   `restore_view_state` once the pane is built. Both have defaults, so
   `PaneContent::new` covers a pane that keeps nothing. The value is opaque to
   the dock (`panes::ViewState`), and a state this build cannot decode is the
   pane's cue to fall back to its defaults (risk R12).
6. `pub mod my_pane;` and one `register_pane(cx, Rc::new(my_pane::MyPane));`
   line in `panes::register_builtins`.

`panes/outline.rs` is a real example; `panes/browser.rs` is the one with an
activation hook and a controller, because the page it is on is worth keeping.

### Adding a document viewer

1. New file in `src/documents/`.
2. Implement `DocumentProvider` (`id`, `title`, `score`, `open`) and a
   `DocumentController` for the view (`is_dirty`, `save`, `focus`,
   `reload_from_disk`, `focus_handle`, `is_stale`, `accept_disk_change`, and
   optionally `mark_stale`, `language`, `cursor`, `outline`, `goto_line`,
   `view_state`, `restore_view_state`).
3. One `register_document_provider(cx, Rc::new(MyProvider));` line in
   `documents::register_builtins`.

`open` returns `DocumentContent`, and its `watch` field is how a viewer tells
panes about its live buffer. A viewer whose contents change under the user must
supply one — it subscribes to the concrete view and re-broadcasts as
`DocumentEvent` on the panel. Without it the outline pane only updates when the
active *document* changes, not when a heading is typed.

`outline` is what the outline pane draws. Answer `None` when the viewer cannot
outline itself (the pane then says so); `Some(vec![])` means "this viewer
outlines, and this file has no entries". Only `markdown` answers today. The pane
knows no syntax — a new outlining viewer needs no edit to it.

An editor viewer draws a status bar along its bottom edge rather than its own
footer, because the row itself is the component library's
`component::status_bar::StatusBar` — three regions, any `IntoElement`s in them,
theme colours and border already applied. `documents::status_bar_helper` is the
seam above it: `editor_status_bar(cx, file_type, text, cursor)` returns that
component with the file type on the left and the caret, line and character
counts on the right, and a viewer chains its own actions on with
`.right(..)` — soft wrap and `markdown`'s layout switch are those today, and
both switches are built here too (`wrap_switch`, `Icon::WrapText`, tinted to
say which state it is in) because every editor viewer needs the same three
things: the glyph, the tint and the session write. The counts are derived from
the buffer it is handed, so they cannot drift from the text; but the bar does
not ask to be redrawn, so a viewer whose caret moves without a change has to
re-render itself (see `MarkdownView::new`, which observes the editor and
compares the position — observing the editor's notifications alone would redraw
the view twice a second, because its blink cycle runs off its own entity).

A viewer keeps what it wants to remember about itself — the markdown layout, both
editors' soft wrap, the caret position — by answering `view_state` and applying
it in `restore_view_state`. Each viewer defines its own state type there
(`MarkdownViewState`, `TextViewState`: structs of serde-derived fields, with the
fields a fresh tab would not have as `Option` so an older session still decodes)
and the seam's `ViewState` carries it opaquely, so the shell moves it into the
session file and back without knowing what is in it. Names inside it are written
to disk: a value this build cannot decode leaves the viewer at its defaults
rather than failing the open (risk R12). The session is written on quit *and* by
`AppShell::schedule_session_save`, and the driver scripts kill the process
rather than quitting it, so a change only the viewer knows about has to ask for
the write itself — `status_bar_helper::request_session_save` is that call, and
why `schedule_session_save` is `pub(crate)` rather than private.

`score` returns `Option<u8>`, higher wins, `None` means "cannot open". The
current table — asserted by tests, so change the tests with it:

| Provider | Score | Claims |
| --- | --- | --- |
| `image` | 90 | image extensions |
| `markdown` | 70 | `.md .markdown .mdx` |
| `text` | 10 | anything not binary and under 8 MiB |
| `refused` | 1 | what text declined, so it opens an explanation |

Keep `refused` last. Without it a binary file opens nothing at all, which reads
as a bug.

## Traps, in the order they will bite you

**`rsx!` spacing is Tailwind, not pixels — only because `rsx.toml` says so.**
The macro defaults to a *pixel* scale where `p-2` is 2px. `rsx.toml` sets
`spacing-scale = "tailwind"`, so `p-2` is 8px and `h-6` is 24px. If that file is
ever deleted, every spacing in the UI silently becomes 4× too tight. The macro
reads it at compile time, so a change needs a rebuild (touch a source file if
cargo does not notice).

**`rsx!` expands to bare `div()`, `px()` and method calls.** Every module using
it must import `div` and `px`, and the traits its attributes expand to
(`InteractiveElement as _`, `ParentElement as _`, `Styled as _`,
`prelude::FluentBuilder as _`). A missing trait surfaces as
"no method named `on_click`/`key_context`/`bg`", not as an import error.

**An element that gets `.id()` becomes `Stateful<Div>`, and `on_click` then
comes from `StatefulInteractiveElement`, not `InteractiveElement`.** Import both
if you use both.

**`#![allow(dead_code)]` at the top of a module is deliberate.** `documents`,
`workspace` and `app::shell` carry it: their seam API is exercised by tests and
by the shell, and the lint cannot see a test-only consumer. Do not "fix" it by
deleting public seam methods.

**A closed dock's panel is hidden, and its strip lives in the centre row.** The
strip is part of its dock's element (`SideView::render`), so hiding the panel
would hide the only button that reopens it. The shell therefore renders that
element inside the panel while the dock is open, and as a child of the centre
panel while it is closed — where `SideView` returns the strip alone. Hiding the
panel is the point: a panel that is merely clamped to the strip's width keeps a
divider, and a divider that cannot move anything reads as broken.

**`ResizablePanel::visible(false)` keeps the slot, so panel indices never
shift.** The centre is column 1 and the docks are 0 and 2, and the session's
saved widths are read by those indices. Never conditionally omit a panel element
from the group; toggle `visible` instead.

**A hidden panel measures zero, so the width has to be remembered outside the
group.** `ResizableState::sizes()` is the layout, and a collapsed dock's entry
is `0`; closing a dock after dragging it would forget the drag. `AppShell`
keeps the last usable measurement per slot (`widths`) and writes that to the
session, which is also what the next first frame lays the columns out at.

**`ResizablePanel::size` is the *initial* size only.** A live drag mutates
`ResizableState`, not the element. So a saved width reaches the first frame
through `AppShell::column_size` (which reads `AppShell::widths`, the last layout
it saw), never by feeding `sizes()` back into `.size()` on every render — that
would fight the drag.

**`ResizablePanelGroup::on_resize` is not what persists a width.** The
`ResizableState` entity emits `ResizablePanelEvent::Resized`, and the shell
subscribes to that in `subscribe_all`. A new group needs its own subscription.

**A dock is `fixed`; the centre is not.** A container resize rescales every
panel that is not fixed, in proportion — so maximizing the window grew a 160 px
dock to 256 px, and the next save wrote the grown number, which is what came
back on the restart. `.fixed()` keeps a dock at the width the user gave it (and
its own divider still drags it); the centre panel takes the difference.

**A pane whose contents live outside the element tree needs an activation
hook.** `PaneContent::activation` is called by `SideView::activate` on every
show/hide transition — dock toggle and pane switch. The browser's page is a
native child window that does not leave the screen with its element (risk
R1b), so the hook is what hides it. It is called only on a transition, never
per frame, so it does not fight the pane's own `Render`.

**A popup is a window, not a dialog layer or a popover.** `window.open_dialog`
draws into the main window's element tree, where the browser pane's WebView2
child window paints over it (risk R1). So does an anchored popover. Anything that
has to be seen over the page goes through `app::popup`: `popup::dialog::open`
for a question, `popup::contextmenu::open` for a menu at the pointer. Both open a
small window owned by the caller and report the choice back through a channel.

The two callbacks are the whole seam: `Popup::options` describes the window, and
`Popup::opened` runs once the window exists — where a view gets its focus handle
and its subscriptions, because neither can be built before there is a window.
`Host<T>` owns the answer channel and the owner's input, so a new popup kind
gets both by holding one.

It must be called outside a draw — opening a window paints its first frame before
it returns, so the shell defers it with `cx.defer_in` — and the returned `Task`
must be kept, or the popup has nowhere to send its answer.

**A popup is shown before it can be adopted, so a taskbar button is shed with a
hide/show.** `open_window` shows the window inside `Window::new` and only then
calls the build closure, and a window created as `WindowKind::Normal` is born with
`WS_EX_APPWINDOW` and no owner. So by the time `window::adopt_owner` runs there is
already a taskbar button for it, and Windows does not re-read a visible window's
taskbar eligibility — clearing the style alone leaves the button there forever.
`adopt_owner` therefore hides, restyles, sets the owner and shows again, and only
when the window is not already a tool window.

**A hide/show is a deactivation, and anything watching activation sees it.** The
context menu dismisses itself when its window loses the activation, so the round
trip above would close it on the frame it opened. That is why the menu asks for
`WindowKind::PopUp` — born a tool window, so no round trip happens and no
deactivation is queued. Its observer is armed from the window's current state
rather than from the first activation event, because a window created shown and
active may never report the activation it arrived with, and a menu that is never
armed never leaves.

**A popup window is owned on the platform, but nothing makes it follow its owner
off the screen.** The tray's Hide and the close-to-tray guard take the owner away
without closing it, so both call `popup::close_for` first. A modal prompt also
re-shows itself when it becomes visible again: its owner is disabled while it is
up, so a prompt left minimized behind it would lock the app. A context menu
instead takes no input from the owner, so the owner stays clickable and a click
there is what dismisses it.

**Side panes are built once and cached in `SideView`, and only while their dock
is open.** Rebuilding on every switch would recreate the WebView2 — slow, and it
re-arms the profile lock (risk R2) — and would throw away the explorer's
expansion state. A collapsed dock builds nothing at all: it only names the pane
it would show, and building one starts a browser process (risk R22).

**`WebView::build` freezes the render thread; build the page asynchronously.**
wry's synchronous path waits for WebView2 inside a *nested* Win32 message pump
(`webview2_com::wait_with_pump`), so GPUI's loop, its executor and every frame
stall for the length of that wait — measured at ~350 ms here, the bulk of risk
R19. `wry::WebViewBuilder::build_as_child_async` awaits the same two steps on
channels instead, which leaves GPUI's own loop draining: the browser pane starts
its build from a task and awaits it, so chrome, resizes and clicks keep working
while the page starts. Two things that look odd and are not:

- The task cannot hold `&Window` across an await, so it captures
  `window::ParentWindow` — the app's HWND copied into an owned
  `HasWindowHandle` shim — and `gpui_webview::WebView::new` (the second half of
  `WebView::build`) adopts the result inside `update_in`.
- The build task must be kept alive (`_build_task`). wry creates its container
  HWND before the first await, so dropping the future leaks that window; the
  builder is also given `with_visible(false)` so a leak would never paint.

**The page can arrive after the transition that decided whether it is shown.**
Startup takes hundreds of milliseconds, and the dock can be closed or switched
away in that window, so `BrowserView::wanted_visible` records what `activation`
and `Render` last asked for and `page_ready` applies it (risk R1b). For the same
reason the page is born hidden, and every navigation asked for before it exists
waits in `pending_url`: an address typed in that window loads once the page is there.

**The window's build closure runs before its first frame, so nothing expensive
belongs in it.** Until GPUI presents a frame the window is a black rectangle,
which is what the platform's window class fills it with. Building a side pane is
the expensive part — the browser's WebView2 alone is a few hundred milliseconds
— so `AppShell::new` does only the cheap part (entities, subscriptions, the
docks' *saved state* via `SideView::apply`, which builds nothing) and the
`restore_session` that builds panes is deferred (risk R19).

**`defer_in` and `on_next_frame` both run *before* a frame is presented.**
`defer_in` runs at the end of the `App::update` that the draw is inside;
`on_next_frame` callbacks run at the start of the next frame-loop iteration. So
neither is "after the frame went out": a deferred restore that builds panes still
delays the *first* present. `AppShell::new` therefore hops twice — `on_next_frame`
from `on_next_frame` — and the callback registered by the first lands before the
second present, which is after the chrome has been shown. The deferred callback
is reached through the window handle, not `Entity::update_in`: that looks the
entity's window up, and the shell is not registered with one yet, so it fails
with "entity has no current window".

**`Workspace::set_root` emits `RootChanged`, and `RootChanged` means "the user
opened a different folder".** Its handler closes every tab and resets both
docks. A restore calls `set_root` too, so `AppShell::restoring` is raised around
that call and the handler returns early for it — without the flag a restart
reopens the session's documents and then silently discards them, and the next
session write loses them for good. The flag is lowered again when the folder
could not be opened, because `set_root` reports that as `Error` and emits no
`RootChanged`; a flag left up would swallow the user's next real folder switch.

**A pane is built from inside `shell.update`, so it must not read the shell in
its constructor.** `SideView::ensure` calls `PaneProvider::build` while the
shell is leased; reading it there panics with "cannot read AppShell while it is
already being updated". Subscribe in `new` (the `WeakEntity` is safe to hand
out) and defer the first read with `cx.defer_in`. `panes/outline.rs` is the
worked example.

**A view reports; the shell decides.** `SideView::show` returns whether anything
changed and `DocumentsView::remove` returns the document that replaced the
closed one — neither writes the session, reveals a file or emits
`ActiveDocumentChanged` itself. That keeps one place that knows what a change
means, and keeps the views free of the shell's other seams.

**Editor dirty tracking.** Subscribe to `InputEvent::Change` only. Restore text
with `EditorState::set_value`, which emits no change event — using
`replace_all` instead would mark a freshly restored tab dirty.

**`Ctrl+S` is scoped to a `DocumentViewer` key context.** Both editor viewers
set it on their root element. A new editor viewer must do the same or the
binding will not reach it. The action is `documents::text::SaveDocument`.

## Things that look like bugs and are not

- **The webview paints over dialogs and dropdowns.** WebView2 cannot be clipped
  to a partial region on Windows. Dialogs render at the window root and the
  browser pane never anchors a dropdown.
- **Video files open an explanation.** GPUI has no video element, and no
  provider claims video extensions, so a video file is a binary one: the panel
  says so instead of pretending it could play.
- **A second app instance cannot start the webview** — it holds the WebView2
  profile lock, and the pane says so instead of panicking.
- **The save prompt is a separate window.** Not a bug, and not a regression from
  the old in-window dialog: it is the only way to draw over the webview.
- **A file can open as "Unavailable".** That is `MissingDocument`: the session
  named a path or provider this build cannot rebuild. It keeps the tab rather
  than dropping it.

## Risk register

The numbers are cited by source comments (`risk R8`), so this table is
append-only: add a row, never renumber. A row that no longer applies is marked
resolved rather than deleted, because deleting it turns every citation of it
into a dangling lookup.

| # | Risk | Mitigation |
| --- | --- | --- |
| R1 | **WebView2 covers GPUI overlays** and cannot be clipped to a partial region on Windows. A dialog or dropdown opened over the right dock hides behind the page. | Modal dialogs render at the window root; the browser pane never anchors a dropdown. Documented as a known limitation in `README.md`. |
| R1b | **The webview is a native child window, so it outlives the element that places it.** Collapsing the dock, switching pane or drawing the pane's error over the page leaves the page painted at its last bounds. | The pane hides and shows the native view itself: `PaneContent::activation` from the shell on every show/hide transition, and every render while an error is on show. Asserted by `scripts/drive-webview-hide.ps1`. |
| R2 | WebView2 profile lock: a second instance fails to start the webview. | The browser pane shows a "could not start" panel instead of panicking. |
| R3 | `TreeItem::is_folder()` is `!children.is_empty()`, so an empty directory is not a folder and cannot be expanded. | Give an empty or unreadable directory a disabled placeholder child; skip it when rendering and when resolving clicks (`workspace/tree.rs`). |
| R4 | **No native video element in GPUI.** | *(resolved)* The placeholder provider is gone; nothing claims video extensions, so a video file opens the binary explanation panel. A real viewer is one provider plus a score row. |
| R5 | `TreeState::set_items` clears the selection, so every lazy expand would lose the highlight. | Re-apply `set_selected_item` after each rebuild; keep expansion state in `DirTree`, not only in `TreeItem`. |
| R6 | The base tree toggles expand on **any** click, including a file click; `on_click` inside the item runs first but selection changes after. | File rows call `cx.stop_propagation()` on the label and open the document there; dirs keep the default behaviour. |
| R7 | `notify` emits duplicates and rename pairs on Windows; watching a large tree is expensive. | 250 ms debounce plus path dedupe, and per-directory refresh only for *loaded* dirs. |
| R8 | Blocking `read_dir`/file reads on the UI thread freeze the app on network drives. | All fs work goes through `cx.background_spawn`; listings are capped at `MAX_DIR_ENTRIES` (5 000). |
| R9 | Path identity on Windows: drive-letter case, `\\?\` prefixes from `canonicalize`, symlinks. | Normalise once in `fs_util.rs`; compare case-insensitively on Windows. Unit-tested there. |
| R10 | Editor dirty tracking vs. programmatic `set_value` — restoring a tab would mark it dirty. | Restore with `EditorState::set_value` (no change event) and subscribe to `InputEvent::Change` only. |
| R11 | Losing unsaved work on quit or on a folder switch. | **Open.** A dirty tab's own close button prompts, but `close_all_documents` (folder switch) and the quit hook discard silently — the hook only logs the count of dirty files, because no window is left to ask in. Closing either gap means one prompt that walks the dirty documents before the discard. |
| R12 | A persisted layout from a future or older build, or a pane id that no longer exists. | `SessionState.version` gate (now 2; a version 1 file was the dock's layout and is ignored); an unknown pane id falls back to the slot's first provider; a document that cannot be reopened survives as an `Unavailable` tab; any load error falls back to the default layout. |
| R13 | Custom title bar drag/close on Windows interacts with the tray "hide on close" guard. | Reuse `tray::keep_window_on_close`; verify the window buttons and that the tray still shows and hides the window. |
| R14 | Lucide font glyphs: baseline/centring, and a 1 MB TTF in the binary. | Render icons in a fixed-size flex box; fall back to `gpui_kit::assets::IconName` for a single icon if its metrics look off. The 1 MB cost is gone: `build/` subsets both icon fonts to the glyphs the source names. |
| R15 | Tree-sitter grammar features inflate build time and binary size. | Keep the grammar list in `Cargo.toml` minimal; the text viewer must still work when a grammar is missing. |
| R16 | *(resolved)* The dock held `Arc<dyn PanelView>`; a tab's title and close button only reached the skin through `panel_handle`. | Resolved by removing the dock: panes are plain entities and the shell draws the tabs itself. |
| R18 | **A popup window shows a taskbar button, and hiding it to remove the button deactivates it.** A window created as `Normal` is born with `WS_EX_APPWINDOW` and is already shown when the build closure runs; clearing the style on a visible window leaves the button, and the hide/show that does clear it is a deactivation, which a menu watching activation reads as a click outside. | The menu is created as `WindowKind::PopUp` (a tool window from birth, no round trip); the prompt needs the round trip and has no activation observer. See the two traps above. |
| R17 | **An in-window dialog or popover is covered by the webview.** `window.open_dialog` and an anchored popover draw into the main window's element tree, under the browser pane's native child window — the same cause as R1, which is why R1 alone did not cover it. | Dialogs and context menus are their own window (`app/popup/`), owned by the caller and reporting their answer back. Toasts and the tab bar's own `menu` still draw in-window, so R1 still governs them. |
| R19 | **A black window at startup.** The window is shown before its first frame is presented, and the platform's window class paints black until one is; anything expensive in the window's build closure — building a pane, reading a folder, starting the watcher — widens that gap. It measured ~570 ms on this machine, with the browser pane's WebView2 as the bulk of it. | `AppShell::new` does only cheap work and defers `restore_session` past the first presented frame; the folder listing and the watcher start on the background executor (R8). Gap is ~90 ms. See the two traps above for why the deferral needs two hops. |
| R20 | **A restart loses every restored tab.** `Workspace::set_root` emits `RootChanged`, whose handler closes all documents and resets the docks; the restore calls `set_root` itself, so it discarded the tabs it had just reopened — and the next session write dropped them from the file too. | `AppShell::restoring` is raised around the restore's `set_root` and the `RootChanged` handler returns early for it. Only a genuine folder switch resets the tabs. |
| R21 | **Starting the webview stalls the whole app.** `wry`'s synchronous build waits for WebView2 inside a nested Win32 message pump, so GPUI's loop, its executor and every frame freeze for the length of the wait (~350 ms here) — inside a click handler when the dock is opened, and inside the restore at startup. | The pane builds its page from a task that awaits `build_as_child_async`, keeping the task alive and adopting the result in `update_in`; a wait for the page shows a placeholder, and the raw HWND is copied into `window::ParentWindow` so the build can cross await points. `scripts/drive-webview-start.ps1` fails if a frame lands more than 150 ms after the build starts. See the trap above. |
| R22 | **A collapsed dock used to pay for its pane.** The restore called `ensure_active` for both docks, so a right dock restored closed still built the WebView2 — hundreds of milliseconds nothing on screen needed. | `SideView::ensure_active` returns early while the dock is closed, and `set_open` ensures the pane when the dock opens. The pane is built on the frame after the dock's chrome, never during it. |

## Verifying UI changes

Unit tests cannot see layout. `scripts/` drives the real window through Win32
input and writes screenshots to `%TEMP%\ll-shots`:

```powershell
pwsh -File scripts/drive-ui.ps1       # expand a folder, open a file
pwsh -File scripts/drive-edit.ps1     # open, type, Ctrl+S, report the md5
pwsh -File scripts/drive-session.ps1  # restart and check the layout came back
pwsh -File scripts/drive-close.ps1    # dirty close prompt, Save, md5
pwsh -File scripts/drive-strip.ps1    # a pane button toggles its dock
pwsh -File scripts/drive-webview-hide.ps1  # the webview leaves with its dock
pwsh -File scripts/drive-webview-start.ps1 # starting it does not stall a frame
```

Build first, then run them. They assert on file hashes where they can, so a
silent no-op save is caught. Row positions in the scripts are pixel coordinates
for the default 1216×768 window; if the layout changes, the clicks need
updating — read the screenshots rather than trusting a green run.

`drive-webview-start.ps1` is the exception to that: it reads the startup marks
below instead of clicking, and fails when `first frame presented` lands more than
150 ms after `webview build started`. A hung-window probe cannot see this,
because wry's nested pump still dispatches window messages while it blocks every
frame.

Startup timing is logged, not benchmarked: `logging` marks `shell built`,
`first draw`, `panes built` and `first frame presented` with milliseconds since
`main` began, so a run says where the black window's time went (risk R19). The
browser pane adds `webview build started` and `webview ready`, and
`first frame presented` has to land between them — that is what
`drive-webview-start.ps1` asserts. A session whose right dock is collapsed logs
neither mark, because that dock builds nothing (risk R22). Run the binary and
read stdout; `panes built` must land after `first draw`, or the deferred restore
is back in front of the frame carrying the chrome. The sink is `simple_logger`,
which also surfaces gpui's own records — its GPU and font lines, and the errors
it used to swallow in `.log_err()`.

To seed a session by hand:

```powershell
$cfg = Join-Path $env:APPDATA 'lllearning'; New-Item -ItemType Directory -Force -Path $cfg
@{ version = 2; workspace_root = "E:\Codes\lllearning" } | ConvertTo-Json |
  Set-Content (Join-Path $cfg 'session.json') -Encoding UTF8
```

Deleting `session.json` gives the fresh-start state (no folder open). That is
the intended way to test the empty states.

## Style

The existing code is the style guide. In particular:

- Comments explain **why**, not what. A comment restating the code is noise; one
  naming the trap it avoids earns its place. Risks referenced as `risk R11` map
  to the [risk register](#risk-register) above.
- Doc comments on public items say what the thing is for and what it costs, not
  just what it does.
- Prefer a `type` alias over a complex inline type that clippy flags.
- Tests are named as sentences about behaviour
  (`changing_the_root_replaces_the_tree`), not as method names.
- No `unwrap()` in library paths that can fail at runtime; `expect` with a
  reason is fine where a failure is a wiring bug.
- **Prefer `rsx!` markup wherever it can express the element.** A layout reads
  as structure in `rsx!` and as a builder chain it does not; use the chain only
  for what markup cannot say — a runtime `match` around a whole element, a
  `map` over children, an element handed to a function. `rsx!` expands to the
  same builder calls at compile time, so this costs nothing.
