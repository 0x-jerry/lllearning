use gpui::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui::prelude::*;
use gpui::*;
use gpui_rsx::rsx;
use gpui_webview::WebView;

mod tray;

/// Shown until the user navigates somewhere, and when they clear the address bar.
const START_PAGE_HTML: &str = include_str!("../statics/start.html");

/// The document URL the engines report for the start page.
const START_PAGE_URL: &str = "about:blank";

/// Whether `url` is the start page rather than a real page.
fn is_start_page(url: &str) -> bool {
    url.is_empty() || url == START_PAGE_URL
}

/// Prefix the injected script puts in front of the page URL it reports.
const URL_MESSAGE_PREFIX: &str = "url:";

/// Reports URL changes that load no new document, such as `#anchor` links and
/// `history.pushState`, which the page load handler never sees.
const URL_REPORTER_JS: &str = include_str!("../statics/url-reporter.js");

/// Something the webview reports about its current page.
enum WebViewEvent {
    /// A document started or finished loading at `url`.
    Load { url: String, loading: bool },
    /// The page changed its own URL without loading a new document.
    Url(String),
}

/// A navigation that could not be loaded, shown in place of the page.
struct LoadError {
    /// What the user asked for, before any URL normalisation.
    address: String,
    /// Why the navigation failed.
    message: String,
}

/// Turn what the user typed into a URL to navigate to.
///
/// A bare host such as `example.com` or `localhost:3000` gets an `https://`
/// scheme, so it loads instead of failing as an unsupported scheme. Anything
/// else is passed through unchanged: a real scheme keeps working, and input
/// that is not a URL at all reaches the webview as typed, where it fails and
/// the error page takes over.
fn resolve_address(input: &str) -> String {
    let input = input.trim();
    let is_bare_host = !input.contains("://")
        && input
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|authority| {
                let host = authority.split(':').next().unwrap_or_default();
                let mut labels = host.split('.');
                labels.next().is_some_and(|label| !label.is_empty())
                    && labels.all(|label| !label.is_empty())
            });

    if is_bare_host {
        format!("https://{input}")
    } else {
        input.to_owned()
    }
}

struct WebViewDemo {
    focus_handle: FocusHandle,
    webview: Entity<WebView>,
    address: Entity<InputState>,
    /// Whether the webview is currently navigating.
    loading: bool,
    /// Whether the address bar holds text the user typed but did not submit.
    address_edited: bool,
    /// Set while a failed navigation replaces the page.
    error: Option<LoadError>,
    /// Whether the page on screen is the start page, which has no URL to reload.
    start_page: bool,
}

impl WebViewDemo {
    fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        // Webview events arrive on the platform's own callback, so they are
        // funnelled through a channel into a foreground task that updates the view.
        let (event_tx, event_rx) = gpui::base::async_util::unbounded::<WebViewEvent>();

        let webview = cx.new(|cx| {
            let ipc_tx = event_tx.clone();
            let builder = wry::WebViewBuilder::new()
                .with_on_page_load_handler(move |event, url| {
                    let loading = matches!(event, wry::PageLoadEvent::Started);
                    let _ = event_tx.try_send(WebViewEvent::Load { url, loading });
                })
                .with_ipc_handler(move |request| {
                    if let Some(url) = request.body().strip_prefix(URL_MESSAGE_PREFIX) {
                        let _ = ipc_tx.try_send(WebViewEvent::Url(url.to_owned()));
                    }
                })
                .with_initialization_script(URL_REPORTER_JS);
            #[cfg(any(debug_assertions, feature = "inspector"))]
            let builder = builder.with_devtools(true);

            WebView::build(builder, window, cx).expect("Failed to create WebView")
        });
        webview.update(cx, |view, _| {
            let _ = view.raw().load_html(START_PAGE_HTML);
        });

        let address = cx.new(|cx| InputState::new(window, cx).placeholder("Enter an address"));
        tray::keep_window_on_close(window, cx);

