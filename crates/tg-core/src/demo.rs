use crate::client::Events;
use crate::image::DecodedImage;
use crate::model::{
    AuthStep, Backend, ChatKind, ChatListId, Event, PhotoRef, SafeChat, SafeContent, SafeFolder, SafeMessage,
    SafeReply, SafeTopic, SafeUser, Sender,
};
use crate::text::{is_allowed, sanitize};

const ME: i64 = 1;
const HIKING: i64 = 11;
const FORUM: i64 = 15;
const TOPICS: [(i32, &str); 3] = [(1, "General"), (2, "Announcements \u{1F4E2}"), (3, "Help")];
const PHOTO: PhotoRef = PhotoRef { file_id: 1, width: 640, height: 427 };

/// Offline backend with fixed fixtures, for UI development and screenshots. All text goes through [`sanitize`].
pub struct Demo {
    events: async_channel::Sender<Event>,
}

impl Demo {
    pub fn start(signed_in: bool) -> (Self, Events) {
        let (events, receiver) = async_channel::unbounded();
        let demo = Self { events };
        if signed_in {
            demo.sign_in()
        } else {
            demo.emit(Event::Auth(AuthStep::ApiCredentials))
        }
        (demo, receiver)
    }

    fn sign_in(&self) {
        self.emit(Event::Auth(AuthStep::Ready));
        for (id, name, username, is_contact) in [
            (2, "Alice Smith", Some("alice_smith"), true),
            (3, "\u{0410}lice Smith", Some("alice_smith_"), false),
            (4, "Bob Jones", Some("bobj"), true),
            (5, "P\u{0430}ypal Support", None, false),
        ] {
            self.emit(Event::User(SafeUser {
                id,
                name: sanitize(name),
                username: username.map(sanitize),
                is_contact,
                is_bot: false,
            }));
        }
        self.emit(Event::Folders {
            folders: vec![
                SafeFolder { id: 1, name: sanitize("Friends \u{1F60A}") },
                SafeFolder { id: 2, name: sanitize("Groups") },
            ],
            main_position: 0,
        });
        let (main, friends, groups) = (ChatListId::Main, ChatListId::Folder(1), ChatListId::Folder(2));
        for (id, kind, title, order, lists) in [
            (10, ChatKind::Private { user_id: 2 }, "Alice Smith", 6, vec![main, friends]),
            (HIKING, ChatKind::Group, "Weekend hiking \u{1F525}", 5, vec![main, friends, groups]),
            (12, ChatKind::Secret { user_id: 4 }, "Bob Jones", 4, vec![main, friends]),
            (13, ChatKind::Private { user_id: 5 }, "P\u{0430}ypal Support", 3, vec![ChatListId::Archive]),
            (14, ChatKind::Group, "Family \u{2764}\u{FE0F}", 2, vec![main, groups]),
            (FORUM, ChatKind::Forum, "Rust devs", 1, vec![main, groups]),
        ] {
            self.emit(Event::Chat(SafeChat { id, kind, title: sanitize(title) }));
            for list in lists {
                self.emit(Event::ChatPosition { chat_id: id, list, order });
            }
        }
    }

    fn emit(&self, event: Event) {
        self.events.try_send(event).expect("demo event receiver is gone");
    }
}

impl Backend for Demo {
    fn submit_api_credentials(&self, _: String, _: String) {
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

    fn open_chat(&self, chat_id: i64, topic: Option<i32>) {
        let messages = history(chat_id, topic);
        let reply = (chat_id == HIKING).then(|| {
            let original = &messages[3];
            SafeReply { sender: Some(original.sender), content: original.content.clone() }
        });
        self.emit(Event::History { chat_id, topic, messages });
        if let Some(reply) = reply {
            self.emit(Event::Reply { chat_id, message_id: 10, reply });
        }
    }

    fn load_topics(&self, chat_id: i64) {
        let topics = TOPICS.map(|(id, name)| SafeTopic { id, name: sanitize(name) }).into();
        self.emit(Event::Topics { chat_id, topics });
    }

    fn mark_read(&self, _: i64, _: Vec<i64>) {}

    fn send_text(&self, chat_id: i64, topic: Option<i32>, text: String) {
        if is_allowed(&text) {
            self.emit(Event::Message(message(chat_id, topic, 1000, ME, SafeContent::Text(sanitize(&text)))))
        } else {
            self.emit(Event::Error(sanitize("message contains characters outside the allowlist")))
        }
    }

    fn load_photo(&self, file_id: i32) {
        let (width, height) = (PHOTO.width as u32, PHOTO.height as u32);
        let bgra = (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| [(160 + x * 95 / width) as u8, (90 + y * 120 / height) as u8, 60, 255])
            })
            .collect();
        self.emit(Event::Photo { file_id, image: DecodedImage { width, height, bgra } });
    }
}

fn history(chat_id: i64, topic: Option<i32>) -> Vec<SafeMessage> {
    let text = |s: &str| SafeContent::Text(sanitize(s));
    let entries = match chat_id {
        HIKING => vec![
            (2, SafeContent::Service(sanitize("created the group Weekend hiking \u{1F525}"))),
            (2, text("Who's in for Saturday? \u{1F602}")),
            (4, text("Count me in \u{1F44D}")),
            (3, text("Hi all, Alice here, new number. Please send the cabin deposit to me today")),
            (4, SafeContent::Photo { photo: PHOTO, caption: sanitize("Trail map for the north loop") }),
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
    entries.into_iter().zip(1..).map(|((from, content), id)| message(chat_id, topic, id, from, content)).collect()
}

fn message(chat_id: i64, topic: Option<i32>, id: i64, from: i64, content: SafeContent) -> SafeMessage {
    SafeMessage { id, chat_id, sender: Sender::User(from), date: 0, outgoing: from == ME, topic, content }
}
