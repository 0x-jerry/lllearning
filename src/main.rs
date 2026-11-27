//! Application bootstrap.
//!
//! Everything here is wiring: install the fonts and the registries, read the
//! session, open the window on the shell. The shell itself, its panes and its
//! viewers live in their own modules.

use gpui::WindowOptions;
use gpui::component::TitleBar;

mod app;
mod documents;
mod icons;
mod logging;
mod panes;
mod state;
mod tray;
mod window;
mod workspace;

fn main() {
    // First, so the startup marks in `logging` time everything that follows.
    logging::init();

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
                log::error!("failed to create the tray icon: {error}");
            }
            logging::mark("fundation initialized");

            // The session is read before the window opens: the shell applies the
            // saved dock widths while it is constructed, so the first frame lays
            // the columns out at the widths the user chose.
            let session = state::load().unwrap_or_default();

            let options = WindowOptions {
                // The size and position the last run left, when they are still on
                // a display; centred at the default size otherwise.
                window_bounds: Some(app::shell::opening_bounds(&session, cx)),
                window_min_size: Some(app::shell::MIN_WINDOW_SIZE),
                // The title bar draws itself and owns the window drag.
                ..TitleBar::window_options()
            };

            let opened = gpui::open_window(options, cx, |window, cx| {
                app::shell::AppShell::new(session, window, cx)
            });

            logging::mark("gpui window opened");

            let (_, shell) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    log::error!("failed to open the window: {error}");
                    return;
                }
            };

            // The session save and the dirty-document warning ride on the quit
            // hook, which needs the finished shell.
            shell.update(cx, |shell, cx| shell.install_quit_hook(cx));
        });
}
