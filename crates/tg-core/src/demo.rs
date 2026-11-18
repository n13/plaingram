use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::Events;
use crate::image::DecodedImage;
use crate::model::{
    AuthStep, Backend, ChatKind, ChatListId, DeleteOptions, Event, PhotoRef, ReactionId, SafeAccountInfo, SafeChat,
    SafeContent, SafeFile, SafeFolder, SafeMessage, SafeProfile, SafeReaction, SafeReply, SafeTopic, SafeUser, Sender,
};
use crate::policy;
use crate::text::{is_allowed, sanitize, sanitize_name};

const ME: i64 = 1;
/// "Alice Smith" with a Cyrillic "А", posing as the contact Alice Smith (user 2).
const IMPOSTOR: i64 = 3;
const DAY: i32 = 24 * 3600;
/// Profile photos have this plus the user id as file id.
const PROFILE_PHOTOS: i32 = 2000;
const TRAIL_FILE: i32 = 3000;
const HIKING: i64 = 11;
const FORUM: i64 = 15;
const PAYPAL: i64 = 13;
const SAVED_MESSAGES: i64 = 16;
const DEMO_SPACING: i32 = 5 * 3600;
/// How long a query must be before Telegram also searches public usernames.
const PUBLIC_SEARCH_LENGTH: usize = 4;
const GENERAL_TOPIC: i32 = 1;
/// Messages in the forum's General topic, enough to scroll back through.
const GENERAL_SIZE: i64 = 1000;
/// A General photo whose download stays at [`SLOW_PHOTO_PROGRESS`], to show the progress bar.
const SLOW_PHOTO: i32 = 1000;
const SLOW_PHOTO_PROGRESS: f32 = 0.42;
const TOPICS: [(i32, &str); 11] = [
    (GENERAL_TOPIC, "General"),
    (2, "Announcements \u{1F4E2}"),
    (3, "Help"),
    (4, "Tech Support"),
    (5, "Mining"),
    (6, "\u{4E2D}\u{6587}"),
    (7, "Community Projects"),
    (8, "\u{0420}\u{0443}\u{0441}\u{0441}\u{043A}\u{0438}\u{0439}"),
    (9, "Speculation"),
    (10, "Off-Topic"),
    (11, "Faucet"),
];
const PHOTO: PhotoRef = PhotoRef { file_id: 1, width: 640, height: 427 };
/// Shaped like the links TDLib sends, so the demo QR code is as dense as a real one.
const QR_LINK: &str = "tg://login?token=AQKm8fNnZrbRgP0x3sGg9lHn2vYq7kHbXzJc1tW4eQdUoLsA";

/// Offline backend with fixed fixtures, for UI development and screenshots. All text goes through [`sanitize`]. The
/// account moderates the hiking group.
pub struct Demo {
    events: async_channel::Sender<Event>,
    /// Reactions changed since the demo started, by chat and message.
    reactions: RefCell<HashMap<(i64, i64), Vec<SafeReaction>>>,
}

impl Demo {
    pub fn start(signed_in: bool) -> (Self, Events) {
        let (events, receiver) = async_channel::unbounded();
        let demo = Self { events, reactions: RefCell::default() };
        if signed_in {
            demo.sign_in()
        } else {
            demo.emit(Event::Auth(AuthStep::ApiCredentials))
        }
        (demo, receiver)
    }

    fn sign_in(&self) {
        self.emit(Event::Auth(AuthStep::Ready));
        self.emit(Event::Account { user_id: ME, phone: sanitize("+15550100") });
        for (id, name, username, is_contact) in PEOPLE {
            self.emit(Event::User(SafeUser {
                id,
                name: sanitize_name(name),
                username: username.map(sanitize_name),
                is_contact,
                is_bot: false,
            }));
        }
        self.emit(Event::AccountInfo {
            user_id: IMPOSTOR,
            info: SafeAccountInfo {
                registered: Some((11, 2026)),
                phone_country: sanitize("NG"),
                name_changed: Some(now() - 2 * DAY),
                photo_changed: Some(now() - DAY),
            },
        });
        self.emit(Event::Folders {
            folders: vec![
                SafeFolder { id: 1, name: sanitize("Friends \u{1F60A}") },
                SafeFolder { id: 2, name: sanitize("Groups") },
            ],
            main_position: 0,
        });
        for (id, kind, title, unread, order, lists) in chats() {
            self.emit(Event::Chat(SafeChat { id, kind, title: sanitize_name(title), unread }));
            for list in lists {
                self.emit(Event::ChatPosition { chat_id: id, list, order });
            }
        }
        self.emit(Event::RecentChats(vec![FORUM, PAYPAL]));
    }

