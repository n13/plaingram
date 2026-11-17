use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::{
    AppContext as _, Entity, ExternalPaths, FileDropEvent, HeadlessAppContext, KeyDownEvent, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, WindowHandle, point, px, size,
};
use tg_core::{AuthStep, Demo, SafeContent};

use crate::app::{Overlay, Plaingram, Viewer};

const SCREENSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/screenshots");
const GROUP_CHAT: i64 = 11;
const FORUM: i64 = 15;
const ANNOUNCEMENTS_TOPIC: usize = 1;
const GROUPS_FOLDER: usize = 2;
const IMPOSTOR: i64 = 3;
const ALICE: i64 = 10;
const BOB: i64 = 12;
const PAYPAL: i64 = 13;
const FAMILY: i64 = 14;
const SAVED_MESSAGES: i64 = 16;

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
            cx.update(|cx| app.update(cx, |app, cx| app.toggle_profile(user_id, cx)));
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
            let view = cx.new(|cx| Plaingram::new(Rc::new(demo), events, window, cx));
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
        let mut ids: Vec<i64> = cx.update(|cx| app.read(cx).requested_photos.keys().map(|&id| id.into()).collect());
        ids.sort_unstable();
        ids
    };
    let on_screen = |cx: &mut HeadlessAppContext, id: i64| {
        cx.update(|cx| app.read(cx).requested_photos[&i32::try_from(id).unwrap()])
    };
    assert_eq!(requested(&mut cx), [1000], "only the photo on screen loads");
    assert!(on_screen(&mut cx, 1000), "the photo on screen downloads first");
    assert_eq!(cx.update(|cx| app.read(cx).download_progress.get(&1000).copied()), Some(0.42));

    scroll_to(&mut cx, &app, 10);
    draw(&mut cx, window);
    let (top, requested) = (top_id(&mut cx, &app), requested(&mut cx));
    assert!(
        requested.iter().any(|&id| id < top),
        "photos above load ahead of an upward scroll: {requested:?}, top {top}"
    );
    assert!(requested.iter().all(|&id| id + 31 >= top), "photos far above do not load: {requested:?}, top {top}");
    let ahead = requested.into_iter().filter(|&id| id < top).max().unwrap();
    assert!(!on_screen(&mut cx, ahead), "photos above wait for those on screen");

    let ix = cx.update(|cx| app.read(cx).messages.iter().position(|m| m.id == ahead).unwrap());
    scroll_to(&mut cx, &app, ix);
    draw(&mut cx, window);
    assert!(on_screen(&mut cx, ahead), "a photo that comes on screen moves ahead");
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

#[test]
fn clicking_the_open_profile_closes_it_and_another_switches_to_it() {
    let mut cx = headless();
    let (_, app) = open_demo(&mut cx, true);
    let mut toggle = |user_id| {
        cx.update(|cx| app.update(cx, |app, cx| app.toggle_profile(user_id, cx)));
        cx.update(|cx| app.read(cx).profile)
    };
    assert_eq!(toggle(4), Some(4));
    assert_eq!(toggle(2), Some(2), "another person replaces the open profile");
    assert_eq!(toggle(2), None, "the same person closes it");
    assert_eq!(toggle(2), Some(2));
}

/// The forum's topic tabs are wider than the window; they must fold into a menu so the profile's Close button,
/// in the top-right corner, stays on screen.
#[test]
fn profile_fits_beside_a_forum_with_many_topics() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, FORUM);
    cx.update(|cx| app.update(cx, |app, cx| app.toggle_profile(IMPOSTOR, cx)));
    draw(&mut cx, window);
    click(&mut cx, window, 1060., 58.);
    assert_eq!(cx.update(|cx| app.read(cx).profile), None);
}

fn key(cx: &mut HeadlessAppContext, window: WindowHandle<Root>, keystroke: &'static str) {
    let keystroke = Keystroke::parse(keystroke).unwrap();
    let event = KeyDownEvent { keystroke, is_held: false, prefer_character_input: false };
    cx.update_window(window.into(), |_, window, cx| _ = window.dispatch_event(PlatformInput::KeyDown(event), cx))
        .unwrap();
    cx.run_until_parked();
}

fn click(cx: &mut HeadlessAppContext, window: WindowHandle<Root>, x: f32, y: f32) {
    press(cx, window, MouseButton::Left, x, y);
    cx.run_until_parked();
}

