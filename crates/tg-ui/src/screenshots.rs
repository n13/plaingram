use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::{
    AppContext as _, Entity, HeadlessAppContext, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput,
    WindowHandle, point, px, size,
};
use tg_core::Demo;

use crate::app::Plaingram;

const SCREENSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/screenshots");
const GROUP_CHAT: i64 = 11;
const FORUM: i64 = 15;
const ANNOUNCEMENTS_TOPIC: usize = 1;
const GROUPS_FOLDER: usize = 2;
const IMPOSTOR: i64 = 3;

#[test]
fn capture_demo_screens() {
    let mut cx = headless();
    std::fs::create_dir_all(SCREENSHOT_DIR).unwrap();
    for (name, signed_in, open, profile) in [
        ("setup", false, None, None),
        ("group-chat", true, Some((GROUP_CHAT, None)), None),
        ("forum", true, Some((FORUM, Some(ANNOUNCEMENTS_TOPIC))), None),
        ("forum-general", true, Some((FORUM, None)), None),
        ("profile", true, Some((FORUM, Some(ANNOUNCEMENTS_TOPIC))), Some(IMPOSTOR)),
    ] {
        let (window, app) = open_demo(&mut cx, signed_in);
        if let Some((chat_id, topic)) = open {
            open_chat(&mut cx, &app, chat_id);
            if let Some(topic) = topic {
                cx.update(|cx| app.update(cx, |app, cx| app.select_topic(topic, cx)));
                cx.run_until_parked();
            }
        }
        if let Some(user_id) = profile {
            cx.update(|cx| app.update(cx, |app, cx| app.open_profile(user_id, cx)));
            cx.run_until_parked();
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
    assert_eq!(loaded(&mut cx), 50, "nothing loads before the reader scrolls");

    scroll_to(&mut cx, &app, 10);
    let anchor = top_id(&mut cx, &app);
    draw(&mut cx, window);
    assert_eq!(loaded(&mut cx), 350, "three pages load ahead of the reader");
    assert_eq!(top_id(&mut cx, &app), anchor, "the reader stays on the same message");

    while top_id(&mut cx, &app) > 1 {
        scroll_to(&mut cx, &app, 0);
        draw(&mut cx, window);
    }
    assert_eq!(loaded(&mut cx), 1000);
    scroll_to(&mut cx, &app, 0);
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

/// The demo General topic has a photo every tenth message; the newest one is on screen at open.
#[test]
fn photos_load_only_on_or_approaching_the_screen() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, FORUM);
    draw(&mut cx, window);
    let requested = |cx: &mut HeadlessAppContext| {
        let mut ids: Vec<i64> = cx.update(|cx| app.read(cx).requested_photos.iter().map(|&id| id.into()).collect());
        ids.sort_unstable();
        ids
    };
    assert_eq!(requested(&mut cx), [1000], "only the photo on screen loads");
    assert_eq!(cx.update(|cx| app.read(cx).photo_progress.get(&1000).copied()), Some(0.42));

    scroll_to(&mut cx, &app, 10);
    draw(&mut cx, window);
    let (top, requested) = (top_id(&mut cx, &app), requested(&mut cx));
    assert!(
        requested.iter().any(|&id| id < top),
        "photos above load ahead of an upward scroll: {requested:?}, top {top}"
    );
    assert!(requested.iter().all(|&id| id + 31 >= top), "photos far above do not load: {requested:?}, top {top}");
}

fn top_id(cx: &mut HeadlessAppContext, app: &Entity<Plaingram>) -> i64 {
    cx.update(|cx| {
        let app = app.read(cx);
        app.messages[app.scroller.read(cx).logical_scroll_top().item_ix].id
    })
}

fn scroll_to(cx: &mut HeadlessAppContext, app: &Entity<Plaingram>, ix: usize) {
    cx.update(|cx| app.update(cx, |app, cx| app.scroller.update(cx, |s, cx| assert!(s.scroll_to_item(ix, cx)))));
}

/// Demo user 6 copies the name of the contact Bob Jones (4); the impostor (3) copies the handle of Alice Smith (2).
#[test]
fn profiles_warn_about_look_alikes_of_contacts() {
    let mut cx = headless();
    let (_, app) = open_demo(&mut cx, true);
    let lookalikes = |cx: &mut HeadlessAppContext, user_id| {
        cx.update(|cx| app.read(cx).lookalikes(user_id).iter().map(|user| user.id).collect::<Vec<_>>())
    };
    assert_eq!(lookalikes(&mut cx, 6), [4]);
    assert_eq!(lookalikes(&mut cx, IMPOSTOR), [2]);
    assert_eq!(lookalikes(&mut cx, 5), [] as [i64; 0]);
    assert_eq!(lookalikes(&mut cx, 4), [] as [i64; 0], "contacts are not flagged");
}

/// The newest message in the demo group chat is from Bob Jones (4), whose name is at about (330, 554).
#[test]
fn clicking_a_sender_opens_their_profile() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    click(&mut cx, window, 330., 554.);
    assert_eq!(cx.update(|cx| app.read(cx).profile), Some(4));
}

/// The forum's topic tabs are wider than the window; they must fold into a menu so the profile's Close button,
/// in the top-right corner, stays on screen.
#[test]
fn profile_fits_beside_a_forum_with_many_topics() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, FORUM);
    cx.update(|cx| app.update(cx, |app, cx| app.open_profile(IMPOSTOR, cx)));
    draw(&mut cx, window);
    click(&mut cx, window, 1060., 58.);
    assert_eq!(cx.update(|cx| app.read(cx).profile), None);
}

fn click(cx: &mut HeadlessAppContext, window: WindowHandle<Root>, x: f32, y: f32) {
    let (button, position, modifiers, click_count) = (MouseButton::Left, point(px(x), px(y)), Modifiers::default(), 1);
    cx.update_window(window.into(), |_, window, cx| {
        let down = MouseDownEvent { button, position, modifiers, click_count, first_mouse: false };
        window.dispatch_event(PlatformInput::MouseDown(down), cx);
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count }), cx);
    })
    .unwrap();
    cx.run_until_parked();
}