    fn emit(&self, event: Event) {
        self.events.try_send(event).expect("demo event receiver is gone");
    }

    /// Reports a save without touching the disk.
    fn saved(&self, file_id: i32, name: &str) {
        self.emit(Event::Saved { file_id, path: sanitize(&format!("~/Downloads/{name}")) });
    }
}

impl Backend for Demo {
    fn submit_api_credentials(&self, _: String, _: String) {
        self.log_in_with_qr();
    }

    fn log_in_with_qr(&self) {
        self.emit(Event::Auth(AuthStep::Qr { link: QR_LINK.into() }));
    }

    fn log_in_with_phone(&self) {
        self.emit(Event::Auth(AuthStep::Closed));
        self.emit(Event::Auth(AuthStep::Phone));
    }

    fn submit_phone(&self, _: String) {
        self.emit(Event::Auth(AuthStep::Code));
    }

    fn submit_code(&self, _: String) {
        self.emit(Event::Auth(AuthStep::Password { hint: sanitize("demo") }));
    }

    fn submit_password(&self, _: String) {
        self.sign_in();
    }

    fn log_out(&self) {
        self.emit(Event::Auth(AuthStep::Closed));
        self.log_in_with_qr();
    }

    fn open_chat(&self, chat_id: i64, topic: Option<i32>) {
        let mut messages = history(chat_id, topic);
        for message in &mut messages {
            if let Some(reactions) = self.reactions.borrow().get(&(chat_id, message.id)) {
                message.reactions = reactions.clone();
            }
        }
        let reply = (chat_id == HIKING).then(|| {
            let (original, last) = (&messages[3], messages.last().expect("the hiking chat has messages"));
            (last.id, SafeReply { sender: Some(original.sender), content: original.content.clone() })
        });
        self.emit(Event::History { chat_id, topic, messages });
        if let Some((message_id, reply)) = reply {
            self.emit(Event::Reply { chat_id, message_id, reply });
        }
        if chat_id == HIKING {
            self.emit(Event::Moderator { chat_id, can_ban: true });
        }
    }

    fn close_chat(&self, _: i64) {}

    fn load_profile(&self, user_id: i64) {
        let now = now();
        let (bio, photo_dates, groups_in_common, labels) = match user_id {
            2 => ("Trail runner and coffee snob \u{2615}", vec![now - 200 * DAY, now - 1100 * DAY], 3, vec!["Premium"]),
            IMPOSTOR => ("Crypto investor. DM me for deals", vec![now - DAY], 1, vec!["unofficial app"]),
            _ => ("", Vec::new(), 1, Vec::new()),
        };
        let file_id = PROFILE_PHOTOS + i32::try_from(user_id).expect("demo user ids are small");
        self.emit(Event::Profile(SafeProfile {
            user_id,
            bio: sanitize(bio),
            photo: (!photo_dates.is_empty()).then_some(PhotoRef { file_id, ..PHOTO }),
            photo_count: photo_dates.len().try_into().expect("few photos"),
            photo_dates,
            groups_in_common,
            labels,
        }));
    }

    fn load_older(&self, chat_id: i64, topic: Option<i32>, before: i64) {
        let messages = match (chat_id, topic) {
            (FORUM, Some(GENERAL_TOPIC)) => general(before, 100),
            _ => Vec::new(),
        };
        self.emit(Event::OlderHistory { chat_id, topic, before, messages });
    }

