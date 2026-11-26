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
cargo test                         # 68 tests, ~0.3s
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

All of these must pass. `cargo test`, `fmt` and `clippy` are the definition of
done, and `-D warnings` means a warning is a failure — do not leave one for
later.

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
  shell.rs            AppShell: layout, documents, close flow, session
  dock.rs             ShellSkin: the dock's appearance (written from scratch)
  activity_bar.rs     the icon strip beside each dock
  title_bar.rs        TitleBar wrapper
  status_bar.rs       the footer
  mod.rs              init(): the one place a feature is switched on
src/panes/            side-pane seam: explorer, browser, outline
src/documents/        document seam: text, markdown, image, video, refused, sniff
src/workspace/        folder root, lazy tree, notify watcher, fs helpers
src/state.rs          the session file
src/icons.rs          icon glyphs as text: lucide, and the nerd font for file types
src/tray.rs, window.rs  tray icon and Win32 hide/show (unchanged from the demo)
```

## The three seams

The shell knows **only** these. If a change requires editing `src/app/shell.rs`
to add a pane or a viewer, the seam is being bypassed and that is the bug.

### Adding a side pane

1. New file in `src/panes/`, e.g. `src/panes/my_pane.rs`.
2. Implement `PaneProvider` (`id`, `title`, `icon`, `slot`, optional `order`,
   `build`).
3. Implement these five on the pane's view entity:

   ```rust
   impl Focusable for MyView { /* focus_handle */ }
   impl EventEmitter<gpui::base::dock::PanelEvent> for MyView {}
   impl gpui::base::dock::Panel for MyView {          // panel_name, dump
   impl gpui::component::dock::Panel for MyView {     // tab_name, title,
                                                      // inner_padding, title_bar
   impl Render for MyView {}
   ```

4. `pub mod my_pane;` and one `register_pane(cx, Rc::new(my_pane::MyPane));`
   line in `panes::register_builtins`.

That is the whole cost. It is measured: a throwaway pane took one new file plus
two lines, and `git status -- src/app` was empty. `panes/outline.rs` is a real
example.

### Adding a document viewer

1. New file in `src/documents/`.
2. Implement `DocumentProvider` (`id`, `title`, `score`, `open`) and a
   `DocumentController` for the view (`is_dirty`, `save`, `focus`,
   `reload_from_disk`, `focus_handle`, `is_stale`, `accept_disk_change`, and
   optionally `mark_stale`, `language`, `cursor`, `outline`, `goto_line`).
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

`score` returns `Option<u8>`, higher wins, `None` means "cannot open". The
current table — asserted by tests, so change the tests with it:

| Provider | Score | Claims |
| --- | --- | --- |
| `image` | 90 | image extensions |
| `video` | 80 | video extensions (placeholder) |
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

**A panel must be wrapped in `panel_handle(...)`.** Passing a bare
`Entity<P>` to the dock stores the entity, `PanelHandle::of` returns `None`, and
the tab silently loses its title and close button. `PaneProvider::build` returns
`Arc<dyn BasePanelView>` and must wrap its own view
(`panel_handle(view)`); `documents::register_documents` wraps for you.

**`DockSkin` cannot be extended.** Its shared state is crate-private and a
second `impl DockAreaRenderer for DockSkin` is barred by the orphan rule, so
`src/app/dock.rs` is a from-scratch renderer. It copies the construction dance
from `DockSkin::dock_area` to get a `WeakEntity<DockArea>` — that weak handle is
the only way a tab bar can read dock open/closed state.

**A lone panel is never "draggable", so `is_panel_closable` is false for it.**
That is why the close button is gated on `handle.closable(cx)` instead, and why
a document asks for a `title_bar` even when it is the only tab — the tab is
where its close button and dirty marker live.

**`Panel::panel_name` returns `&'static str`.** A persisted name that must round
trip has to be interned; see `interned_name` in `dock.rs`.

**A pane is built from inside `shell.update`, so it must not read the shell in
its constructor.** `mount_side_panes` and `show_pane` both call
`PaneProvider::build` while the shell is leased; reading it there panics with
"cannot read AppShell while it is already being updated". Subscribe in `new`
(the `WeakEntity` is safe to hand out) and defer the first read with
`cx.defer_in`. `panes/outline.rs` is the worked example.

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
- **Video files open a placeholder.** GPUI has no video element. The provider
  explains that and offers the browser pane.
- **A second app instance cannot start the webview** — it holds the WebView2
  profile lock, and the pane says so instead of panicking.
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
| R1b | **The webview is a native child window, so it outlives the element that places it.** Collapsing the dock, switching tab or drawing the pane's error over the page leaves the page painted at its last bounds. | The pane hides and shows the native view itself: `set_active`/`on_removed` from the dock, and every render while an error is on show. Asserted by `scripts/drive-webview-hide.ps1`. |
| R2 | WebView2 profile lock: a second instance fails to start the webview. | The browser pane shows a "could not start" panel instead of panicking. |
| R3 | `TreeItem::is_folder()` is `!children.is_empty()`, so an empty directory is not a folder and cannot be expanded. | Give an empty or unreadable directory a disabled placeholder child; skip it when rendering and when resolving clicks (`workspace/tree.rs`). |
| R4 | **No native video element in GPUI.** | The `video` provider is a placeholder that explains why and offers the browser pane. The seam is unaffected — a real viewer is one provider. |
| R5 | `TreeState::set_items` clears the selection, so every lazy expand would lose the highlight. | Re-apply `set_selected_item` after each rebuild; keep expansion state in `DirTree`, not only in `TreeItem`. |
| R6 | The base tree toggles expand on **any** click, including a file click; `on_click` inside the item runs first but selection changes after. | File rows call `cx.stop_propagation()` on the label and open the document there; dirs keep the default behaviour. |
| R7 | `notify` emits duplicates and rename pairs on Windows; watching a large tree is expensive. | 250 ms debounce plus path dedupe, and per-directory refresh only for *loaded* dirs. |
| R8 | Blocking `read_dir`/file reads on the UI thread freeze the app on network drives. | All fs work goes through `cx.background_spawn`; listings are capped at `MAX_DIR_ENTRIES` (5 000). |
| R9 | Path identity on Windows: drive-letter case, `\\?\` prefixes from `canonicalize`, symlinks. | Normalise once in `fs_util.rs`; compare case-insensitively on Windows. Unit-tested there. |
| R10 | Editor dirty tracking vs. programmatic `set_value` — restoring a tab would mark it dirty. | Restore with `EditorState::set_value` (no change event) and subscribe to `InputEvent::Change` only. |
| R11 | Losing unsaved work on quit or on a folder switch. | **Open.** A dirty tab's own close button prompts, but `close_all_documents` (folder switch) and the quit hook discard silently — the hook only counts the dirty files on stderr, because no window is left to ask in. Closing either gap means one prompt that walks the dirty documents before the discard. |
| R12 | A persisted layout from a future or older build, or a panel id that no longer exists. | `SessionState.version` gate; an unknown panel survives as a placeholder; any load error falls back to the default layout. |
| R13 | Custom title bar drag/close on Windows interacts with the tray "hide on close" guard. | Reuse `tray::keep_window_on_close`; verify the window buttons and that the tray still shows and hides the window. |
| R14 | Lucide font glyphs: baseline/centring, and a 1 MB TTF in the binary. | Render icons in a fixed-size flex box; fall back to `gpui_kit::assets::IconName` for a single icon if its metrics look off. |
| R15 | Tree-sitter grammar features inflate build time and binary size. | Keep the grammar list in `Cargo.toml` minimal; the text viewer must still work when a grammar is missing. |
| R16 | The dock holds `Arc<dyn PanelView>`; a tab's title and close button only reach the skin through `panel_handle`. | Always wrap panels with `panel_handle(...)`; never hand a bare `Entity<P>` to the dock. |

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
```

Build first, then run them. They assert on file hashes where they can, so a
silent no-op save is caught. Row positions in the scripts are pixel coordinates
for the default 1216×768 window; if the layout changes, the clicks need
updating — read the screenshots rather than trusting a green run.

To seed a session by hand:

```powershell
$cfg = Join-Path $env:APPDATA 'lllearning'; New-Item -ItemType Directory -Force -Path $cfg
@{ version = 1; workspace_root = "E:\Codes\lllearning" } | ConvertTo-Json |
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

## Do not

- Do not add a dependency without a reason in a comment.
- Do not renumber or rename a `PaneProvider::id` or a panel name — it is written
  into the session file, and renaming turns every saved layout into an
  "Unavailable" tab.
- Do not edit `statics/` (the start page and URL reporter); they are embedded
  with `include_str!` and are not part of the shell.
- Do not commit `target/`, `node_modules/`, or a `session.json`.
