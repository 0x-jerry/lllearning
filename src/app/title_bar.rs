//! The title bar: `gpui_component::TitleBar` owns the drag, the zoom and the
//! native controls; this supplies the app mark.

use std::sync::{Arc, OnceLock};

use gpui::component::TitleBar;
use gpui::{
    AnyElement, Image, ImageFormat, IntoElement, ParentElement as _, Styled as _, div, img, px,
};
use gpui_rsx::rsx;

/// The app icon, decoded once and embedded so it needs no asset lookup.
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

/// The app icon, then the app name.
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