    fn load_topics(&self, chat_id: i64) {
        let topics = TOPICS.map(|(id, name)| SafeTopic { id, name: sanitize(name) }).into();
        self.emit(Event::Topics { chat_id, topics });
    }

    /// Reads the whole chat, as the demo's messages are all among the newest.
    fn mark_read(&self, chat_id: i64, _: Vec<i64>) {
        self.emit(Event::ChatUnread { chat_id, count: 0 });
    }

    fn load_delete_options(&self, chat_id: i64, message_id: i64) {
        let options = DeleteOptions { for_me: true, for_everyone: true };
        self.emit(Event::DeleteOptions { chat_id, message_id, options });
    }

    fn delete_message(&self, chat_id: i64, message_id: i64, _: bool) {
        self.emit(Event::MessagesDeleted { chat_id, message_ids: vec![message_id] });
    }

    fn ban_sender(&self, chat_id: i64, user_id: i64) {
        let message_ids =
            history(chat_id, None).iter().filter(|m| m.sender == Sender::User(user_id)).map(|m| m.id).collect();
        self.emit(Event::MessagesDeleted { chat_id, message_ids });
    }

    /// Contacts whose name or username contains the query, then, from four characters on, others with a username, as
    /// Telegram finds people by public username. Someone without a private chat gets one, numbered as they are.
    fn search_people(&self, query: String) {
        let lowercase = query.to_lowercase();
        let public = query.chars().count() >= PUBLIC_SEARCH_LENGTH;
        let mut found: Vec<_> = PEOPLE
            .into_iter()
            .filter(|&(id, name, username, is_contact)| {
                let named =
                    name.to_lowercase().contains(&lowercase) || username.is_some_and(|u| u.contains(&lowercase));
                id != ME && named && (is_contact || public && username.is_some())
            })
            .collect();
        found.sort_by_key(|&(.., is_contact)| !is_contact);
        let chats = found
            .into_iter()
            .map(|(user_id, name, ..)| {
                let kind = ChatKind::Private { user_id };
                chats().into_iter().find(|chat| chat.1 == kind).map_or_else(
                    || {
                        self.emit(Event::Chat(SafeChat { id: user_id, kind, title: sanitize_name(name), unread: 0 }));
                        user_id
                    },
                    |chat| chat.0,
                )
            })
            .collect();
        self.emit(Event::People { query, chats });
    }

    fn load_available_reactions(&self, chat_id: i64, message_id: i64) {
        let reactions = AVAILABLE_REACTIONS.iter().map(|&emoji| (sanitize(emoji), ReactionId(emoji.into()))).collect();
        self.emit(Event::AvailableReactions { chat_id, message_id, reactions });
    }

    /// Like Telegram without Premium, one reaction per message: adding one takes back the one before.
    fn react(&self, chat_id: i64, message_id: i64, reaction: ReactionId, add: bool) {
        let mut changed = self.reactions.borrow_mut();
        let reactions = changed.entry((chat_id, message_id)).or_insert_with(|| reactions(chat_id, message_id));
        for other in reactions.iter_mut().filter(|r| r.chosen && (!add || r.id.as_ref() != Some(&reaction))) {
            other.count -= 1;
            other.chosen = false;
        }
        if add {
            match reactions.iter_mut().find(|r| r.id.as_ref() == Some(&reaction)) {
                Some(picked) if !picked.chosen => (picked.count, picked.chosen) = (picked.count + 1, true),
                Some(_) => {}
                None => reactions.push(SafeReaction {
                    emoji: sanitize(&reaction.0),
                    count: 1,
                    chosen: true,
                    id: Some(reaction),
                }),
            }
        }
        reactions.retain(|r| r.count > 0);
        self.emit(Event::Reactions { chat_id, message_id, reactions: reactions.clone() });
    }

    fn leave_chat(&self, chat_id: i64) {
        for list in [ChatListId::Main, ChatListId::Archive, ChatListId::Folder(1), ChatListId::Folder(2)] {
            self.emit(Event::ChatPosition { chat_id, list, order: 0 });
        }
    }

