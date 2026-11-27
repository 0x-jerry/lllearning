//! Application bootstrap: fonts, registries, session, window.

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
    // First, so the startup marks time everything that follows.
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
            icons::install_font(cx);
            app::init(cx);

            if let Err(error) = tray::Tray::install(cx) {
                log::error!("failed to create the tray icon: {error}");
            }
            logging::mark("fundation initialized");

            // Read before the window: the shell applies the saved widths as it builds.
            let session = state::load().unwrap_or_default();

            let options = WindowOptions {
                window_bounds: Some(app::shell::opening_bounds(&session, cx)),
                window_min_size: Some(app::shell::MIN_WINDOW_SIZE),
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

            // The quit hook saves the session and warns about dirty documents.
            shell.update(cx, |shell, cx| shell.install_quit_hook(cx));
        });
}