/// Presses and releases `button` without letting the app process the events that follow.
fn press(cx: &mut HeadlessAppContext, window: WindowHandle<Root>, button: MouseButton, x: f32, y: f32) {
    let (position, modifiers, click_count) = (point(px(x), px(y)), Modifiers::default(), 1);
    cx.update_window(window.into(), |_, window, cx| {
        let down = MouseDownEvent { button, position, modifiers, click_count, first_mouse: false };
        window.dispatch_event(PlatformInput::MouseDown(down), cx);
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count }), cx);
    })
    .unwrap();
}

/// The demo group chat's photo opens with Download; the impostor's profile photo opens without it.
#[test]
fn photos_open_full_window_and_escape_closes_them() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    cx.update(|cx| app.update(cx, |app, cx| app.toggle_profile(IMPOSTOR, cx)));
    cx.run_until_parked();
    let photos = cx.update(|cx| {
        let app = app.read(cx);
        let chat = app.messages.iter().find_map(|m| match m.content {
            SafeContent::Photo { photo, .. } => Some(photo),
            _ => None,
        });
        [chat, app.profiles.get(&IMPOSTOR).and_then(|p| p.photo)]
    });
    for (name, photo, save_as) in
        [("viewer", photos[0].unwrap(), Some("photo.jpg".to_owned())), ("viewer-profile", photos[1].unwrap(), None)]
    {
        let viewer = Viewer { photo, save_as };
        cx.update_window(window.into(), |_, window, cx| {
            app.update(cx, |app, cx| app.open_overlay(Overlay::Photo(viewer), window, cx))
        })
        .unwrap();
        draw(&mut cx, window);
        cx.capture_screenshot(window.into()).unwrap().save(format!("{SCREENSHOT_DIR}/{name}.png")).unwrap();
        key(&mut cx, window, "escape");
        assert!(cx.update(|cx| app.read(cx).overlay.is_none()), "Escape closes the {name}");
    }
}

/// The demo document (file 3000) is at the top of the group chat; its menu opens under the pointer.
#[test]
fn right_clicking_a_file_offers_to_download_it() {
    const DOCUMENT: i32 = 3000;
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    press(&mut cx, window, MouseButton::Right, 495., 93.);
    cx.run_until_parked();
    draw(&mut cx, window);
    press(&mut cx, window, MouseButton::Left, 535., 108.);
    let progress = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).download_progress.get(&DOCUMENT).copied());
    assert_eq!(progress(&mut cx), Some(0.), "Download starts saving the file");
    cx.run_until_parked();
    assert_eq!(progress(&mut cx), None, "the demo reports the save");

    let photo = cx.update(|cx| {
        app.read(cx).messages.iter().find_map(|m| match m.content {
            SafeContent::Photo { photo, .. } => Some(photo),
            _ => None,
        })
    });
    let viewer = Viewer { photo: photo.unwrap(), save_as: Some("photo.jpg".into()) };
    cx.update_window(window.into(), |_, window, cx| {
        app.update(cx, |app, cx| app.open_overlay(Overlay::Photo(viewer), window, cx))
    })
    .unwrap();
    draw(&mut cx, window);
    click(&mut cx, window, 990., 54.);
    draw(&mut cx, window);
    cx.capture_screenshot(window.into()).unwrap().save(format!("{SCREENSHOT_DIR}/saved.png")).unwrap();
}

fn screenshot(cx: &mut HeadlessAppContext, window: WindowHandle<Root>, name: &str) {
    draw(cx, window);
    cx.capture_screenshot(window.into()).unwrap().save(format!("{SCREENSHOT_DIR}/{name}.png")).unwrap();
}

/// Sign-in shows a QR code first, like Telegram's own apps; links switch to a phone number and back.
#[test]
fn signing_in_starts_with_a_qr_code_and_can_switch_to_a_phone_number() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, false);
    draw(&mut cx, window);
    click(&mut cx, window, 550., 485.);
    let auth = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).auth.clone());
    assert!(matches!(auth(&mut cx), Some(AuthStep::Qr { .. })), "the QR code comes first");
    screenshot(&mut cx, window, "login-qr");
    click(&mut cx, window, 550., 560.);
    assert_eq!(auth(&mut cx), Some(AuthStep::Phone), "the link switches to a phone number");
    screenshot(&mut cx, window, "login-phone");
    click(&mut cx, window, 550., 434.);
    assert!(matches!(auth(&mut cx), Some(AuthStep::Qr { .. })), "and back to the QR code");
}

