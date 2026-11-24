use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, ExternalPaths, FileDropEvent, FontFeatures, HeadlessAppContext,
    Image, ImageFormat, IntoElement, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    ParentElement as _, PlatformInput, Render, Styled as _, Window, WindowHandle, black, div, point, px, size, white,
};
use tg_core::{AuthStep, Demo, Event, SafeContent, SafeReaction, Sender, sanitize};

use crate::app::{Attachment, Confirm, Overlay, Plaingram, Viewer};
use crate::theme::{Palette, THEMES};

const SCREENSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/screenshots");
const SETTINGS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/test-settings");
const GROUP_CHAT: i64 = 11;
const FORUM: i64 = 15;
const ANNOUNCEMENTS_TOPIC: usize = 1;
const GROUPS_FOLDER: usize = 2;
const IMPOSTOR: i64 = 3;
const ALICE: i64 = 10;
const BOB: i64 = 12;
const BOB_JONES: i64 = 4;
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
    cx.update(gpui_kit::init);
    cx
}

fn open_demo(cx: &mut HeadlessAppContext, signed_in: bool) -> (WindowHandle<Root>, Entity<Plaingram>) {
    open_app(cx, signed_in, settings_file())
}

fn open_app(
    cx: &mut HeadlessAppContext,
    signed_in: bool,
    settings: PathBuf,
) -> (WindowHandle<Root>, Entity<Plaingram>) {
    let (demo, events) = Demo::start(signed_in);
    let mut app = None;
    let window = cx
        .open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Plaingram::new(Rc::new(demo), events, settings, window, cx));
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

/// Demo user 6 copies the name of the contact Bob Jones (4); the impostor (3) copies the handle of Alice Smith (2);
/// user 7 copies this account's name, and @trai1headgear (8) the username of a public channel only Telegram knows.
#[test]
fn profiles_warn_about_look_alikes_of_anyone() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    let lookalikes = |cx: &mut HeadlessAppContext, user_id| cx.update(|cx| app.read(cx).lookalikes(user_id));
    assert_eq!(lookalikes(&mut cx, 6), ["looks like your contact Bob Jones (@bobj)"]);
    assert_eq!(lookalikes(&mut cx, IMPOSTOR), ["looks like your contact Alice Smith (@alice_smith)"]);
    assert_eq!(lookalikes(&mut cx, 7), ["looks like you (@samrivera)"]);
    assert_eq!(lookalikes(&mut cx, 8), [] as [&str; 0], "Telegram is asked as the profile opens");
    cx.update(|cx| app.update(cx, |app, cx| app.toggle_profile(8, cx)));
    draw(&mut cx, window);
    assert_eq!(lookalikes(&mut cx, 8), ["looks like Trailhead Gear (@trailheadgear), another account"]);
    screenshot(&mut cx, window, "profile-lookalike");
    assert_eq!(lookalikes(&mut cx, 5), [] as [&str; 0]);
    assert_eq!(lookalikes(&mut cx, 4), [] as [&str; 0], "contacts are not flagged");
    assert_eq!(lookalikes(&mut cx, 1), [] as [&str; 0], "nor is this account");
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

