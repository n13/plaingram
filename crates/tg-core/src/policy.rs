use std::collections::HashSet;
use std::fmt::Debug;

use td::enums::{ChatList, ChatType, MessageContent, MessageOrigin, MessageSender, MessageTopic, UserType};
use td::types;

use crate::model::{
    ChatKind, ChatListId, PhotoRef, SafeChat, SafeContent, SafeFolder, SafeMessage, SafeReply, SafeTopic, SafeUser,
    Sender,
};
use crate::text::{sanitize, sanitize_name};

const MAX_PHOTO_SIDE: i32 = 1280;

/// `forums` holds the supergroup ids of forums.
pub(crate) fn chat(chat: &types::Chat, bots: &HashSet<i64>, forums: &HashSet<i64>) -> Option<SafeChat> {
    let kind = chat_kind(&chat.r#type, bots, forums)?;
    Some(SafeChat { id: chat.id, kind, title: sanitize_name(&chat.title) })
}

fn chat_kind(chat_type: &ChatType, bots: &HashSet<i64>, forums: &HashSet<i64>) -> Option<ChatKind> {
    match chat_type {
        ChatType::Private(p) if !bots.contains(&p.user_id) => Some(ChatKind::Private { user_id: p.user_id }),
        ChatType::Secret(s) if !bots.contains(&s.user_id) => Some(ChatKind::Secret { user_id: s.user_id }),
        ChatType::BasicGroup(_) => Some(ChatKind::Group),
        ChatType::Supergroup(s) if !s.is_channel && forums.contains(&s.supergroup_id) => Some(ChatKind::Forum),
        ChatType::Supergroup(s) if !s.is_channel => Some(ChatKind::Group),
        _ => None,
    }
}

pub(crate) fn chat_list(list: &ChatList) -> ChatListId {
    match list {
        ChatList::Main => ChatListId::Main,
        ChatList::Archive => ChatListId::Archive,
        ChatList::Folder(f) => ChatListId::Folder(f.chat_folder_id),
    }
}

pub(crate) fn folder(folder: &types::ChatFolderInfo) -> SafeFolder {
    SafeFolder { id: folder.id, name: sanitize(&folder.name.text.text) }
}

pub(crate) fn topic(info: &types::ForumTopicInfo) -> SafeTopic {
    SafeTopic { id: info.forum_topic_id, name: sanitize(&info.name) }
}

pub(crate) fn user(user: &types::User) -> SafeUser {
    SafeUser {
        id: user.id,
        name: sanitize_name(&full_name(user)),
        username: user.usernames.as_ref().and_then(|u| u.active_usernames.first()).map(|u| sanitize_name(u)),
        is_contact: user.is_contact,
        is_bot: matches!(user.r#type, UserType::Bot(_)),
    }
}

pub(crate) fn download_progress(file: &types::File) -> f32 {
    let size = if file.size > 0 { file.size } else { file.expected_size };
    (file.local.downloaded_size as f32 / size.max(1) as f32).clamp(0., 1.)
}

pub(crate) fn full_name(user: &types::User) -> String {
    format!("{} {}", user.first_name, user.last_name).trim().to_owned()
}

pub(crate) fn message(message: &types::Message) -> SafeMessage {
    SafeMessage {
        id: message.id,
        chat_id: message.chat_id,
        sender: sender(&message.sender_id),
        date: message.date,
        outgoing: message.is_outgoing,
        topic: match &message.topic_id {
            Some(MessageTopic::Forum(t)) => Some(t.forum_topic_id),
            _ => None,
        },
        content: content(&message.content),
    }
}

pub(crate) fn sender(sender: &MessageSender) -> Sender {
    match sender {
        MessageSender::User(u) => Sender::User(u.user_id),
        MessageSender::Chat(c) => Sender::Chat(c.chat_id),
    }
}

/// The original message's `sender` and `content`, or the `quote` from it; both are missing if it is unavailable.
pub(crate) fn reply(
    sender: Option<Sender>,
    content: Option<&MessageContent>,
    quote: Option<&types::TextQuote>,
) -> SafeReply {
    let content = match (quote, content) {
        (Some(quote), _) => SafeContent::Text(sanitize(&quote.text.text)),
        (None, Some(content)) => self::content(content),
        (None, None) => SafeContent::Service(sanitize("message unavailable")),
    };
    SafeReply { sender, content }
}

pub(crate) fn origin(origin: &MessageOrigin) -> Option<Sender> {
    match origin {
        MessageOrigin::User(o) => Some(Sender::User(o.sender_user_id)),
        MessageOrigin::Chat(o) => Some(Sender::Chat(o.sender_chat_id)),
        MessageOrigin::Channel(o) => Some(Sender::Chat(o.chat_id)),
        MessageOrigin::HiddenUser(_) => None,
    }
}

fn content(content: &MessageContent) -> SafeContent {
    let service = |text: String| SafeContent::Service(sanitize(&text));
    match content {
        MessageContent::MessageText(t) => SafeContent::Text(sanitize(&t.text.text)),
        MessageContent::MessagePhoto(p) => match photo(&p.photo) {
            Some(photo) => SafeContent::Photo { photo, caption: sanitize(&p.caption.text) },
            None => hidden(content),
        },
        MessageContent::MessageBasicGroupChatCreate(g) => service(format!("created the group {}", g.title)),
        MessageContent::MessageSupergroupChatCreate(g) => service(format!("created the group {}", g.title)),
        MessageContent::MessageChatChangeTitle(t) => service(format!("renamed the group to {}", t.title)),
        MessageContent::MessageChatAddMembers(m) => service(format!("added {} member(s)", m.member_user_ids.len())),
        MessageContent::MessageChatJoinByLink => service("joined via invite link".into()),
        MessageContent::MessageChatJoinByRequest => service("joined after approval".into()),
        MessageContent::MessageChatDeleteMember(_) => service("removed a member".into()),
        MessageContent::MessageChatChangePhoto(_) | MessageContent::MessageChatDeletePhoto => {
            service("changed the group photo".into())
        }
        MessageContent::MessageChatUpgradeTo(_) | MessageContent::MessageChatUpgradeFrom(_) => {
            service("upgraded the group".into())
        }
        MessageContent::MessagePinMessage(_) => service("pinned a message".into()),
        _ => hidden(content),
    }
}

fn photo(photo: &types::Photo) -> Option<PhotoRef> {
    photo.sizes.iter().filter(|s| s.width.max(s.height) <= MAX_PHOTO_SIDE).max_by_key(|s| s.width).map(|s| PhotoRef {
        file_id: s.photo.id,
        width: s.width,
        height: s.height,
    })
}

fn hidden(content: &MessageContent) -> SafeContent {
    SafeContent::Hidden(sanitize(variant_name(content).trim_start_matches("Message")))
}

pub(crate) fn variant_name(value: &impl Debug) -> String {
    let debug = format!("{value:?}");
    debug.split(['(', ' ', '{']).next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(chat_type: ChatType) -> Option<ChatKind> {
        chat_kind(&chat_type, &HashSet::from([7]), &HashSet::from([2]))
    }

    fn size(width: i32, height: i32, file_id: i32) -> types::PhotoSize {
        types::PhotoSize {
            width,
            height,
            photo: types::File { id: file_id, ..Default::default() },
            ..Default::default()
        }
    }

    fn photo_message(sizes: Vec<types::PhotoSize>) -> MessageContent {
        MessageContent::MessagePhoto(types::MessagePhoto {
            photo: types::Photo { sizes, ..Default::default() },
            video: None,
            caption: types::FormattedText { text: "hi".into(), entities: Vec::new() },
            show_caption_above_media: false,
            has_spoiler: false,
            is_secret: false,
        })
    }

    #[test]
    fn only_people_and_groups_are_allowed() {
        let private = |user_id| ChatType::Private(types::ChatTypePrivate { user_id });
        let supergroup = |is_channel| ChatType::Supergroup(types::ChatTypeSupergroup { supergroup_id: 1, is_channel });
        assert_eq!(kind(private(1)), Some(ChatKind::Private { user_id: 1 }));
        assert_eq!(kind(private(7)), None);
        assert_eq!(kind(ChatType::Secret(types::ChatTypeSecret { secret_chat_id: 1, user_id: 7 })), None);
        assert_eq!(kind(ChatType::BasicGroup(types::ChatTypeBasicGroup { basic_group_id: 1 })), Some(ChatKind::Group));
        assert_eq!(kind(supergroup(false)), Some(ChatKind::Group));
        assert_eq!(kind(supergroup(true)), None);
        let forum = ChatType::Supergroup(types::ChatTypeSupergroup { supergroup_id: 2, is_channel: false });
        assert_eq!(kind(forum), Some(ChatKind::Forum));
    }

    #[test]
    fn chat_lists_and_folder_names() {
        let list = ChatList::Folder(types::ChatListFolder { chat_folder_id: 3 });
        assert_eq!(chat_list(&list), ChatListId::Folder(3));
        let info = types::ChatFolderInfo {
            id: 3,
            name: types::ChatFolderName {
                text: types::FormattedText { text: "W\u{043E}rk".into(), entities: Vec::new() },
                animate_custom_emoji: false,
            },
            ..Default::default()
        };
        assert_eq!(folder(&info).name.to_string(), "W?rk");
    }

    #[test]
    fn text_is_sanitized_and_entities_ignored() {
        let content = content(&MessageContent::MessageText(types::MessageText {
            text: types::FormattedText { text: "P\u{0430}y".into(), entities: Vec::new() },
            link_preview: None,
            link_preview_options: None,
        }));
        let SafeContent::Text(text) = content else { panic!("{content:?}") };
        assert_eq!(text.to_string(), "P?y");
    }

    #[test]
    fn replies_prefer_the_quote_and_survive_missing_originals() {
        let original = MessageContent::MessageChatJoinByLink;
        let quote = types::TextQuote {
            text: types::FormattedText { text: "P\u{0430}y me".into(), entities: Vec::new() },
            ..Default::default()
        };
        let quoted = reply(Some(Sender::User(2)), Some(&original), Some(&quote));
        assert!(matches!(quoted.content, SafeContent::Text(t) if t.to_string() == "P?y me"));
        assert!(
            matches!(reply(None, Some(&original), None).content, SafeContent::Service(t) if t.to_string() == "joined via invite link")
        );
        assert!(
            matches!(reply(None, None, None).content, SafeContent::Service(t) if t.to_string() == "message unavailable")
        );
    }

    #[test]
    fn photo_uses_largest_size_within_limit() {
        let content = content(&photo_message(vec![size(320, 240, 1), size(800, 600, 2), size(2560, 1920, 3)]));
        let SafeContent::Photo { photo, .. } = content else { panic!("{content:?}") };
        assert_eq!(photo, PhotoRef { file_id: 2, width: 800, height: 600 });
        assert!(matches!(super::content(&photo_message(vec![size(4000, 3000, 1)])), SafeContent::Hidden(_)));
    }

    #[test]
    fn everything_else_is_hidden_with_its_kind() {
        for (content, kind) in [
            (MessageContent::MessageScreenshotTaken, "ScreenshotTaken"),
            (MessageContent::MessageExpiredPhoto, "ExpiredPhoto"),
            (MessageContent::MessageContactRegistered, "ContactRegistered"),
        ] {
            let SafeContent::Hidden(text) = super::content(&content) else { panic!("{content:?}") };
            assert_eq!(text.to_string(), kind);
        }
    }

    #[test]
    fn service_messages_sanitize_titles() {
        let content = content(&MessageContent::MessageChatChangeTitle(types::MessageChatChangeTitle {
            title: "\u{0410}dmins".into(),
        }));
        let SafeContent::Service(text) = content else { panic!("{content:?}") };
        assert_eq!(text.to_string(), "renamed the group to \u{0410}?????");
    }
}
