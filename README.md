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
