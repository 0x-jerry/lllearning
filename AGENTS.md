# AGENTS.md

A three-column desktop workspace in Rust on GPUI Kit: file tree on the left,
document tabs in the middle, a WebView2 browser on the right.

## Commands

```bash
cargo check --all-targets
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

All of these must pass; `-D warnings` means a warning is a failure. Skip
`cargo build --release`: fat LTO makes it a ~9 minute rebuild, run once per
version.

On Windows a running `lllearning.exe` holds the binary, so `cargo build` fails
with `Access is denied`; kill the process first.

## Layout

```
src/main.rs           bootstrap: fonts, registries, session, open_window
src/app/              shell, side_view, documents_view, activity_bar, title_bar, popup/
src/panes/            explorer, browser, outline
src/documents/        text, markdown, image, refused, sniff, status_bar_helper
src/workspace/        folder root, lazy tree, notify watcher, fs helpers
src/state.rs          the session file
src/icons.rs          icon glyphs as text (lucide, nerd font for file types)
src/window.rs         Win32 hide/show, the owned handle a webview build awaits
src/tray.rs           tray icon; hide-on-close
build/                subsets the icon fonts, compiles the exe's own icon
```

## The seams

The shell knows only three: `PaneRegistry`, `DocumentRegistry` and `Workspace`.
If a change needs an edit in `src/app/shell.rs` to add a pane or a viewer, the
seam is being bypassed.

- **Side pane:** a file in `src/panes/` with a `PaneProvider` whose view is
  `Focusable` and `Render`, returning `PaneContent::new(view)`, plus one
  `register_pane` line in `panes::register_builtins`. A pane whose contents live
  outside the element tree supplies `PaneContent::activation`; one that keeps
  state supplies a `PaneController`.
- **Viewer:** a file in `src/documents/` with a `DocumentProvider` and a
  `DocumentController`, plus one `register_document_provider` line. `score`
  picks it — image 90, markdown 70, text 10, refused 1 — and `refused` must stay
  last so a binary file opens an explanation instead of nothing. `watch` is how
  a viewer tells panes its live buffer changed; `outline` is what the outline
  pane draws.
- Views report, the shell decides: `SideView::show`, `DocumentsView::remove`
  return what changed and never write the session, reveal a file or emit
  `ActiveDocumentChanged` themselves.

## Traps

- `rsx!` spacing is Tailwind only because `rsx.toml` says so; without it
  `p-2` is 2 px. It expands to `div()`, `px()` and builder calls, so every
  module needs `InteractiveElement`, `ParentElement`, `Styled` and
  `prelude::FluentBuilder` imported — the error names a missing method, not an
  import.
- A closed dock's panel is hidden and its strip is drawn in the centre row: a
  panel merely clamped keeps a divider that moves nothing. Toggle
  `ResizablePanel::visible` instead of omitting a panel, because the saved
  widths are read by panel index.
- A hidden panel measures zero, so the last dock width lives in
  `AppShell::widths`; `ResizablePanel::size` is the initial size only. Docks are
  `.fixed()`, or a window resize rescales them.
- Popups are windows (`app::popup`): `window.open_dialog` and popovers are
  covered by the webview. Open one outside a draw and keep the returned `Task`.
- Panes are built once and cached, and only while their dock is open; a
  collapsed dock builds nothing.
- The browser page is a native child window, so `PaneContent::activation` hides
  it, and it may arrive after the dock that wanted it closed
  (`wanted_visible`, `pending_url`).
- `WebView::build` blocks the render thread inside a nested Win32 pump; build
  the page with `build_as_child_async` from a kept task. Nothing expensive
  belongs in the window's build closure, which runs before the first frame —
  the session restore is deferred past it (see the startup marks in `logging`).
- `Workspace::set_root` emits `RootChanged`, which closes every tab; the restore
  raises `AppShell::restoring` around its own call.
- A pane is built from inside `shell.update`, so it must not read the shell in
  its constructor: subscribe with the weak entity and read it in `defer_in`.
- Dirty tracking subscribes to `InputEvent::Change` only, and restores with
  `EditorState::set_value`, which emits no change.
- `Ctrl+S` is bound to the `DocumentViewer` key context; a new editor viewer
  must set it on its root element.
- `#![allow(dead_code)]` on `documents`, `workspace` and a few shell items is
  deliberate: the lint cannot see a test-only consumer.

## Style

- Comments explain why, not what, and are rare. Doc comments say what a public
  item is for.
- **Prefer `rsx!` markup wherever it can express the element.** A layout reads
  as structure in `rsx!`; use the builder chain only for what markup cannot say
  — a runtime `match` around a whole element, a `map` over children, an element
  handed to a function. It expands to the same calls, so this costs nothing.
- Tests are named as sentences about behaviour. No `unwrap()` in runtime paths;
  `expect` with a reason is fine where a failure is a wiring bug.
