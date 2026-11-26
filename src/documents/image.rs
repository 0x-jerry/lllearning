//! The image viewer.
//!
//! A read-only document: there is nothing to edit and nothing to save, so its
//! controller reports itself clean and every mutation is a no-op. The image is
//! drawn at its own pixel size and the wheel scales it; the pane clips whatever
//! no longer fits rather than shrinking the image to the pane.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, ImgResourceLoader,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Render, Resource,
    ScrollDelta, ScrollEvent, Styled as _, StyledImage as _, Window, div, img, px,
};
use gpui_rsx::rsx;

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
        DocumentContent::static_content(view.into(), controller)
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
    /// 1.0 is the image's own pixel size; the buttons and the wheel scale it.
    zoom: f32,
}

/// The zoom ladder the buttons step through.
const ZOOM_STEPS: [f32; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];

/// What one wheel line multiplies the zoom by. Multiplicative so a trackpad's
/// many small deltas feel like a mouse's few large ones.
const ZOOM_PER_LINE: f32 = 1.1;

/// One wheel line in pixels, for platforms that report a pixel delta.
const PIXELS_PER_LINE: f32 = 40.;

/// The range the wheel can reach, so the image cannot be scrolled out of sight.
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 8.;

impl ImageView {
    fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        Self {
            path,
            focus_handle: cx.focus_handle(),
            zoom: 1.0,
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
        self.set_zoom(next, cx);
    }

    fn zoom_out(&mut self, cx: &mut Context<Self>) {
        let next = ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|step| *step < self.zoom - f32::EPSILON)
            .unwrap_or(ZOOM_STEPS[0]);
        self.set_zoom(next, cx);
    }

    fn reset_zoom(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(1.0, cx);
    }

    fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        cx.notify();
    }

    /// Scale with the wheel. The event is consumed so a list around the viewer
    /// does not scroll while the image zooms.
    fn on_scroll(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / PIXELS_PER_LINE,
        };
        if lines == 0. {
            return;
        }
        cx.stop_propagation();
        let zoom = self.zoom * ZOOM_PER_LINE.powf(lines);
        self.set_zoom(zoom, cx);
    }

    fn dimensions_label(size: Option<(f32, f32)>) -> String {
        match size {
            Some((width, height)) => format!("{} × {}", width.round(), height.round()),
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.path.clone();
        let zoom = self.zoom;
        let zoom_label = format!("{:.0}%", zoom * 100.);
        let size = image_size(window, &path, cx);
        let dimensions = Self::dimensions_label(size);
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let muted_foreground = cx.theme().muted_foreground;

        rsx! {
            <div class="size-full flex flex-col">
                <div
                    class="flex flex-row items-center gap-1 px-2 py-1"
                    border_b_1
                    border_color={border}
                >
                    <Button("image-zoom-out")
                        label="−"
                        xsmall
                        ghost
                        onClick={cx.listener(|this, _, _, cx| this.zoom_out(cx))}
                    />
                    <Button("image-zoom-reset")
                        label={zoom_label}
                        xsmall
                        ghost
                        onClick={cx.listener(|this, _, _, cx| this.reset_zoom(cx))}
                    />
                    <Button("image-zoom-in")
                        label="+"
                        xsmall
                        ghost
                        onClick={cx.listener(|this, _, _, cx| this.zoom_in(cx))}
                    />
                    <div class="text-xs" text_color={muted_foreground}>{dimensions}</div>
                </div>
                <div
                    id="image-canvas"
                    class="flex flex-row flex-1 min-h-0 overflow-hidden items-center justify-center"
                    bg={muted}
                    onScroll={cx.listener(|this, event: &ScrollEvent, _, cx| {
                        this.on_scroll(event, cx)
                    })}
                >
                    {match size {
                        Some((width, height)) => rsx! {
                            <img
                                src={path}
                                objectFit={ObjectFit::Fill}
                                w={px(width * zoom)}
                                h={px(height * zoom)}
                                flex_none
                            />
                        },
                        // A format with no readable header (SVG) keeps its own
                        // size and cannot be scaled: there is nothing to
                        // multiply.
                        None => rsx! {
                            <img
                                src={path}
                                objectFit={ObjectFit::Contain}
                                max_w_full
                                max_h_full
                            />
                        },
                    }}
                </div>
            </div>
        }
    }
}

/// The image's pixel size, as the element will draw it.
///
/// Asked of GPUI's own image loader rather than decoded here: the loader is
/// already reading the file for the element, so this shares that one decode and
/// covers every format the element supports instead of PNG alone. `None` means
/// the file is not loaded yet (or cannot be), which also covers SVG — it has no
/// pixel size until the renderer picks one, and stays unzoomed.
fn image_size(window: &mut Window, path: &Path, cx: &mut App) -> Option<(f32, f32)> {
    let resource = Resource::from(path.to_path_buf());
    let image = window.use_asset::<ImgResourceLoader>(&resource, cx)?.ok()?;
    let size = image.size(0);
    (size.width.0 > 0 && size.height.0 > 0).then_some((size.width.0 as f32, size.height.0 as f32))
}
