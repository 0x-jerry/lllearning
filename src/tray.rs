use std::{error::Error, io::Cursor, time::Duration};

use gpui::{App, AsyncApp, Global, QuitMode};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

const SHOW: &str = "show";
const HIDE: &str = "hide";
const QUIT: &str = "quit";

pub(crate) struct Tray(#[expect(dead_code, reason = "dropping it removes the tray icon")] TrayIcon);

impl Global for Tray {}

impl Tray {
    pub(crate) fn install(cx: &mut App) -> Result<(), Box<dyn Error>> {
        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id(SHOW, "Show", true, None),
            &MenuItem::with_id(HIDE, "Hide", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(QUIT, "Quit", true, None),
        ])?;

        let icon = TrayIconBuilder::new()
            .with_tooltip("lllearning")
            .with_icon(decode_icon(include_bytes!("../assets/icon.png"))?)
            .with_menu(Box::new(menu))
            .build()?;

        cx.set_global(Self(icon));
        forward_menu_events(cx);
        Ok(())
    }
}

/// Closing the window must not end the app: the tray stays until Quit is chosen.
pub(crate) fn quit_mode() -> QuitMode {
    QuitMode::Explicit
}

/// Keeps the window alive on close, so the tray can bring it back.
pub(crate) fn keep_window_on_close(window: &gpui::Window, cx: &App) {
    window.on_window_should_close(cx, |window, _| {
        window.minimize_window();
        false
    });
}

fn decode_icon(png: &[u8]) -> Result<Icon, Box<dyn Error>> {
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("icon is too large")?];
    let frame = reader.next_frame(&mut buf)?;
    if frame.color_type != png::ColorType::Rgba {
        return Err(format!("tray icon must be RGBA, got {:?}", frame.color_type).into());
    }
    buf.truncate(frame.buffer_size());
    Ok(Icon::from_rgba(buf, frame.width, frame.height)?)
}

/// `muda` delivers menu events on a blocking channel, so poll it from the foreground.
fn forward_menu_events(cx: &App) {
    let events = MenuEvent::receiver().clone();

    cx.spawn(async move |cx| {
        loop {
            while let Ok(event) = events.try_recv() {
                on_menu(&event, cx);
            }
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
        }
    })
    .detach();
}

fn on_menu(event: &MenuEvent, cx: &AsyncApp) {
    match event.id.0.as_str() {
        SHOW => window(cx, |window| window.activate_window()),
        HIDE => window(cx, |window| window.minimize_window()),
        QUIT => cx.update(|cx| cx.quit()),
        _ => {}
    }
}

fn window(cx: &AsyncApp, run: impl FnOnce(&gpui::Window)) {
    cx.update(|cx| {
        let Some(handle) = cx
            .active_window()
            .or_else(|| cx.window_stack().and_then(|stack| stack.first().copied()))
        else {
            return;
        };
        handle.update(cx, |_, window, _| run(window)).ok();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icon_decodes_to_rgba() {
        let icon = decode_icon(include_bytes!("../assets/icon.png")).expect("icon decodes");
        let _ = icon;
    }
}