fn type_in_switcher(cx: &mut HeadlessAppContext, window: WindowHandle<Root>, app: &Entity<Plaingram>, text: &str) {
    cx.update_window(window.into(), |_, window, cx| {
        let switcher = app.read(cx).switcher.clone().unwrap();
        switcher.update(cx, |switcher, cx| switcher.set_query(text, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
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
    scroll_to(&mut cx, &app, 5);
    draw(&mut cx, window);
    press(&mut cx, window, MouseButton::Right, 495., 117.);
    cx.run_until_parked();
    draw(&mut cx, window);
    press(&mut cx, window, MouseButton::Left, 535., 132.);
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
    click(&mut cx, window, 385., 566.);
    screenshot(&mut cx, window, "settings-log-out");
    click(&mut cx, window, 458., 640.);
    let overlay = |cx: &mut HeadlessAppContext, confirm: bool| {
        cx.update(|cx| matches!(app.read(cx).overlay, Some(Overlay::Settings { confirm_log_out }) if confirm_log_out == confirm))
    };
    assert!(overlay(&mut cx, false), "Cancel keeps the account");
    click(&mut cx, window, 385., 566.);
    assert!(overlay(&mut cx, true));
    click(&mut cx, window, 384., 640.);
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
    assert_eq!(attachments(&mut cx), vec![Attachment::File(photo.clone())]);
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

/// Cmd-V in the message box attaches a copied image, such as a screenshot, with a preview; text still pastes as text.
#[test]
fn pasted_images_are_attached_and_text_pastes_as_text() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    click(&mut cx, window, 700., 698.);
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 48, image::Rgba([240, 120, 40, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let image = Image::from_bytes(ImageFormat::Png, png);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_image(&image)));
    key(&mut cx, window, "cmd-v");
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("Summit!".into())));
    key(&mut cx, window, "cmd-v");
    cx.update(|cx| {
        let app = app.read(cx);
        assert_eq!(app.attachments, [Attachment::Pasted(Arc::new(image))]);
        assert_eq!(app.compose.read(cx).value(), "Summit!");
    });
    screenshot(&mut cx, window, "paste");
    click(&mut cx, window, 1068., 698.);
    let sent = cx.update(|cx| app.read(cx).messages.last().unwrap().content.clone());
    assert!(matches!(&sent, SafeContent::Photo { caption, .. } if caption.to_string() == "Summit!"), "{sent:?}");
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
    draw(&mut cx, window);
    key(&mut cx, window, "cmd-k");
    assert!(switcher_open(&mut cx));
    type_in_switcher(&mut cx, window, &app, "al");
    screenshot(&mut cx, window, "switcher");
    key(&mut cx, window, "down");
    key(&mut cx, window, "enter");
    assert!(!switcher_open(&mut cx));
    assert_eq!(cx.update(|cx| app.read(cx).open_chat), Some(PAYPAL), "Down then Enter opens the second match");
    assert_eq!(ranked(&mut cx, "")[..2], [PAYPAL, FORUM], "the chat just opened comes first");

    key(&mut cx, window, "cmd-k");
    type_in_switcher(&mut cx, window, &app, "rust");
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

/// Typing in the switcher also finds people to start a chat with: contacts first, then, from four characters on,
/// strangers by public username, marked as not a contact. Picking one opens a new chat with them. The search button in the
/// title bar opens the switcher too.
#[test]
fn the_switcher_finds_people_to_start_a_chat_with() {
    const BOB_CONTACT: i64 = 4;
    const BOB_STRANGER: i64 = 6;
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    let ranked = |cx: &mut HeadlessAppContext, query: &str| cx.update(|cx| app.read(cx).switcher_chats(query));
    draw(&mut cx, window);
    click(&mut cx, window, 1040., 16.);
    type_in_switcher(&mut cx, window, &app, "bob");
    draw(&mut cx, window);
    assert_eq!(ranked(&mut cx, "bob"), [BOB, BOB_CONTACT], "the secret chat with Bob, then a new chat with him");
    type_in_switcher(&mut cx, window, &app, "bob j");
    draw(&mut cx, window);
    assert_eq!(ranked(&mut cx, "bob j"), [BOB, BOB_CONTACT, BOB_STRANGER], "four characters also find strangers");
    screenshot(&mut cx, window, "switcher-people");
    key(&mut cx, window, "down");
    key(&mut cx, window, "down");
    key(&mut cx, window, "enter");
    assert_eq!(cx.update(|cx| app.read(cx).open_chat), Some(BOB_STRANGER));
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
    let deleting =
        |cx: &mut HeadlessAppContext| cx.update(|cx| matches!(app.read(cx).confirming, Some(Confirm::Delete { .. })));
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
    click(&mut cx, window, 367., 531.);
    assert!(shown(&mut cx));
    screenshot(&mut cx, window, "settings-fps");
    click(&mut cx, window, 367., 531.);
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

/// After logging out, a sign-in that completes while the QR code shows, as when the phone scans it, opens the chats.
#[test]
fn signing_in_again_after_logging_out_opens_the_chats() {
    let mut cx = headless();
    let (_, app) = open_demo(&mut cx, true);
    cx.update(|cx| app.read(cx).client.log_out());
    cx.run_until_parked();
    assert!(matches!(cx.update(|cx| app.read(cx).auth.clone()), Some(AuthStep::Qr { .. })));
    cx.update(|cx| app.read(cx).client.submit_password(String::new()));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| app.read(cx).auth.clone()), Some(AuthStep::Ready));
}

/// The demo account moderates the hiking group: right-clicking another member's message also offers to delete all
/// their messages and ban them, after asking.
#[test]
fn moderators_can_delete_all_of_a_members_messages_and_ban_them() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    let from_bob = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| app.read(cx).messages.iter().filter(|m| m.sender == Sender::User(BOB_JONES)).count())
    };
    let confirming =
        |cx: &mut HeadlessAppContext| cx.update(|cx| matches!(app.read(cx).confirming, Some(Confirm::Ban { .. })));
    let ask = |cx: &mut HeadlessAppContext| {
        press(cx, window, MouseButton::Right, 550., 649.);
        cx.run_until_parked();
        draw(cx, window);
        click(cx, window, 600., 687.);
    };
    ask(&mut cx);
    assert!(confirming(&mut cx), "Delete all and ban asks first");
    screenshot(&mut cx, window, "ban-member");
    key(&mut cx, window, "escape");
    assert!(!confirming(&mut cx) && from_bob(&mut cx) > 0, "Escape cancels");

    ask(&mut cx);
    draw(&mut cx, window);
    click(&mut cx, window, 608., 178.);
    assert_eq!(from_bob(&mut cx), 0, "confirming deletes all their messages");
}

