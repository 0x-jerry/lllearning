use gpui::component::{
    ActiveTheme as _, IconName,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui::prelude::*;
use gpui::*;
use gpui_rsx::rsx;
use gpui_webview::WebView;

mod tray;

const HOME: &str = "https://gpui-kit.com/docs/webview";

/// A navigation reported by the webview.
struct PageLoad {
    url: String,
    loading: bool,
}

struct WebViewDemo {
    focus_handle: FocusHandle,
    webview: Entity<WebView>,
    address: Entity<InputState>,
    /// Whether the webview is currently navigating.
    loading: bool,
    /// Whether the address bar holds text the user typed but did not submit.
    address_edited: bool,
}

impl WebViewDemo {
    fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        // Page load events arrive on the platform's own callback, so they are
        // funnelled through a channel into a foreground task that updates the view.
        let (load_tx, load_rx) = gpui::base::async_util::unbounded::<PageLoad>();

        let webview = cx.new(|cx| {
            let builder =
                wry::WebViewBuilder::new().with_on_page_load_handler(move |event, url| {
                    let loading = matches!(event, wry::PageLoadEvent::Started);
                    let _ = load_tx.try_send(PageLoad { url, loading });
                });
            #[cfg(any(debug_assertions, feature = "inspector"))]
            let builder = builder.with_devtools(true);

            WebView::build(builder, window, cx).expect("Failed to create WebView")
        });
        webview.update(cx, |view, _| view.load_url(HOME));

        let address = cx.new(|cx| InputState::new(window, cx).default_value(HOME));
        tray::keep_window_on_close(window, cx);

        cx.new(|cx| {
            cx.subscribe(
                &address,
                |this: &mut Self, input, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.address_edited = true,
                    InputEvent::Blur => this.address_edited = false,
                    InputEvent::PressEnter { .. } => {
                        this.address_edited = false;
                        let url = input.read(cx).value();
                        this.webview.update(cx, |view, _| view.load_url(&url));
                        this.set_loading(true, cx);
                    }
                    _ => {}
                },
            )
            .detach();

            let load_rx = load_rx;
            cx.spawn(async move |this, cx| {
                while let Ok(page) = load_rx.recv().await {
                    if this
                        .update_in(cx, |this, window, cx| this.page_loaded(page, window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();

            Self {
                focus_handle: cx.focus_handle(),
                webview,
                address,
                loading: true,
                address_edited: false,
            }
        })
    }

    fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        if self.loading != loading {
            self.loading = loading;
            cx.notify();
        }
    }

    /// Reflect a navigation the webview started on its own, such as a link click
    /// or a redirect, in the toolbar.
    fn page_loaded(&mut self, page: PageLoad, window: &mut Window, cx: &mut Context<Self>) {
        self.set_loading(page.loading, cx);

        // Leave text the user is still editing alone until they submit or blur.
        let editing =
            self.address_edited && self.address.read(cx).focus_handle(cx).is_focused(window);
        if editing || self.address.read(cx).value().as_ref() == page.url {
            return;
        }
        self.address
            .update(cx, |state, cx| state.set_value(page.url, window, cx));
    }

    fn go_back(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.update(cx, |view, _| view.back());
    }

    fn go_forward(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.update(cx, |view, _| view.forward());
    }

    fn reload(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.read(cx).raw().reload();
        self.set_loading(true, cx);
    }

    fn stop(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        // wry has no stop API; `window.stop()` is supported by both engines.
        let _ = self
            .webview
            .read(cx)
            .raw()
            .evaluate_script("window.stop();");
        self.set_loading(false, cx);
    }
}

impl Focusable for WebViewDemo {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WebViewDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rsx! {
            <div class="flex flex-col gap-[12px] p-[8px] size-full">
                <div class="flex flex-row gap-[8px] items-center">
                    <Button("back") ghost icon={IconName::ChevronLeft}
                        onClick={cx.listener(Self::go_back)} />
                    <Button("forward") ghost icon={IconName::ChevronRight}
                        onClick={cx.listener(Self::go_forward)} />
                    {if self.loading {
                        rsx! {
                            <Button("stop") ghost icon={IconName::Close}
                                onClick={cx.listener(Self::stop)} />
                        }
                    } else {
                        rsx! {
                            <Button("reload") ghost icon={IconName::Redo}
                                onClick={cx.listener(Self::reload)} />
                        }
                    }}
                    <Input(&self.address) />
                </div>
                <div class="flex-1 border" border_color={cx.theme().border}>
                    {self.webview.clone()}
                </div>
            </div>
        }
    }
}

fn main() {
    // Without this, the WebView2 child view does not render on Windows.
    #[cfg(target_os = "windows")]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }

    // WebKitGTK only embeds into X11 windows, so Wayland sessions go through XWayland.
    #[cfg(target_os = "linux")]
    let app = gpui::platform::linux(WindowingModes::X11);
    #[cfg(not(target_os = "linux"))]
    let app = gpui::application();

    app.with_assets(gpui::assets::Assets)
        .with_quit_mode(tray::quit_mode())
        .run(|cx| {
            gpui::init(cx);

            if let Err(error) = tray::Tray::install(cx) {
                eprintln!("failed to create the tray icon: {error}");
            }

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(None, size(px(800.), px(600.)), cx)),
                ..Default::default()
            };

            gpui::open_window(options, cx, |window, cx| WebViewDemo::new(window, cx))
                .expect("Failed to open window");
        });
}
