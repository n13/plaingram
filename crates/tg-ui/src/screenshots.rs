use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::{AppContext as _, HeadlessAppContext, px, size};
use tg_core::Demo;

use crate::app::Plaingram;

const SCREENSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/screenshots");
const GROUP_CHAT: i64 = 11;
const FORUM: i64 = 15;
const ANNOUNCEMENTS_TOPIC: usize = 1;
const GROUPS_FOLDER: usize = 2;

#[test]
fn capture_demo_screens() {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(gpui_apple::AppleTextSystem::new()),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(crate::theme::init).unwrap();
    std::fs::create_dir_all(SCREENSHOT_DIR).unwrap();
    for (name, signed_in, open) in [
        ("setup", false, None),
        ("group-chat", true, Some((GROUPS_FOLDER, GROUP_CHAT, None))),
        ("forum", true, Some((GROUPS_FOLDER, FORUM, Some(ANNOUNCEMENTS_TOPIC)))),
    ] {
        let (demo, events) = Demo::start(signed_in);
        let mut app = None;
        let window = cx
            .open_window(size(px(1100.), px(720.)), |window, cx| {
                let view = cx.new(|cx| Plaingram::new(Box::new(demo), events, window, cx));
                app = Some(view.clone());
                cx.new(|cx| Root::new(view, window, cx))
            })
            .unwrap();
        let app = app.unwrap();
        cx.run_until_parked();
        if let Some((folder, chat_id, topic)) = open {
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    app.select_folder(folder, cx);
                    app.open(chat_id, cx);
                })
            });
            cx.run_until_parked();
            if let Some(topic) = topic {
                cx.update(|cx| app.update(cx, |app, cx| app.select_topic(topic, cx)));
                cx.run_until_parked();
            }
        }
        for _ in 0..2 {
            cx.update_window(window.into(), |_, window, cx| _ = window.draw(cx)).unwrap();
            cx.advance_clock(Duration::from_secs(1));
        }
        let path = format!("{SCREENSHOT_DIR}/{name}.png");
        cx.capture_screenshot(window.into()).unwrap().save(&path).unwrap();
        println!("wrote {path}");
    }
}
