use crate::image::DecodedImage;
use crate::text::SafeText;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthStep {
    ApiCredentials,
    /// Scan `link` as a QR code with Telegram on a signed-in device. It changes every 30 seconds or so.
    Qr {
        link: String,
    },
    Phone,
    Code,
    Password {
        hint: SafeText,
    },
    Ready,
    /// TDLib closed, after logging out or leaving the QR code for a phone number; a fresh instance is starting.
    Closed,
}

/// How a message can be deleted, as Telegram allows for this account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteOptions {
    /// Only from this account's copy of the chat.
    pub for_me: bool,
    /// For every member of the chat.
    pub for_everyone: bool,
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

/// What a profile shows beyond [`SafeUser`], for judging whether an account is who it claims to be.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeProfile {
    pub user_id: i64,
    pub bio: SafeText,
    /// The current profile photo.
    pub photo: Option<PhotoRef>,
    /// When each profile photo was set, newest first, from the first page of them.
    pub photo_dates: Vec<i32>,
    pub photo_count: i32,
    pub groups_in_common: i32,
    /// Telegram's labels, such as "Premium", "SCAM" or "unofficial app".
    pub labels: Vec<&'static str>,
}

/// What Telegram shares about someone who messaged you first.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeAccountInfo {
    /// Month (1 to 12) and year the account was registered.
    pub registered: Option<(u32, i32)>,
    /// Two-letter country code of the phone number; empty if unknown.
    pub phone_country: SafeText,
    pub name_changed: Option<i32>,
    pub photo_changed: Option<i32>,
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
pub struct SafeFile {
    pub file_id: i32,
    /// The name the sender gave it; empty for voice and video messages.
    pub name: SafeText,
    pub size: i64,
}

#[derive(Clone, Debug)]
pub enum SafeContent {
    Text(SafeText),
    Photo {
        photo: PhotoRef,
        caption: SafeText,
    },
    /// A document, video or voice message: never shown or opened, but its file can be saved.
    File {
        kind: SafeText,
        file: SafeFile,
    },
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
    /// Chats opened on this computer, the most recent first.
    RecentChats(Vec<i64>),
    DeleteOptions {
        chat_id: i64,
        message_id: i64,
        options: DeleteOptions,
    },
    /// Messages gone from a chat, deleted here or by someone else.
    MessagesDeleted {
        chat_id: i64,
        message_ids: Vec<i64>,
    },
    /// The signed-in account.
    Account {
        user_id: i64,
        phone: SafeText,
    },
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
    /// How much of a requested photo or file has downloaded, from 0 to 1.
    DownloadProgress {
        file_id: i32,
        progress: f32,
    },
    Photo {
        file_id: i32,
        image: DecodedImage,
    },
    /// A photo or file was saved to `path`.
    Saved {
        file_id: i32,
        path: SafeText,
    },
    Profile(SafeProfile),
    AccountInfo {
        user_id: i64,
        info: SafeAccountInfo,
    },
    Error(SafeText),
}

/// Commands the UI can issue; results arrive as [`Event`]s.
pub trait Backend {
    fn submit_api_credentials(&self, api_id: String, api_hash: String);
    /// Signs in by scanning a QR code with Telegram on another device, the default.
    fn log_in_with_qr(&self);
    /// Signs in with a phone number and a code instead.
    fn log_in_with_phone(&self);
    fn submit_phone(&self, phone: String);
    fn submit_code(&self, code: String);
    fn submit_password(&self, password: String);
    /// Logs out and deletes everything TDLib stored, then starts over at sign-in.
    fn log_out(&self);
    /// Loads the history of a chat, or of one topic in a forum.
    fn open_chat(&self, chat_id: i64, topic: Option<i32>);
    /// Loads messages older than message `before`, as an [`Event::OlderHistory`].
    fn load_older(&self, chat_id: i64, topic: Option<i32>, before: i64);
    fn load_topics(&self, chat_id: i64);
    fn mark_read(&self, chat_id: i64, message_ids: Vec<i64>);
    /// Emits [`Event::DeleteOptions`].
    fn load_delete_options(&self, chat_id: i64, message_id: i64);
    fn delete_message(&self, chat_id: i64, message_id: i64, for_everyone: bool);
    fn send_text(&self, chat_id: i64, topic: Option<i32>, text: String);
    /// Sends each file as a photo, upright, scaled to Telegram's limit and re-encoded as JPEG; `caption` goes with the first.
    fn send_photos(&self, chat_id: i64, topic: Option<i32>, files: Vec<std::path::PathBuf>, caption: String);
    fn load_photo(&self, file_id: i32);
    /// Saves a photo to ~/Downloads as `name`, re-encoded so no byte from its sender reaches the disk.
    fn save_photo(&self, file_id: i32, name: String);
    /// Saves a file to ~/Downloads as `name`, or as TDLib named it when `name` is empty.
    fn save_file(&self, file_id: i32, name: String);
    /// Loads a user's full profile, as an [`Event::Profile`].
    fn load_profile(&self, user_id: i64);
}