/// The menu beside a group's name leaves it, after asking.
#[test]
fn the_group_menu_leaves_the_group() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, FAMILY);
    draw(&mut cx, window);
    click(&mut cx, window, 1075., 58.);
    cx.run_until_parked();
    draw(&mut cx, window);
    screenshot(&mut cx, window, "group-menu");
    click(&mut cx, window, 1028., 91.);
    assert!(cx.update(|cx| matches!(app.read(cx).confirming, Some(Confirm::Leave { .. }))), "Leave group asks first");
    screenshot(&mut cx, window, "leave-group");
    click(&mut cx, window, 634., 156.);
    cx.run_until_parked();
    cx.update(|cx| {
        let app = app.read(cx);
        assert_eq!(app.open_chat, None, "confirming closes the group");
        assert!(!app.switcher_chats("").contains(&FAMILY), "and drops it from every list");
    });
    screenshot(&mut cx, window, "left-group");
}

/// Reactions show under a message with their counts, filled where this account reacted, and follow Telegram's
/// updates for the open chat only, as message ids repeat across chats.
#[test]
fn reactions_show_under_messages_and_follow_updates() {
    const WHOS_IN: i64 = 2;
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    scroll_to(&mut cx, &app, 4);
    cx.run_until_parked();
    screenshot(&mut cx, window, "reactions");
    let update = |cx: &mut HeadlessAppContext, chat_id, count| {
        let reactions = vec![SafeReaction { emoji: sanitize("\u{1F44D}"), count, chosen: true, id: None }];
        let event = Event::Reactions { chat_id, message_id: WHOS_IN, reactions };
        cx.update_window(window.into(), |_, window, cx| app.update(cx, |app, cx| app.apply(event, window, cx)))
            .unwrap();
    };
    let counts = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| {
            let message = app.read(cx).messages.iter().find(|m| m.id == WHOS_IN).unwrap();
            message.reactions.iter().map(|r| r.count).collect::<Vec<_>>()
        })
    };
    assert_eq!(counts(&mut cx), [3, 2]);
    update(&mut cx, FORUM, 9);
    assert_eq!(counts(&mut cx), [3, 2], "another chat's message with the same id is not this one");
    update(&mut cx, GROUP_CHAT, 4);
    assert_eq!(counts(&mut cx), [4]);
}

/// A settings file of its own for each app a test opens, none there yet.
fn settings_file() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = PathBuf::from(SETTINGS_DIR).join(format!(
        "{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    _ = std::fs::remove_file(&path);
    path
}

/// The theme and CRT scanlines are picked in Settings, apply at once, and come back the next time plaingram opens.
#[test]
fn themes_are_picked_in_settings_and_kept_between_runs() {
    const ORANGE_GLOW: (f32, f32) = (357., 343.);
    const SCANLINES: (f32, f32) = (367., 501.);
    let mut cx = headless();
    let settings = settings_file();
    let (window, app) = open_app(&mut cx, true, settings.clone());
    draw(&mut cx, window);
    click(&mut cx, window, 1078., 16.);
    click(&mut cx, window, ORANGE_GLOW.0, ORANGE_GLOW.1);
    click(&mut cx, window, SCANLINES.0, SCANLINES.1);
    let picked = |cx: &mut HeadlessAppContext, app: &Entity<Plaingram>| {
        cx.update(|cx| (app.read(cx).theme.name, app.read(cx).scanlines, cx.theme().foreground.to_rgb().g))
    };
    let amber = 0xb0 as f32 / 255.;
    let (name, scanlines, green) = picked(&mut cx, &app);
    assert!((name, scanlines) == ("Orange glow", true) && (green - amber).abs() < 0.01, "applies at once");
    screenshot(&mut cx, window, "settings-orange-glow");

    let (_, reopened) = open_app(&mut cx, true, settings);
    let (name, scanlines, _) = picked(&mut cx, &reopened);
    assert_eq!((name, scanlines), ("Orange glow", true), "comes back");
}

/// Every theme on the hiking group, with marked characters, warnings, reactions and both kinds of bubble.
#[test]
fn every_theme_draws_the_chat() {
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    for theme in &THEMES {
        cx.update(|cx| app.update(cx, |app, cx| app.set_theme(theme, cx)));
        draw(&mut cx, window);
        scroll_to(&mut cx, &app, 5);
        cx.run_until_parked();
        screenshot(&mut cx, window, &format!("theme-{}", theme.name.to_lowercase().replace(' ', "-")));
    }
}

/// San Francisco's high legibility set reaches the text: with it, `Il1 0O` draws differently.
#[test]
fn cli_proportional_draws_high_legibility_letters() {
    struct Sample(FontFeatures);
    impl Render for Sample {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .bg(white())
                .text_color(black())
                .text_size(px(40.))
                .font_family(".SystemUIFont")
                .font_features(self.0.clone())
                .child("Il1 0O")
        }
    }
    let mut cx = headless();
    let mut pixels = |features: FontFeatures| {
        let window = cx.open_window(size(px(300.), px(80.)), |_, cx| cx.new(|_| Sample(features))).unwrap();
        cx.update_window(window.into(), |_, window, cx| _ = window.draw(cx)).unwrap();
        cx.capture_screenshot(window.into()).unwrap().into_raw()
    };
    let plain = pixels(THEMES[0].font_features());
    assert_ne!(plain, pixels(Palette::named("CLI proportional").unwrap().font_features()));
}

