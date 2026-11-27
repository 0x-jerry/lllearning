use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();

pub fn init() {
    START.get_or_init(Instant::now);
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .init()
        .expect("`logging::init` must run before anything else installs a logger");
}

pub fn elapsed() -> Duration {
    START.get().map_or(Duration::ZERO, Instant::elapsed)
}

pub fn mark(what: &str) {
    log::info!("{what} at {:.1} ms", elapsed().as_secs_f64() * 1000.);
}

/// The callback runs at the start of the next frame-loop iteration, after this
/// frame's present — the earliest point the app can observe its own pixels —
/// and only the first call arms it.
pub fn arm_first_paint(window: &gpui::Window) {
    static ARMED: AtomicBool = AtomicBool::new(false);
    if ARMED.swap(true, Ordering::Relaxed) {
        return;
    }
    mark("first draw");
    window.on_next_frame(|_, _| mark("first frame presented"));
}
