mod app;
mod safe_text;
#[cfg(all(test, target_os = "macos"))]
mod screenshots;
mod theme;

use std::fs::{self, OpenOptions};
use std::rc::Rc;
use std::sync::Mutex;

use anyhow::{Context as _, Result};
use gpui_kit::component::TitleBar;
use gpui_kit::*;
use tg_core::{Backend, Events, log_file};
use tracing_subscriber::fmt::writer::MakeWriterExt as _;

fn main() -> Result<()> {
    let log = log_file()?;
    fs::create_dir_all(log.parent().context("log file has no folder")?)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .with_context(|| format!("cannot open {}", log.display()))?;
    tracing_subscriber::fmt().with_ansi(false).with_writer(std::io::stdout.and(Mutex::new(log))).init();
    let (backend, events) = start_backend()?;
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        if let Err(error) = open(backend, events, cx) {
            tracing::error!("failed to open window: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}

fn open(backend: Rc<dyn Backend>, events: Events, cx: &mut App) -> Result<()> {
    theme::init(cx)?;
    app_menu(cx);
    cx.on_window_closed(|cx, _| cx.quit()).detach();
    let window = TitleBar::window_options();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1100.), px(720.)), cx))),
        titlebar: window.titlebar.map(|titlebar| TitlebarOptions { title: Some("plaingram".into()), ..titlebar }),
        ..window
    };
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| app::Plaingram::new(backend, events, window, cx)))?;
    Ok(())
}

actions!(plaingram, [Quit, Hide]);

/// The app menu with the shortcuts Mac users expect: Cmd-H hides plaingram, Cmd-Q (Ctrl-Q on Linux) quits it.
pub(crate) fn app_menu(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None), KeyBinding::new("cmd-h", Hide, None)]);
    #[cfg(target_os = "linux")]
    cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
    let items =
        vec![MenuItem::action("Hide plaingram", Hide), MenuItem::separator(), MenuItem::action("Quit plaingram", Quit)];
    cx.set_menus([Menu { name: "plaingram".into(), items, disabled: false }]);
}

#[cfg(not(feature = "demo"))]
fn start_backend() -> Result<(Rc<dyn Backend>, Events)> {
    let (client, events) = tg_core::Client::start()?;
    Ok((Rc::new(client), events))
}

#[cfg(feature = "demo")]
fn start_backend() -> Result<(Rc<dyn Backend>, Events)> {
    let (demo, events) = tg_core::Demo::start(false);
    Ok((Rc::new(demo), events))
}
