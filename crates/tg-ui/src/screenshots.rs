use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::{AppContext as _, Entity, HeadlessAppContext, WindowHandle, px, size};
use tg_core::Demo;

use crate::app::Plaingram;

const SCREENSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/screenshots");
const GROUP_CHAT: i64 = 11;
const FORUM: i64 = 15;
const ANNOUNCEMENTS_TOPIC: usize = 1;
const GROUPS_FOLDER: usize = 2;

#[test]
fn capture_demo_screens() {
    let mut cx = headless();
    std::fs::create_dir_all(SCREENSHOT_DIR).unwrap();
    for (name, signed_in, open) in [
        ("setup", false, None),
        ("group-chat", true, Some((GROUP_CHAT, None))),
        ("forum", true, Some((FORUM, Some(ANNOUNCEMENTS_TOPIC)))),
    ] {
        let (window, app) = open_demo(&mut cx, signed_in);
        if let Some((chat_id, topic)) = open {
            open_chat(&mut cx, &app, chat_id);
            if let Some(topic) = topic {
                cx.update(|cx| app.update(cx, |app, cx| app.select_topic(topic, cx)));
                cx.run_until_parked();
            }
        }
        draw(&mut cx, window);
        let path = format!("{SCREENSHOT_DIR}/{name}.png");
        cx.capture_screenshot(window.into()).unwrap().save(&path).unwrap();
        println!("wrote {path}");
    }
}

/// The demo forum's General topic has 1000 messages and opens with the newest 50.
#[test]
fn scrolling_up_prefetches_older_messages() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, FORUM);
    draw(&mut cx, window);
    let loaded = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).messages.len());
    let top_id = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| {
            let app = app.read(cx);
            app.messages[app.scroller.read(cx).logical_scroll_top().item_ix].id
        })
    };
    let scroll_to = |cx: &mut HeadlessAppContext, ix: usize| {
        cx.update(|cx| app.update(cx, |app, cx| app.scroller.update(cx, |s, cx| assert!(s.scroll_to_item(ix, cx)))));
    };
    assert_eq!(loaded(&mut cx), 50, "nothing loads before the reader scrolls");

    scroll_to(&mut cx, 10);
    let anchor = top_id(&mut cx);
    draw(&mut cx, window);
    assert_eq!(loaded(&mut cx), 350, "three pages load ahead of the reader");
    assert_eq!(top_id(&mut cx), anchor, "the reader stays on the same message");

    while top_id(&mut cx) > 1 {
        scroll_to(&mut cx, 0);
        draw(&mut cx, window);
    }
    assert_eq!(loaded(&mut cx), 1000);
    scroll_to(&mut cx, 0);
    draw(&mut cx, window);
    assert_eq!(loaded(&mut cx), 1000, "loading stops at the first message");
}

fn headless() -> HeadlessAppContext {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(gpui_apple::AppleTextSystem::new()),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(crate::theme::init).unwrap();
    cx
}

fn open_demo(cx: &mut HeadlessAppContext, signed_in: bool) -> (WindowHandle<Root>, Entity<Plaingram>) {
    let (demo, events) = Demo::start(signed_in);
    let mut app = None;
    let window = cx
        .open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Plaingram::new(Box::new(demo), events, window, cx));
            app = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    (window, app.unwrap())
}

fn open_chat(cx: &mut HeadlessAppContext, app: &Entity<Plaingram>, chat_id: i64) {
    cx.update(|cx| {
        app.update(cx, |app, cx| {
            app.select_folder(GROUPS_FOLDER, cx);
            app.open(chat_id, cx);
        })
    });
    cx.run_until_parked();
}

fn draw(cx: &mut HeadlessAppContext, window: WindowHandle<Root>) {
    for _ in 0..2 {
        cx.update_window(window.into(), |_, window, cx| _ = window.draw(cx)).unwrap();
        cx.advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }
}
