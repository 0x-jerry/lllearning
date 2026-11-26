# lllearning

A three-column workspace app built on [GPUI Kit](https://gpui-kit.com): a file
tree on the left, document tabs in the middle, and a real browser on the right.

```
┌──────────────────────────────────────────────────────────────────────┐
│ title bar: app mark · change folder · refresh          window controls│
├────┬──────────────────┬───────────────────────────┬──────────────┬────┤
│ ▲  │ Explorer │Outline│ main.rs • │ README.md │  │ ← → ⟳ address│ ▲  │
│ ▮  │ ▾ llllearning    │ 1  use gpui::…            │              │ ▮  │
│ ▮  │   ▸ assets       │ 2                         │  the page    │ ▮  │
│ ▮  │   ▾ src          │ …                         │              │ ▮  │
├────┴──────────────────┴───────────────────────────┴──────────────┴────┤
│ ~/lllearning │ main.rs │ Rust │ Ln 3, Col 1 │ Unsaved │   Watching    │
└──────────────────────────────────────────────────────────────────────┘
```

## Run

```bash
cargo run
```

- **Windows**: needs the WebView2 runtime (preinstalled on Windows 11). The app
  disables DirectComposition, which the webview child view requires.
- **Linux**: run on X11/XWayland (`gpui::platform::linux(WindowingModes::X11)` in
  `src/main.rs`) and install WebKitGTK 4.1 + GTK 3 dev packages before building.
- **macOS**: WKWebView, nothing extra.

The UI is written with the [`rsx!`](https://docs.rs/gpui-rsx) macro; `gpui-kit`
is imported under the `gpui` alias that `rsx!` expansions expect.

## Architecture

The shell knows **three seams** and nothing else. It has no idea what a file, an
editor or a webview is, which is what makes each part replaceable.

```
src/app/shell.rs        AppShell: title bar · activity strips · dock area · footer
src/app/skin.rs         ShellSkin: the dock's appearance (frames, dividers, tabs)
src/panes/              the side-pane seam
src/documents/          the document seam
src/workspace/          the folder, its tree and its watcher
```

### The pane seam — `src/panes/`

A side pane is a `PaneProvider`: an id, a title, an icon, which side it belongs
to, and a `build` that returns the panel. `register_pane` also installs the
dock's own panel builder, so a persisted layout rebuilds through the same
provider.

**To add a side pane**, write one module and add one line to
`panes::register_builtins`:

```rust
register_pane(cx, Rc::new(MyPane));
```

Nothing in `app/shell.rs` or `app/skin.rs` changes. `panes/outline.rs` exists to
prove this: it is a module and one registration line, and the left strip gained
a third button.

### The document seam — `src/documents/`

A `DocumentProvider` scores a file: `score(path, sniff) -> Option<u8>`, higher
wins, `None` means "cannot open". The registry takes the best score, so the kind
of viewer is decided by file type and never by the user.

| Provider | Score | Claims |
| --- | --- | --- |
| `image` | 90 | `.png .jpg .jpeg .gif .webp .bmp .ico .tif .tiff .avif .svg .qoi` |
| `video` | 80 | `.mp4 .webm .mov .mkv .avi` — a placeholder; see below |
| `markdown` | 70 | `.md .markdown .mdx` — editor + live preview |
| `text` | 10 | anything that is not binary and not over 8 MiB |
| `refused` | 1 | what text declined, so it opens an explanation, not nothing |

**To add a viewer**, write one module implementing `DocumentProvider` and
`DocumentController`, then add one line to `documents::register_builtins`.

The score table is asserted by tests, so what opens a `.png`, a `.md`, a `.rs`, a
binary file and an oversized file is a contract rather than a claim.

### The workspace — `src/workspace/`

Owns the folder root, a lazily-read directory cache, the tree state, and a
`notify` watcher. Nothing is read until a directory is expanded; the watcher
batches events with a 250 ms debounce and ignores the same paths the listing
does (`target`, `node_modules`, `.git`, dot-files).

## Keyboard

| Keys | Action |
| --- | --- |
| `Ctrl+S` / `Cmd+S` | Save the active document |

The binding is scoped to a `DocumentViewer` key context that both editor
viewers set, so no other input in the window can claim it.

## Session

The folder, the open tabs, the active tab, the pane widths and each dock's
selected pane survive a restart. The file is
`dirs::config_dir()/lllearning/session.json`:

- written on a 500 ms debounce after a layout change, and once on quit;
- read before the first window opens;
- a missing, malformed or version-mismatched file falls back to the default
  layout with one line on stderr — a bad session never stops the app starting.

## Tray

`src/tray.rs` adds a `tray-icon` system tray (Windows Shell, macOS
NSStatusItem, Linux AppIndicator/StatusNotifierItem) with Show / Hide / Quit.
Closing the window hides it instead of exiting, and the app quits only from the
tray: `QuitMode::Explicit` plus an `on_window_should_close` guard. The tray icon
is decoded from `assets/icon.png`.

## Checks

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`scripts/` holds the manual pass as runnable scripts, which drive the real
window through Win32 input:

```powershell
pwsh -File scripts/drive-ui.ps1       # expand a folder, open a file
pwsh -File scripts/drive-edit.ps1     # open, type, Ctrl+S, report the hash
pwsh -File scripts/drive-session.ps1  # restart and check the layout came back
```

They write screenshots to `%TEMP%\ll-shots`.

## Known limitations

- **The webview covers GPUI overlays.** WebView2 cannot be clipped to a partial
  region on Windows, so a dialog or dropdown opened over the browser pane is
  hidden behind the page. Dialogs are rendered at the window root and the
  browser pane never anchors a dropdown; the dirty-close prompt is unaffected
  because it is centred over the window.
- **No video playback.** GPUI has no video element. The `video` provider claims
  video extensions, explains why it cannot play them, and offers to open the
  file in the browser pane, which can. The seam is unaffected — swapping in a
  real viewer is one provider.
- **A second instance cannot start the webview** while the first holds the
  WebView2 profile lock. The browser pane reports it instead of panicking.
- **`Ctrl+S` is the only document shortcut.** No save-as, no close-tab binding,
  no find.

## Package

```bash
cargo packager --release
```

Config lives in `[package.metadata.packager]`; icons are `assets/icon.png` and
`assets/icon.ico`.

## Icon

`assets/icon.svg` is the source (blue rounded square with three knocked-out
vertical bars). Regenerate the raster icons after editing it:

```bash
bun install
bun run build:icon
```

`assets/build-icon.ts` renders the SVG with `@resvg/resvg-js`, downscales and
encodes with `Bun.Image`, and writes `icon.png` (256×256) plus a multi-size
`icon.ico` (16–256 px). On Windows this produces
`target/release/lllearning_0.1.0_x64-setup.exe` (needs NSIS on `PATH` —
cargo-packager downloads it on first run).
