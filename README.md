# lllearning

GPUI Kit + Wry WebView demo: <https://gpui-kit.com/docs/webview>

## Run

```bash
cargo run
```

- Windows: needs WebView2 runtime (preinstalled on Windows 11). The app disables
  DirectComposition, which the child view requires.
- Linux: run on X11/XWayland (`gpui::platform::linux(WindowingModes::X11)` in `src/main.rs`)
  and install WebKitGTK 4.1 + GTK 3 dev packages before building.
- macOS: WKWebView, nothing extra.

The UI is written with the [`rsx!`](https://docs.rs/gpui-rsx) macro; `gpui-kit` is
imported under the `gpui` alias that `rsx!` expansions expect.

The toolbar drives navigation: Enter in the address input loads a URL, arrows run
`history.back`/`history.forward`, reload uses the raw Wry handle. Devtools are on in
debug builds (right-click → Inspect).

## Tray

`src/tray.rs` adds a `tray-icon` system tray (Windows Shell, macOS NSStatusItem, Linux
AppIndicator/StatusNotifierItem) with Show / Hide / Quit. Closing the window minimizes
it instead of exiting, and the app quits only from the tray: `QuitMode::Explicit` plus a
`on_window_should_close` guard. The tray icon is decoded from `assets/icon.png`.

## Package

```bash
cargo packager --release
```

Config lives in `[package.metadata.packager]`; icons are `assets/icon.png` and
`assets/icon.ico`.

## Icon

`assets/icon.svg` is the source (blue rounded square with three knocked-out vertical
bars). Regenerate the raster icons after editing it:

```bash
bun install
bun run build:icon
```

`assets/build-icon.ts` renders the SVG with `@resvg/resvg-js`, downscales and encodes
with `Bun.Image`, and writes `icon.png` (256×256) plus a multi-size `icon.ico`
(16–256 px). On Windows this produces `target/release/lllearning_0.1.0_x64-setup.exe`
(needs NSIS on `PATH` — cargo-packager downloads it on first run).
