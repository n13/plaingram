use crate::image::DecodedImage;
use crate::text::SafeText;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthStep {
    ApiCredentials,
    Phone,
    Code,
    Password { hint: SafeText },
    Ready,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatKind {
    Private {
        user_id: i64,
    },
    Secret {
        user_id: i64,
    },
    Group,
    /// A group split into topics.
    Forum,
}

#[derive(Clone, Debug)]
pub struct SafeChat {
    pub id: i64,
    pub kind: ChatKind,
    pub title: SafeText,
}

/// "All chats", the Archive, or one of the user's folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatListId {
    Main,
    Archive,
    Folder(i32),
}

#[derive(Clone, Debug)]
pub struct SafeFolder {
    pub id: i32,
    pub name: SafeText,
}

#[derive(Clone, Debug)]
pub struct SafeTopic {
    pub id: i32,
    pub name: SafeText,
}

#[derive(Clone, Debug)]
pub struct SafeUser {
    pub id: i64,
    pub name: SafeText,
    pub username: Option<SafeText>,
    pub is_contact: bool,
    pub is_bot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sender {
    User(i64),
    Chat(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhotoRef {
    pub file_id: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug)]
pub enum SafeContent {
    Text(SafeText),
    Photo { photo: PhotoRef, caption: SafeText },
    Service(SafeText),
    Hidden(SafeText),
}

#[derive(Clone, Debug)]
pub struct SafeMessage {
    pub id: i64,
    pub chat_id: i64,
    pub sender: Sender,
    pub date: i32,
    pub outgoing: bool,
    /// The forum topic it was posted in.
    pub topic: Option<i32>,
    pub content: SafeContent,
}

/// The message another message replies to, or the quoted part of it.
#[derive(Clone, Debug)]
pub struct SafeReply {
    pub sender: Option<Sender>,
    pub content: SafeContent,
}

#[derive(Debug)]
pub enum Event {
    Auth(AuthStep),
    User(SafeUser),
    Chat(SafeChat),
    ChatTitle {
        chat_id: i64,
        title: SafeText,
    },
    /// `order` 0 means the chat left the list.
    ChatPosition {
        chat_id: i64,
        list: ChatListId,
        order: i64,
    },
    /// The user's folders in display order; "All chats" goes at `main_position`.
    Folders {
        folders: Vec<SafeFolder>,
        main_position: usize,
    },
    History {
        chat_id: i64,
        topic: Option<i32>,
        messages: Vec<SafeMessage>,
    },
    /// Messages older than message `before`, oldest first; empty at the start of the history.
    OlderHistory {
        chat_id: i64,
        topic: Option<i32>,
        before: i64,
        messages: Vec<SafeMessage>,
    },
    /// A forum's topics in display order.
    Topics {
        chat_id: i64,
        topics: Vec<SafeTopic>,
    },
    /// A new or renamed forum topic.
    Topic {
        chat_id: i64,
        topic: SafeTopic,
    },
    Message(SafeMessage),
    Reply {
        chat_id: i64,
        message_id: i64,
        reply: SafeReply,
    },
    MessageSent {
        old_id: i64,
        message: SafeMessage,
    },
    /// How much of a requested photo has downloaded, from 0 to 1.
    PhotoProgress {
        file_id: i32,
        progress: f32,
    },
    Photo {
        file_id: i32,
        image: DecodedImage,
    },
    Error(SafeText),
}

/// Commands the UI can issue; results arrive as [`Event`]s.
pub trait Backend {
    fn submit_api_credentials(&self, api_id: String, api_hash: String);
    fn submit_phone(&self, phone: String);
    fn submit_code(&self, code: String);
    fn submit_password(&self, password: String);
    /// Loads the history of a chat, or of one topic in a forum.
    fn open_chat(&self, chat_id: i64, topic: Option<i32>);
    /// Loads messages older than message `before`, as an [`Event::OlderHistory`].
    fn load_older(&self, chat_id: i64, topic: Option<i32>, before: i64);
    fn load_topics(&self, chat_id: i64);
    fn mark_read(&self, chat_id: i64, message_ids: Vec<i64>);
    fn send_text(&self, chat_id: i64, topic: Option<i32>, text: String);
    fn load_photo(&self, file_id: i32);
}
