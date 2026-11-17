use std::collections::HashSet;
use std::fmt::Debug;

use td::enums::{
    ChatList, ChatType, MessageContent, MessageOrigin, MessageSender, MessageTopic, ReactionType, UserType,
};
use td::types;

use crate::model::{
    ChatKind, ChatListId, PhotoRef, SafeAccountInfo, SafeChat, SafeContent, SafeFile, SafeFolder, SafeMessage,
    SafeProfile, SafeReaction, SafeReply, SafeTopic, SafeUser, Sender,
};
use crate::text::{SafeText, sanitize, sanitize_name};

const MAX_PHOTO_SIDE: i32 = 1280;

/// `forums` holds the supergroup ids of forums.
pub(crate) fn chat(chat: &types::Chat, bots: &HashSet<i64>, forums: &HashSet<i64>) -> Option<SafeChat> {
    let kind = chat_kind(&chat.r#type, bots, forums)?;
    Some(SafeChat { id: chat.id, kind, title: sanitize_name(&chat.title), unread: chat.unread_count })
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
        official: user.is_support || user.verification_status.as_ref().is_some_and(|s| s.is_verified),
    }
}

pub(crate) fn profile(user: &types::User, full: &types::UserFullInfo, photos: &types::ChatPhotos) -> SafeProfile {
    let status = user.verification_status.as_ref();
    let labels = [
        (user.is_premium, "Premium"),
        (status.is_some_and(|s| s.is_verified), "verified"),
        (status.is_some_and(|s| s.is_scam), "SCAM"),
        (status.is_some_and(|s| s.is_fake), "FAKE"),
        (full.uses_unofficial_app, "unofficial app"),
    ];
    SafeProfile {
        user_id: user.id,
        bio: sanitize(full.bio.as_ref().map_or("", |bio| &bio.text)),
        photo: full.photo.as_ref().and_then(|p| photo(&p.sizes)),
        photo_dates: photos.photos.iter().map(|p| p.added_date).collect(),
        photo_count: photos.total_count,
        groups_in_common: full.group_in_common_count,
        labels: labels.into_iter().filter_map(|(on, label)| on.then_some(label)).collect(),
    }
}

pub(crate) fn account_info(info: &types::AccountInfo) -> SafeAccountInfo {
    let date = |date: i32| (date > 0).then_some(date);
    let month = u32::try_from(info.registration_month).ok().filter(|m| (1..=12).contains(m));
    SafeAccountInfo {
        registered: month.zip((info.registration_year > 0).then_some(info.registration_year)),
        phone_country: sanitize(&info.phone_number_country_code),
        name_changed: date(info.last_name_change_date),
        photo_changed: date(info.last_photo_change_date),
    }
}

pub(crate) fn download_progress(file: &types::File) -> f32 {
    (file.local.downloaded_size as f32 / file_size(file).max(1) as f32).clamp(0., 1.)
}

/// The exact size when known, else TDLib's estimate.
fn file_size(file: &types::File) -> i64 {
    if file.size > 0 { file.size } else { file.expected_size }
}

fn full_name(user: &types::User) -> String {
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
        reactions: reactions(message.interaction_info.as_ref()),
    }
}

/// What a custom emoji reaction, a picture, shows as.
pub(crate) const CUSTOM_EMOJI: &str = "custom emoji";
/// What a paid reaction shows as: a star, as Telegram draws it.
pub(crate) const PAID: &str = "\u{2B50}";