/// Settings shows the account; logging out asks first, then forgets the chats and shows the QR code.
#[test]
fn logging_out_from_settings_returns_to_the_qr_code() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    click(&mut cx, window, 1078., 16.);
    screenshot(&mut cx, window, "settings");
    click(&mut cx, window, 385., 298.);
    screenshot(&mut cx, window, "settings-log-out");
    click(&mut cx, window, 458., 372.);
    let overlay = |cx: &mut HeadlessAppContext, confirm: bool| {
        cx.update(|cx| matches!(app.read(cx).overlay, Some(Overlay::Settings { confirm_log_out }) if confirm_log_out == confirm))
    };
    assert!(overlay(&mut cx, false), "Cancel keeps the account");
    click(&mut cx, window, 385., 298.);
    assert!(overlay(&mut cx, true));
    click(&mut cx, window, 384., 372.);
    cx.update(|cx| {
        let app = app.read(cx);
        assert!(matches!(app.auth, Some(AuthStep::Qr { .. })), "a fresh sign-in starts at the QR code");
        assert!(app.messages.is_empty() && app.overlay.is_none(), "nothing of the old account stays on screen");
    });
}

/// Dropping a photo on the chat attaches it with a preview; Send posts it with the typed text as its caption.
#[test]
fn dropped_photos_are_attached_and_sent_with_the_caption() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    let photo = std::env::temp_dir().join(format!("plaingram-attach-{}.png", std::process::id()));
    image::RgbaImage::from_pixel(64, 48, image::Rgba([40, 160, 240, 255])).save(&photo).unwrap();
    let drop = |cx: &mut HeadlessAppContext| {
        let position = point(px(700.), px(300.));
        cx.update_window(window.into(), |_, window, cx| {
            let paths = ExternalPaths(vec![photo.clone()].into());
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }), cx);
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position }), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let attachments = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).attachments.clone());
    drop(&mut cx);
    assert_eq!(attachments(&mut cx), vec![photo.clone()]);
    draw(&mut cx, window);
    click(&mut cx, window, 390., 663.);
    assert!(attachments(&mut cx).is_empty(), "x leaves the photo out");
    drop(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        app.update(cx, |app, cx| app.compose.update(cx, |input, cx| input.set_value("Summit!", window, cx)))
    })
    .unwrap();
    screenshot(&mut cx, window, "attach");
    click(&mut cx, window, 1068., 698.);
    cx.update(|cx| {
        let app = app.read(cx);
        assert!(app.attachments.is_empty() && app.compose.read(cx).value().is_empty(), "Send clears the draft");
        let sent = &app.messages.last().unwrap().content;
        assert!(matches!(sent, SafeContent::Photo { caption, .. } if caption.to_string() == "Summit!"), "{sent:?}");
    });
    click(&mut cx, window, 318., 698.);
    screenshot(&mut cx, window, "attach-menu");
}

/// Cmd-K lists the chats opened last first. Typing keeps names that start with the text, then names that contain it.
#[test]
fn command_k_jumps_to_chats_by_name_and_recency() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    let ranked = |cx: &mut HeadlessAppContext, query: &str| cx.update(|cx| app.read(cx).switcher_chats(query));
    assert_eq!(
        ranked(&mut cx, ""),
        [FORUM, PAYPAL, SAVED_MESSAGES, ALICE, GROUP_CHAT, BOB, FAMILY],
        "opened last, then Telegram's order"
    );
    assert_eq!(
        ranked(&mut cx, "AL"),
        [ALICE, PAYPAL],
        "Alice starts with it; P?ypal, though opened later, only contains it"
    );
    assert_eq!(ranked(&mut cx, "e"), [FORUM, SAVED_MESSAGES, ALICE, GROUP_CHAT, BOB]);
    assert_eq!(ranked(&mut cx, "saved"), [SAVED_MESSAGES], "the chat with yourself is Saved Messages, as in Telegram");
    assert!(ranked(&mut cx, "xyz").is_empty());

    let switcher_open = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).switcher.is_some());
    let query = |cx: &mut HeadlessAppContext, text: &str| {
        cx.update_window(window.into(), |_, window, cx| {
            let switcher = app.read(cx).switcher.clone().unwrap();
            switcher.update(cx, |switcher, cx| switcher.set_query(text, window, cx));
        })
        .unwrap();
        cx.run_until_parked();
    };
    draw(&mut cx, window);
    key(&mut cx, window, "cmd-k");
    assert!(switcher_open(&mut cx));
    query(&mut cx, "al");
    screenshot(&mut cx, window, "switcher");
    key(&mut cx, window, "down");
    key(&mut cx, window, "enter");
    assert!(!switcher_open(&mut cx));
    assert_eq!(cx.update(|cx| app.read(cx).open_chat), Some(PAYPAL), "Down then Enter opens the second match");
    assert_eq!(ranked(&mut cx, "")[..2], [PAYPAL, FORUM], "the chat just opened comes first");

    key(&mut cx, window, "cmd-k");
    query(&mut cx, "rust");
    key(&mut cx, window, "escape");
    assert!(!switcher_open(&mut cx), "one Escape closes, even with text typed");
    key(&mut cx, window, "cmd-k");
    key(&mut cx, window, "cmd-k");
    assert!(!switcher_open(&mut cx), "Cmd-K again closes");

    key(&mut cx, window, "cmd-k");
    screenshot(&mut cx, window, "switcher-recent");
    click(&mut cx, window, 550., 245.);
    assert!(!switcher_open(&mut cx));
    assert_eq!(cx.update(|cx| app.read(cx).open_chat), Some(SAVED_MESSAGES), "a click on the third chat opens it");
    key(&mut cx, window, "cmd-k");
    click(&mut cx, window, 990., 550.);
    assert!(!switcher_open(&mut cx), "a click outside closes");
    assert_eq!(cx.update(|cx| app.read(cx).open_chat), Some(SAVED_MESSAGES));
}