    fn send_text(&self, chat_id: i64, topic: Option<i32>, text: String) {
        if is_allowed(&text) {
            self.emit(Event::Message(message(chat_id, topic, 1000, ME, SafeContent::Text(sanitize(&text)), now())))
        } else {
            self.emit(Event::Error(sanitize("message contains characters outside the allowlist")))
        }
    }

    /// Shows each file as the demo photo, without reading it.
    fn send_photos(&self, chat_id: i64, topic: Option<i32>, files: Vec<PathBuf>, caption: String) {
        let mut caption = Some(sanitize(&caption));
        for id in (1001..).take(files.len()) {
            let content = SafeContent::Photo { photo: PHOTO, caption: caption.take().unwrap_or_default() };
            self.emit(Event::Message(message(chat_id, topic, id, ME, content, now())));
        }
    }

    fn hurry_photo(&self, _: i32) {}

    fn load_photo(&self, file_id: i32, _: bool) {
        if file_id == SLOW_PHOTO {
            return self.emit(Event::DownloadProgress { file_id, progress: SLOW_PHOTO_PROGRESS });
        }
        let (width, height) = (PHOTO.width as u32, PHOTO.height as u32);
        let bgra = (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| [(160 + x * 95 / width) as u8, (90 + y * 120 / height) as u8, 60, 255])
            })
            .collect();
        self.emit(Event::Photo { file_id, image: DecodedImage { width, height, bgra } });
    }

    fn save_photo(&self, file_id: i32, name: String) {
        self.saved(file_id, &name);
    }

    fn save_file(&self, file_id: i32, name: String) {
        self.saved(file_id, &name);
    }
}

fn history(chat_id: i64, topic: Option<i32>) -> Vec<SafeMessage> {
    if (chat_id, topic) == (FORUM, Some(GENERAL_TOPIC)) {
        return general(GENERAL_SIZE + 1, 50);
    }
    let text = |s: &str| SafeContent::Text(sanitize(s));
    let entries = match chat_id {
        HIKING => vec![
            (2, SafeContent::Service(sanitize("created the group Weekend hiking \u{1F525}"))),
            (2, text("Who's in for Saturday? \u{1F602}")),
            (4, text("Count me in \u{1F44D}")),
            (3, text("Hi all, Alice here, new number. Please send the cabin deposit to me today")),
            (4, SafeContent::Photo { photo: PHOTO, caption: sanitize("Trail map for the north loop") }),
            (
                2,
                SafeContent::File {
                    kind: sanitize("Document"),
                    file: SafeFile { file_id: TRAIL_FILE, name: sanitize("north-loop.gpx.zip"), size: 2_400_000 },
                },
            ),
            (2, SafeContent::Hidden(sanitize("Sticker"))),
            (3, text("Pay here: paypa\u{0399}.com/cabin")),
            (ME, text("I'm in! \u{2764}\u{FE0F}")),
            (4, text("Packing list:\r\n-\tboots\r\n-\twater")),
            (4, text("Wait, that second Alice has a weird name \u{1F914}")),
        ],
        FORUM if topic == Some(2) => vec![(4, text("Release 2.0 is out: Москва and 東京 meetups next week \u{1F680}"))],
        FORUM => vec![(2, text("Welcome! Ask questions in Help.")), (4, text("Thanks \u{1F91D}"))],
        _ => vec![(ME, text("Hey!")), (2, text("Hi \u{1F60A}"))],
    };
    let (now, count) = (now(), entries.len() as i32);
    entries
        .into_iter()
        .zip(1..)
        .map(|((from, content), id)| {
            message(chat_id, topic, id.into(), from, content, now - 120 - (count - id) * DEMO_SPACING)
        })
        .collect()
}