/// The reactions on a message, in Telegram's order, most first.
pub(crate) fn reactions(info: Option<&types::MessageInteractionInfo>) -> Vec<SafeReaction> {
    let Some(reactions) = info.and_then(|info| info.reactions.as_ref()) else { return Vec::new() };
    reactions
        .reactions
        .iter()
        .map(|reaction| SafeReaction {
            emoji: match &reaction.r#type {
                ReactionType::Emoji(emoji) => sanitize(&emoji.emoji),
                ReactionType::CustomEmoji(_) => sanitize(CUSTOM_EMOJI),
                ReactionType::Paid => sanitize(PAID),
            },
            count: reaction.total_count,
            chosen: reaction.is_chosen,
        })
        .collect()
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
        MessageContent::MessagePhoto(p) => match photo(&p.photo.sizes) {
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
        MessageContent::MessageDocument(d) => file(content, &d.document.file_name, &d.document.document),
        MessageContent::MessageVideo(v) => file(content, &v.video.file_name, &v.video.video),
        MessageContent::MessageAudio(a) => file(content, &a.audio.file_name, &a.audio.audio),
        MessageContent::MessageAnimation(a) => file(content, &a.animation.file_name, &a.animation.animation),
        MessageContent::MessageVoiceNote(v) => file(content, "", &v.voice_note.voice),
        MessageContent::MessageVideoNote(v) => file(content, "", &v.video_note.video),
        _ => hidden(content),
    }
}

/// The largest size within [`MAX_PHOTO_SIDE`].
fn photo(sizes: &[types::PhotoSize]) -> Option<PhotoRef> {
    sizes.iter().filter(|s| s.width.max(s.height) <= MAX_PHOTO_SIDE).max_by_key(|s| s.width).map(|s| PhotoRef {
        file_id: s.photo.id,
        width: s.width,
        height: s.height,
    })
}

fn hidden(content: &MessageContent) -> SafeContent {
    SafeContent::Hidden(kind(content))
}

fn file(content: &MessageContent, name: &str, file: &types::File) -> SafeContent {
    SafeContent::File {
        kind: kind(content),
        file: SafeFile { file_id: file.id, name: sanitize(name), size: file_size(file) },
    }
}

/// "Document" for `MessageDocument`.
fn kind(content: &MessageContent) -> SafeText {
    sanitize(variant_name(content).trim_start_matches("Message"))
}

