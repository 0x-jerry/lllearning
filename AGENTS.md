# AGENTS.md

Instructions for coding agents working in this repository. Read this before
touching the code; it records what is expensive to rediscover.

## What this is

A three-column desktop workspace app in Rust on GPUI Kit: file tree on the left,
document tabs in the middle, a real webview browser on the right.

`docs/plan.md` is the design document the app was built from. It is accurate and
worth reading for the *why*; this file covers the *how* and the traps.

## Commands

```bash
cargo check --all-targets          # fast feedback
cargo build                        # the binary the driver scripts use
cargo test                         # 63 tests, ~0.3s
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

All four must pass. `cargo test` and `clippy` are the definition of done, and
`-D warnings` means a warning is a failure — do not leave one for later.

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
  skin.rs             ShellSkin: the dock's appearance (written from scratch)
  activity_bar.rs     the icon strip beside each dock
  title_bar.rs        TitleBar wrapper
  status_bar.rs       the footer
  mod.rs              init(): the one place a feature is switched on
src/panes/            side-pane seam: explorer, browser, outline
src/documents/        document seam: text, markdown, image, video, refused, sniff
src/workspace/        folder root, lazy tree, notify watcher, fs helpers
src/state.rs          the session file
src/icons.rs          lucide glyphs as text
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
   optionally `mark_stale`, `language`, `cursor`).
3. One `register_document_provider(cx, Rc::new(MyProvider));` line in
   `documents::register_builtins`.

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
`src/app/skin.rs` is a from-scratch renderer. It copies the construction dance
from `DockSkin::dock_area` to get a `WeakEntity<DockArea>` — that weak handle is
the only way a tab bar can read dock open/closed state.

**A lone panel is never "draggable", so `is_panel_closable` is false for it.**
That is why the close button is gated on `handle.closable(cx)` instead, and why
a document asks for a `title_bar` even when it is the only tab — the tab is
where its close button and dirty marker live.

**`Panel::panel_name` returns `&'static str`.** A persisted name that must round
trip has to be interned; see `interned_name` in `skin.rs`.

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

## Verifying UI changes

Unit tests cannot see layout. `scripts/` drives the real window through Win32
input and writes screenshots to `%TEMP%\ll-shots`:

```powershell
pwsh -File scripts/drive-ui.ps1       # expand a folder, open a file
pwsh -File scripts/drive-edit.ps1     # open, type, Ctrl+S, report the md5
pwsh -File scripts/drive-session.ps1  # restart and check the layout came back
pwsh -File scripts/drive-close.ps1    # dirty close prompt, Save, md5
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
  to `docs/plan.md` §7.
- Doc comments on public items say what the thing is for and what it costs, not
  just what it does.
- Prefer a `type` alias over a complex inline type that clippy flags.
- Tests are named as sentences about behaviour
  (`changing_the_root_replaces_the_tree`), not as method names.
- No `unwrap()` in library paths that can fail at runtime; `expect` with a
  reason is fine where a failure is a wiring bug.

## Do not

- Do not add a dependency without a reason in a comment.
- Do not renumber or rename a `PaneProvider::id` or a panel name — it is written
  into the session file, and renaming turns every saved layout into an
  "Unavailable" tab.
- Do not edit `statics/` (the start page and URL reporter); they are embedded
  with `include_str!` and are not part of the shell.
- Do not commit `target/`, `node_modules/`, or a `session.json`.
