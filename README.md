# lllearning

A desktop workspace for a folder of mixed files: file tree on the left, document
tabs in the middle, a real web browser on the right. The file decides the viewer;
the folder, tabs and pane widths survive a restart; closing the window leaves the
app in the tray.

## What opens

| File | Opens as |
| --- | --- |
| `.md .markdown .mdx` | Markdown editor with live preview (split / editor / preview) |
| `.png .jpg .jpeg .gif .webp .bmp .ico .tif .tiff .avif .svg .qoi` | Image viewer with zoom steps |
| code and plain text | Syntax-highlighted editor, language by extension |
| binary, or over 8 MiB | An explanation instead of a frozen window |

## Run

```bash
cargo run --release
```

Windows needs the WebView2 runtime (preinstalled on Windows 11). Linux needs
X11/XWayland plus WebKitGTK 4.1 and GTK 3; macOS needs nothing extra. An
installer comes from `cargo packager --release`.

## Release

```bash
bun run release patch        # or minor, major, or an explicit 1.2.3
bun run release minor --push # ...and push the commit and the tag
```

`scripts/release.ts` bumps `Cargo.toml`, commits and makes the tag; the tag
(`v` + the version in `Cargo.toml`) publishes the NSIS installer and a portable
zip of the exe through `.github/workflows/release.yml`.

## Develop

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Those are the definition of done. On Windows, kill a running `lllearning.exe`
before `cargo build` — it holds the binary. `assets/icon.svg` regenerates the
rasters with `bun run build:icon`.

`AGENTS.md` carries the layout, the seams and the traps. Read it before changing
code.
