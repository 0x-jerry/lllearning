//! Application bootstrap.
//!
//! Everything here is wiring: install the fonts and the registries, read the
//! session, open the window on the shell. The shell itself, its panes and its
//! viewers live in their own modules.

use gpui::component::TitleBar;
use gpui::{WindowBounds, WindowOptions, px, size};

mod app;
mod documents;
mod icons;
mod panes;
mod state;
mod tray;
mod window;
mod workspace;

fn main() {
    // Without this, the WebView2 child view does not render on Windows.
    #[cfg(target_os = "windows")]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }

    // WebKitGTK only embeds into X11 windows, so Wayland sessions go through XWayland.
    #[cfg(target_os = "linux")]
    let app = gpui::platform::linux(gpui::WindowingModes::X11);
    #[cfg(not(target_os = "linux"))]
    let app = gpui::application();

    app.with_assets(gpui::assets::Assets)
        .with_quit_mode(tray::quit_mode())
        .run(|cx| {
            gpui::init(cx);
            // The icon font must be installed before any pane draws a glyph.
            icons::install_font(cx);
            // The registries must exist before the shell asks what to mount.
            app::init(cx);

            if let Err(error) = tray::Tray::install(cx) {
                eprintln!("failed to create the tray icon: {error}");
            }

            // The session is read before the window opens: the dock's panel
            // builders run while the shell is constructed, and they need the
            // folder it names.
            let session = state::load().unwrap_or_default();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(None, size(px(1200.), px(760.)), cx)),
                window_min_size: Some(size(px(900.), px(600.))),
                // The title bar draws itself and owns the window drag.
                ..TitleBar::window_options()
            };

            let opened = gpui::open_window(options, cx, |window, cx| {
                app::shell::AppShell::new(session, window, cx)
            });

            let (_, shell) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    eprintln!("failed to open the window: {error}");
                    return;
                }
            };

            // The session save and the dirty-document warning ride on the quit
            // hook, which needs the finished shell.
            shell.update(cx, |shell, cx| shell.install_quit_hook(cx));
        });
}