        cx.new(|cx| {
            cx.subscribe(
                &address,
                |this: &mut Self, input, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.address_edited = true,
                    InputEvent::Blur => this.address_edited = false,
                    InputEvent::PressEnter { .. } => {
                        this.address_edited = false;
                        let input = input.read(cx).value();
                        this.navigate(&input, cx);
                    }
                    _ => {}
                },
            )
            .detach();

            let event_rx = event_rx;
            cx.spawn(async move |this, cx| {
                while let Ok(event) = event_rx.recv().await {
                    if this
                        .update_in(cx, |this, window, cx| this.webview_event(event, window, cx))
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
                error: None,
                start_page: true,
            }
        })
    }

    fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        if self.loading != loading {
            self.loading = loading;
            cx.notify();
        }
    }

    /// Reflect what the webview reports in the toolbar.
    fn webview_event(&mut self, event: WebViewEvent, window: &mut Window, cx: &mut Context<Self>) {
        let url = match event {
            WebViewEvent::Load { url, loading } => {
                // The content is reloading, so follow the event even when the
                // flag was already set optimistically by the click that started it.
                if self.loading != loading {
                    self.loading = loading;
                    cx.notify();
                }
                // A real page replaces the start page; the start page itself is
                // not a URL, so the address bar stays empty.
                self.start_page = is_start_page(&url);
                url
            }
            WebViewEvent::Url(url) => url,
        };
        let shown = if is_start_page(&url) {
            String::new()
        } else {
            url
        };
        self.set_address(shown, window, cx);
    }

    /// Show `url` in the address bar unless the user is editing it.
    fn set_address(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let editing =
            self.address_edited && self.address.read(cx).focus_handle(cx).is_focused(window);
        if editing || self.address.read(cx).value().as_ref() == url {
            return;
        }
        self.address
            .update(cx, |state, cx| state.set_value(url, window, cx));
    }

    /// Navigate to what the user typed. An empty address shows the start page;
    /// a navigation the webview cannot start at all replaces the page with an
    /// error view.
    fn navigate(&mut self, input: &str, cx: &mut Context<Self>) {
        // Show a stop button straight away rather than waiting for the load event.
        self.set_loading(true, cx);
        self.error = None;

        if input.trim().is_empty() {
            self.show_start_page(cx);
            return;
        }

        let url = resolve_address(input);
        let started = self
            .webview
            .read(cx)
            .raw()
            .load_url(&url)
            .map_err(|error| error.to_string());
        match started {
            Ok(()) => self.start_page = false,
            Err(message) => {
                // The engines report some bad input here and render their own
                // page for the rest, so a failure means nothing will load.
                self.error = Some(LoadError {
                    address: input.trim().to_owned(),
                    message,
                });
                self.set_loading(false, cx);
            }
        }
    }

    /// Replace the page with the built-in start page.
    fn show_start_page(&mut self, cx: &mut Context<Self>) {
        let loaded = self
            .webview
            .read(cx)
            .raw()
            .load_html(START_PAGE_HTML)
            .map_err(|error| error.to_string());
        self.start_page = loaded.is_ok();
        if let Err(message) = loaded {
            self.error = Some(LoadError {
                address: String::new(),
                message,
            });
        }
        self.set_loading(false, cx);
    }

    fn go_back(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.update(cx, |view, _| view.back());
        self.error = None;
        self.set_loading(true, cx);
    }

    fn go_forward(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.update(cx, |view, _| view.forward());
        self.error = None;
        self.set_loading(true, cx);
    }

    fn reload(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        // The start page is an in-memory document, so it is shown again rather
        // than reloaded through a URL it does not have.
        if self.start_page {
            self.set_loading(true, cx);
            self.show_start_page(cx);
            return;
        }
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
                    {if let Some(error) = &self.error {
                        rsx! {
                            <div class="flex flex-col gap-[12px] items-center justify-center size-full p-[24px]">
                                {Icon::new(IconName::TriangleAlert).large()
                                    .text_color(cx.theme().danger)}
                                <div class="text-lg font-semibold">{"This page could not be loaded"}</div>
                                <div class="text-sm" text_color={cx.theme().muted_foreground}>
                                    {error.address.clone()}
                                </div>
                                <div class="text-xs" text_color={cx.theme().muted_foreground}>
                                    {error.message.clone()}
                                </div>
                            </div>
                        }
                    } else {
                        rsx! { <div class="size-full">{self.webview.clone()}</div> }
                    }}
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
