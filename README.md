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
| `.mp4 .webm .mov .mkv .avi` | An explanation, plus a button to open it in the browser pane |
| binary, or over 8 MiB | An explanation instead of a frozen window |

The tree is read lazily and watched, so changes made outside the app land within
about half a second. Closing a dirty tab asks Save / Discard / Cancel, and a file
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
| `Enter` | Load the address in the browser bar |

## Develop

```bash
cargo test                         # 67 tests, ~0.3s
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Those three are the definition of done. On Windows, kill a running
`lllearning.exe` before `cargo build` — it holds the binary.

Rust on [GPUI Kit](https://gpui-kit.com), with the UI written in
[`rsx!`](https://docs.rs/gpui-rsx). The shell knows three seams and nothing else
— panes (`src/panes/`), document viewers (`src/documents/`) and the workspace
(`src/workspace/`) — so adding a pane or a viewer is one new module plus one
`register_*` line, with no edit under `src/app/`.

Editing `assets/icon.svg` means regenerating the rasters with `bun install` then
`bun run build:icon`. The explorer's file-type icons come from
[`devicons`](https://github.com/alexpasmantier/rust-devicons), which needs the
nerd font vendored in `assets/` — it is embedded the same way `lucide.ttf` is.

`AGENTS.md` carries the rest: the traps, the risk register, the session file and
the `scripts/` UI drivers. Read it before changing code.
