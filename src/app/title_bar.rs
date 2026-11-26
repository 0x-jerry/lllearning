//! The title bar.
//!
//! `gpui_component::TitleBar` already owns the window drag, the double-click
//! zoom and the native window controls, so this only supplies the app mark on
//! the left. The folder actions live on the explorer's own toolbar, which is
//! the pane they act on.

use std::sync::{Arc, OnceLock};

use gpui::component::TitleBar;
use gpui::{
    AnyElement, Image, ImageFormat, IntoElement, ParentElement as _, Styled as _, div, img, px,
};
use gpui_rsx::rsx;

/// The app icon, decoded once for the life of the process.
///
/// It is embedded rather than loaded through the asset source so the mark needs
/// no asset path and no `img` resource lookup: the bytes are already in the
/// binary, and the decoder is keyed on their hash, so one `Arc` serves every
/// frame.
fn logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/icon.png").to_vec(),
        ))
    })
    .clone()
}

/// The title bar: the app icon, then the app name.
pub fn render() -> AnyElement {
    let icon = logo();

    rsx! {
        <TitleBar base={TitleBar::new()}>
            <div class="flex items-center gap-2">
                <img src={icon} w={px(16.)} h={px(16.)} />
                <div class="text-sm">{"lllearning"}</div>
            </div>
        </TitleBar>
    }
    .into_any_element()
}