/// Up to `count` messages of the General topic before message `before`; message `id` is `GENERAL_SIZE - id` ten-minute steps old, and every tenth is a photo with file id `id`.
fn general(before: i64, count: i64) -> Vec<SafeMessage> {
    let now = now();
    ((before - count).max(1)..before)
        .map(|id| {
            let date = now - i32::try_from(GENERAL_SIZE - id).expect("General is small") * 600;
            let text = sanitize(&format!("General message {id}"));
            let content = match id % 10 {
                0 => SafeContent::Photo {
                    photo: PhotoRef { file_id: id.try_into().expect("General is small"), ..PHOTO },
                    caption: text,
                },
                _ => SafeContent::Text(text),
            };
            message(FORUM, Some(GENERAL_TOPIC), id, if id % 2 == 0 { 2 } else { 4 }, content, date)
        })
        .collect()
}

fn message(chat_id: i64, topic: Option<i32>, id: i64, from: i64, content: SafeContent, date: i32) -> SafeMessage {
    let reactions = reactions(chat_id, id);
    SafeMessage { id, chat_id, sender: Sender::User(from), date, outgoing: from == ME, topic, content, reactions }
}

/// Everyone the demo knows: id, name, @username and whether they are a contact.
const PEOPLE: [(i64, &str, Option<&str>, bool); 6] = [
    (ME, "Sam Rivera", Some("samrivera"), false),
    (2, "Alice Smith", Some("alice_smith"), true),
    (IMPOSTOR, "\u{0410}lice Smith", Some("alice_smith_"), false),
    (4, "Bob Jones", Some("bobj"), true),
    (6, "Bob Jones", Some("bobjones_support"), false),
    (5, "P\u{0430}ypal Support", None, false),
];

/// A demo chat: id, kind, title, unread messages, order and the lists it is in.
type DemoChat = (i64, ChatKind, &'static str, i32, i64, Vec<ChatListId>);

fn chats() -> [DemoChat; 7] {
    let (main, friends, groups) = (ChatListId::Main, ChatListId::Folder(1), ChatListId::Folder(2));
    [
        (SAVED_MESSAGES, ChatKind::Private { user_id: ME }, "Sam Rivera", 0, 7, vec![main]),
        (10, ChatKind::Private { user_id: 2 }, "Alice Smith", 1, 6, vec![main, friends]),
        (HIKING, ChatKind::Group, "Weekend hiking \u{1F525}", 3, 5, vec![main, friends, groups]),
        (12, ChatKind::Secret { user_id: 4 }, "Bob Jones", 0, 4, vec![main, friends]),
        (PAYPAL, ChatKind::Private { user_id: 5 }, "P\u{0430}ypal Support", 1, 3, vec![ChatListId::Archive]),
        (14, ChatKind::Group, "Family \u{2764}\u{FE0F}", 0, 2, vec![main, groups]),
        (FORUM, ChatKind::Forum, "Rust devs", 128, 1, vec![main, groups]),
    ]
}

/// What every demo message may be reacted with.
const AVAILABLE_REACTIONS: [&str; 8] =
    ["\u{1F44D}", "\u{2764}", "\u{1F525}", "\u{1F602}", "\u{1F62E}", "\u{1F622}", "\u{1F64F}", "\u{1F389}"];

/// Reactions on a few hiking messages: counts, one this account chose, a custom emoji and a paid one.
fn reactions(chat_id: i64, id: i64) -> Vec<SafeReaction> {
    let reaction = |emoji: &str, count, chosen| {
        let id = (emoji != policy::CUSTOM_EMOJI && emoji != policy::PAID).then(|| ReactionId(emoji.into()));
        SafeReaction { emoji: sanitize(emoji), count, chosen, id }
    };
    match (chat_id, id) {
        (HIKING, 2) => vec![reaction("\u{1F44D}", 3, true), reaction("\u{1F602}", 2, false)],
        (HIKING, 5) => vec![
            reaction("\u{2764}", 4, false),
            reaction("\u{1F525}", 2, true),
            reaction(policy::CUSTOM_EMOJI, 1, false),
            reaction(policy::PAID, 1, false),
        ],
        (HIKING, 9) => vec![reaction("\u{1F389}", 2, false)],
        _ => Vec::new(),
    }
}

fn now() -> i32 {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock is after 1970").as_secs();
    i32::try_from(secs).expect("clock is before 2038")
}
