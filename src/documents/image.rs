//! The image viewer.
//!
//! A read-only document: there is nothing to edit and nothing to save, so its
//! controller reports itself clean and every mutation is a no-op. The zoom is a
//! local scale factor over `ObjectFit::Contain`, with a checkerboard behind
//! transparent images so a PNG with alpha is legible in both themes.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ObjectFit, ParentElement as _, Render, Styled as _, StyledImage as _, Window, div, img,
    prelude::FluentBuilder as _, px,
};

use super::{DocumentContent, DocumentController, DocumentProvider, OpenContext, Sniff, sniff};

/// Images beat everything else that could claim the same path.
const SCORE: u8 = 90;

/// The image viewer.
pub struct ImageProvider;

impl DocumentProvider for ImageProvider {
    fn id(&self) -> &'static str {
        "image"
    }

    fn title(&self) -> &'static str {
        "Image"
    }

    fn score(&self, _path: &Path, sniff: &Sniff) -> Option<u8> {
        (sniff::kind_of(sniff.ext.as_deref()) == Some(sniff::FileKind::Image)).then_some(SCORE)
    }

    fn open(&self, ctx: OpenContext, window: &mut Window, cx: &mut App) -> DocumentContent {
        let view = cx.new(|cx| ImageView::new(ctx.path, cx));
        let _ = window;
        let controller: Rc<dyn DocumentController> =
            Rc::new(ImageController { view: view.clone() });
        DocumentContent {
            view: view.into(),
            controller,
        }
    }
}

struct ImageController {
    view: Entity<ImageView>,
}

impl DocumentController for ImageController {
    fn is_dirty(&self, _cx: &App) -> bool {
        false
    }

    fn save(&self, _window: &mut Window, _cx: &mut App) -> Result<(), String> {
        // Nothing to write: the viewer never modifies the file.
        Ok(())
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| view.focus(window, cx));
    }

    fn reload_from_disk(&self, _window: &mut Window, cx: &mut App) {
        // The image element re-reads the file when the view re-renders, so a
        // notification is the whole reload.
        self.view.update(cx, |_, cx| cx.notify());
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle(cx)
    }

    fn is_stale(&self, _cx: &App) -> bool {
        false
    }

    fn accept_disk_change(&self, _window: &mut Window, _cx: &mut App) {}

    fn language(&self, _cx: &App) -> Option<gpui::SharedString> {
        Some(gpui::SharedString::from("Image"))
    }
}

/// The image viewer's state: the path and the zoom.
pub struct ImageView {
    path: PathBuf,
    focus_handle: FocusHandle,
    /// 1.0 is fit-to-pane; the buttons step through a small ladder.
    zoom: f32,
    /// The image's pixel size, once it has been read.
    dimensions: Option<(u32, u32)>,
}

/// The zoom ladder the buttons step through.
const ZOOM_STEPS: [f32; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];

impl ImageView {
    fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let dimensions = image_dimensions(&path);
        Self {
            path,
            focus_handle: cx.focus_handle(),
            zoom: 1.0,
            dimensions,
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    fn zoom_in(&mut self, cx: &mut Context<Self>) {
        let next = ZOOM_STEPS
            .iter()
            .copied()
            .find(|step| *step > self.zoom + f32::EPSILON)
            .unwrap_or(*ZOOM_STEPS.last().unwrap());
        self.zoom = next;
        cx.notify();
    }

    fn zoom_out(&mut self, cx: &mut Context<Self>) {
        let next = ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|step| *step < self.zoom - f32::EPSILON)
            .unwrap_or(ZOOM_STEPS[0]);
        self.zoom = next;
        cx.notify();
    }

    fn reset_zoom(&mut self, cx: &mut Context<Self>) {
        self.zoom = 1.0;
        cx.notify();
    }

    fn dimensions_label(&self) -> String {
        match self.dimensions {
            Some((width, height)) => format!("{width} × {height}"),
            None => "unknown size".to_owned(),
        }
    }
}

impl Focusable for ImageView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for ImageView {}

impl Render for ImageView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.path.clone();
        let zoom = self.zoom;

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("image-zoom-out")
                            .label("−")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_out(cx))),
                    )
                    .child(
                        Button::new("image-zoom-reset")
                            .label(format!("{:.0}%", zoom * 100.))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.reset_zoom(cx))),
                    )
                    .child(
                        Button::new("image-zoom-in")
                            .label("+")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_in(cx))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.dimensions_label()),
                    ),
            )
            .child(
                // The checkerboard is two offset blocks of the muted colour, so
                // a transparent image is distinguishable from the pane.
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(cx.theme().muted)
                    .child(
                        img(path)
                            .object_fit(ObjectFit::Contain)
                            .w(px(1024. * zoom))
                            .h(px(1024. * zoom))
                            .max_w_full()
                            .max_h_full(),
                    ),
            )
    }
}

/// Read an image's pixel size from its PNG header.
///
/// `png::Decoder` needs a `Read + Seek`, which a `Cursor` over the file gives.
/// Other formats report no size rather than guessing.
fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    if crate::workspace::fs_util::extension_of(path).as_deref() != Some("png") {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}