pub(crate) fn variant_name(value: &impl Debug) -> String {
    let debug = format!("{value:?}");
    debug.split(['(', ' ', '{']).next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::is_allowed;

    /// Telegram's standard reactions, as it sends them: some without the variation selector that makes them emoji.
    const REACTIONS: [&str; 74] = [
        "\u{1F44D}",
        "\u{1F44E}",
        "\u{2764}",
        "\u{1F525}",
        "\u{1F970}",
        "\u{1F44F}",
        "\u{1F601}",
        "\u{1F914}",
        "\u{1F92F}",
        "\u{1F631}",
        "\u{1F92C}",
        "\u{1F622}",
        "\u{1F389}",
        "\u{1F929}",
        "\u{1F92E}",
        "\u{1F4A9}",
        "\u{1F64F}",
        "\u{1F44C}",
        "\u{1F54A}",
        "\u{1F921}",
        "\u{1F971}",
        "\u{1F974}",
        "\u{1F60D}",
        "\u{1F433}",
        "\u{2764}\u{200D}\u{1F525}",
        "\u{1F31A}",
        "\u{1F32D}",
        "\u{1F4AF}",
        "\u{1F923}",
        "\u{26A1}",
        "\u{1F34C}",
        "\u{1F3C6}",
        "\u{1F494}",
        "\u{1F928}",
        "\u{1F610}",
        "\u{1F353}",
        "\u{1F37E}",
        "\u{1F48B}",
        "\u{1F595}",
        "\u{1F608}",
        "\u{1F634}",
        "\u{1F62D}",
        "\u{1F913}",
        "\u{1F47B}",
        "\u{1F468}\u{200D}\u{1F4BB}",
        "\u{1F440}",
        "\u{1F383}",
        "\u{1F648}",
        "\u{1F607}",
        "\u{1F628}",
        "\u{1F91D}",
        "\u{270D}",
        "\u{1F917}",
        "\u{1FAE1}",
        "\u{1F385}",
        "\u{1F384}",
        "\u{2603}",
        "\u{1F485}",
        "\u{1F92A}",
        "\u{1F5FF}",
        "\u{1F192}",
        "\u{1F498}",
        "\u{1F649}",
        "\u{1F984}",
        "\u{1F618}",
        "\u{1F48A}",
        "\u{1F64A}",
        "\u{1F60E}",
        "\u{1F47E}",
        "\u{1F937}\u{200D}\u{2642}",
        "\u{1F937}",
        "\u{1F937}\u{200D}\u{2640}",
        "\u{1F621}",
        "\u{2B50}",
    ];

    #[test]
    fn telegram_reactions_pass_the_allowlist() {
        let blocked: Vec<_> = REACTIONS.iter().filter(|r| !is_allowed(r)).collect();
        assert!(blocked.is_empty(), "{blocked:?}");
        assert!(is_allowed(CUSTOM_EMOJI) && is_allowed(PAID));
    }

    #[test]
    fn reactions_keep_telegrams_order_show_as_emoji_and_label_custom_and_paid_ones() {
        let reaction = |r#type, total_count, is_chosen| types::MessageReaction {
            r#type,
            total_count,
            is_chosen,
            used_sender_id: None,
            recent_sender_ids: Vec::new(),
        };
        let info = types::MessageInteractionInfo {
            view_count: 0,
            forward_count: 0,
            reply_info: None,
            reactions: Some(types::MessageReactions {
                reactions: vec![
                    reaction(ReactionType::Emoji(types::ReactionTypeEmoji { emoji: "\u{2764}".into() }), 5, true),
                    reaction(
                        ReactionType::CustomEmoji(types::ReactionTypeCustomEmoji { custom_emoji_id: 1 }),
                        2,
                        false,
                    ),
                    reaction(ReactionType::Paid, 1, false),
                ],
                are_tags: false,
                paid_reactors: Vec::new(),
                can_get_added_reactions: false,
            }),
        };
        let shown: Vec<_> =
            reactions(Some(&info)).into_iter().map(|r| (r.emoji.to_string(), r.count, r.chosen)).collect();
        assert_eq!(shown, [("\u{2764}\u{FE0F}".into(), 5, true), (CUSTOM_EMOJI.into(), 2, false), (PAID.into(), 1, false)]);
        assert!(reactions(None).is_empty());
    }

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

    #[test]
    fn account_info_drops_unknown_values() {
        let info = account_info(&types::AccountInfo {
            registration_month: 0,
            registration_year: 2024,
            phone_number_country_code: "NG".into(),
            last_name_change_date: 1_800_000_000,
            last_photo_change_date: 0,
        });
        assert_eq!(info.registered, None);
        assert_eq!(info.phone_country.to_string(), "NG");
        assert_eq!((info.name_changed, info.photo_changed), (Some(1_800_000_000), None));
    }

    #[test]
    fn documents_and_voice_messages_become_files_that_can_be_saved() {
        let file = |id| types::File { id, size: 2048, ..Default::default() };
        let document = MessageContent::MessageDocument(types::MessageDocument {
            document: types::Document {
                file_name: "notes\u{202E}piz.exe".into(),
                mime_type: String::new(),
                minithumbnail: None,
                thumbnail: None,
                document: file(7),
            },
            caption: types::FormattedText::default(),
        });
        let SafeContent::File { kind, file: saved } = content(&document) else { panic!("{document:?}") };
        assert_eq!((kind.to_string(), saved.file_id, saved.size), ("Document".into(), 7, 2048));
        assert_eq!(saved.name.to_string(), "notes?piz.exe", "a right-to-left override cannot disguise the extension");
        let voice = MessageContent::MessageVoiceNote(types::MessageVoiceNote {
            voice_note: types::VoiceNote { voice: file(8), ..Default::default() },
            ..Default::default()
        });
        let SafeContent::File { kind, file: saved } = content(&voice) else { panic!("{voice:?}") };
        assert_eq!((kind.to_string(), saved.file_id, saved.name.is_empty()), ("VoiceNote".into(), 8, true));
    }
}
