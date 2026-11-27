# lllearning

A desktop workspace for a folder of mixed files: a file tree on the left,
document tabs in the middle, and a real web browser on the right. A note, the
source it describes and the page it links to stay visible at once, and the
layout you leave is the layout you come back to.

## What opens

The file decides the viewer; there is no mode to pick.

| File | Opens as |
| --- | --- |
| `.md .markdown .mdx` | Markdown editor with live preview (split / editor / preview) |
| `.png .jpg .jpeg .gif .webp .bmp .ico .tif .tiff .avif .svg .qoi` | Image viewer with zoom steps |
| code and plain text | Syntax-highlighted editor, language by extension |
| binary, or over 8 MiB | An explanation instead of a frozen window |

The tree is read lazily and watched, so changes made outside the app land within
about half a second. Closing a dirty tab asks Save / Discard / Cancel in its own
small window, and a tab's right-click menu is one too — they have to be windows,
because the browser pane's page is native and paints over anything drawn inside
the main one. A file
changed on disk is flagged rather than written over your buffer. The folder,
tabs and pane widths survive a restart. Closing the window hides it to the tray;
Quit is in the tray menu.

## Run

```bash
cargo run --release
```

Windows is the primary target and needs the WebView2 runtime (preinstalled on
Windows 11). Linux needs X11/XWayland plus WebKitGTK 4.1 and GTK 3; macOS needs
nothing extra. For an installer, `cargo packager --release` writes
`target/release/lllearning_0.1.0_x64-setup.exe` and fetches NSIS on first use.

| Keys | Action |
| --- | --- |
| `Ctrl+S` / `Cmd+S` | Save the active document |
| `Enter` | Load the address in the browser bar, or the prompt's default button |
| Right-click a tab | Close, Close All, Close Others |
| `Escape` | Dismiss a prompt or a context menu |

## Develop

```bash
cargo test                         # 71 tests, ~0.3s
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Those three are the definition of done. On Windows, kill a running
`lllearning.exe` before `cargo build` — it holds the binary.

Rust on [GPUI Kit](https://gpui-kit.com), with the UI written in
[`rsx!`](https://docs.rs/gpui-rsx). The shell knows three seams and nothing else
— panes (`src/panes/`), document viewers (`src/documents/`) and the workspace
(`src/workspace/`) — so adding a pane or a viewer is one new module plus one
`register_*` line, with no edit under `src/app/`. The window itself is two
activity strips around an `h_resizable` group of three panels (left dock,
document tabs, right dock); there is no dock crate involved. A dock's strip is
part of the dock: closing one hides its panel and leaves the strip at the edge
of the centre.

Editing `assets/icon.svg` means regenerating the rasters with `bun install` then
`bun run build:icon`. The two icon fonts need no such step: `build/` subsets
lucide and the nerd font down to the glyphs the app actually draws, at build
time, so the binary does not carry a 1 MB font to paint thirteen icons. See
`build/main.rs` for which glyphs it keeps and how it finds them.

`AGENTS.md` carries the rest: the traps, the risk register, the session file and
the `scripts/` UI drivers. Read it before changing code.