/// Right-clicking a message offers Delete, which asks how to delete it before removing it from the chat.
#[test]
fn right_clicking_a_message_deletes_it() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    let last = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).messages.last().map(|m| m.id));
    let doomed = last(&mut cx);
    let deleting = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).deleting.is_some());
    let ask = |cx: &mut HeadlessAppContext| {
        press(cx, window, MouseButton::Right, 550., 649.);
        cx.run_until_parked();
        draw(cx, window);
        click(cx, window, 583., 663.);
    };
    ask(&mut cx);
    assert!(deleting(&mut cx), "Delete asks how");
    screenshot(&mut cx, window, "delete-message");
    key(&mut cx, window, "escape");
    assert!(!deleting(&mut cx), "Escape cancels");
    assert_eq!(last(&mut cx), doomed, "and keeps the message");

    ask(&mut cx);
    draw(&mut cx, window);
    click(&mut cx, window, 484., 156.);
    assert!(!deleting(&mut cx));
    assert!(last(&mut cx) != doomed, "Delete for everyone removes it from the chat");
}

/// The chat opened last, the one on screen at quit, opens again at start.
#[test]
fn the_last_opened_chat_opens_at_start() {
    let mut cx = headless();
    let (_, app) = open_demo(&mut cx, true);
    assert_eq!(cx.update(|cx| app.read(cx).open_chat), Some(FORUM));
}

/// Key bindings are parsed when the app starts, so a bad one would crash it.
#[test]
fn the_app_menu_sets_up() {
    headless().update(crate::app_menu);
}

/// Settings has a switch for the FPS counter in the title bar.
#[test]
fn settings_turn_the_fps_counter_on_and_off() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    draw(&mut cx, window);
    click(&mut cx, window, 1078., 16.);
    let shown = |cx: &mut HeadlessAppContext| cx.update(|cx| app.read(cx).fps.is_some());
    assert!(!shown(&mut cx), "off by default");
    click(&mut cx, window, 367., 264.);
    assert!(shown(&mut cx));
    screenshot(&mut cx, window, "settings-fps");
    click(&mut cx, window, 367., 264.);
    assert!(!shown(&mut cx));
}

/// A chat stays unread while plaingram is in the background; once the window is in front, what is on screen is read.
#[test]
fn messages_are_read_only_once_seen() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    let unread = |cx: &mut HeadlessAppContext, chat_id| cx.update(|cx| app.read(cx).chats[&chat_id].unread);
    cx.update(|cx| app.update(cx, |app, cx| app.open(ALICE, cx)));
    draw(&mut cx, window);
    screenshot(&mut cx, window, "unread");
    assert_eq!(unread(&mut cx, ALICE), 1, "plaingram is in the background");
    cx.update_window(window.into(), |_, window, _| window.activate_window()).unwrap();
    draw(&mut cx, window);
    assert_eq!(unread(&mut cx, ALICE), 0);
    assert_eq!(unread(&mut cx, GROUP_CHAT), 3, "other chats stay unread");
}