/// Clicking an emoji reaction under a message reacts with it or takes the reaction back, while custom emoji and paid
/// ones do nothing; React… in the message's menu offers the emoji Telegram allows, where the message was right-clicked
/// but inside the window. Like Telegram without Premium, the demo keeps one reaction per person.
#[test]
fn reacting_from_the_chips_or_the_menu() {
    const TRAIL_MAP: i64 = 5;
    const TRAIL_FILE: i64 = 6;
    const PAY_HERE: i64 = 8;
    const HEART: (f32, f32) = (352., 394.);
    const CUSTOM_EMOJI: (f32, f32) = (470., 394.);
    const CAPTION: (f32, f32) = (448., 366.);
    let mut cx = headless();
    let (window, app) = open_demo(&mut cx, true);
    open_chat(&mut cx, &app, GROUP_CHAT);
    draw(&mut cx, window);
    scroll_to(&mut cx, &app, 4);
    cx.run_until_parked();
    draw(&mut cx, window);
    let shown = |cx: &mut HeadlessAppContext, id| {
        cx.update(|cx| {
            let message = app.read(cx).messages.iter().find(|m| m.id == id).unwrap();
            message.reactions.iter().map(|r| (r.emoji.to_string(), r.count, r.chosen)).collect::<Vec<_>>()
        })
    };
    let reaction = |emoji: &str, count, chosen| (emoji.to_string(), count, chosen);
    let (custom, paid) = (reaction("custom emoji", 1, false), reaction("\u{2B50}", 1, false));
    click(&mut cx, window, HEART.0, HEART.1);
    cx.run_until_parked();
    let heart = "\u{2764}\u{FE0F}";
    assert_eq!(
        shown(&mut cx, TRAIL_MAP),
        [reaction(heart, 5, true), reaction("\u{1F525}", 1, false), custom.clone(), paid.clone()],
        "the heart replaces the fire"
    );
    draw(&mut cx, window);
    click(&mut cx, window, HEART.0, HEART.1);
    cx.run_until_parked();
    let before = [reaction(heart, 4, false), reaction("\u{1F525}", 1, false), custom, paid];
    assert_eq!(shown(&mut cx, TRAIL_MAP), before, "clicking it again takes it back");
    draw(&mut cx, window);
    click(&mut cx, window, CUSTOM_EMOJI.0, CUSTOM_EMOJI.1);
    cx.run_until_parked();
    assert_eq!(shown(&mut cx, TRAIL_MAP), before, "custom emoji are never sent");

    let pick = |cx: &mut HeadlessAppContext, message: (f32, f32), react: (f32, f32), emoji: (f32, f32), name| {
        draw(cx, window);
        press(cx, window, MouseButton::Right, message.0, message.1);
        cx.run_until_parked();
        draw(cx, window);
        click(cx, window, react.0, react.1);
        cx.run_until_parked();
        screenshot(cx, window, name);
        click(cx, window, emoji.0, emoji.1);
        cx.run_until_parked();
    };
    let thumbs_up = [reaction("\u{1F44D}", 1, true)];
    pick(&mut cx, CAPTION, (480., 404.), (505., 427.), "react-picker");
    assert!(cx.update(|cx| app.read(cx).confirming.is_none()), "picking closes the dialog");
    assert_eq!(shown(&mut cx, TRAIL_MAP).last(), thumbs_up.last(), "and reacts");
    pick(&mut cx, (440., 657.), (470., 648.), (497., 682.), "react-picker-bottom");
    assert_eq!(shown(&mut cx, PAY_HERE), thumbs_up, "near the bottom the dialog moves up into the window");
    pick(&mut cx, (700., 480.), (732., 518.), (709., 541.), "react-picker-right");
    assert_eq!(shown(&mut cx, TRAIL_FILE), thumbs_up, "and near the right edge, left");
}
