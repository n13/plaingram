use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use chrono::{DateTime, Local, NaiveDate};
use gpui_kit::base::actions::Cancel;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use tg_core::{
    AuthStep, Backend, ChatKind, ChatListId, DecodedImage, DeleteOptions, Event, Events, PhotoRef, SafeAccountInfo,
    SafeChat, SafeContent, SafeFolder, SafeMessage, SafeProfile, SafeReply, SafeText, SafeTopic, SafeUser, Sender,
    is_allowed, sanitize,
};

use crate::safe_text::{name_label, safe_text, warning};

const SIDEBAR_WIDTH: f32 = 300.;
const CHAT_ROW_HEIGHT: f32 = 56.;
const PHOTO_MAX_WIDTH: f32 = 360.;
const PROFILE_PHOTO_WIDTH: f32 = 200.;
const PROFILE_WIDTH: f32 = 400.;
/// Once the reader scrolls up, older messages load while fewer than this many are above the viewport.
const PREFETCH_ROWS: usize = 300;
/// Photos load this long before the scroll velocity brings them on screen.
const PHOTO_LOOKAHEAD: Duration = Duration::from_secs(1);
/// The most rows ahead whose photos load, so a scrollbar jump does not fetch a whole chat.
const PHOTO_LOOKAHEAD_ROWS: usize = 30;
/// How far back scroll positions count toward the scroll velocity.
const VELOCITY_WINDOW: Duration = Duration::from_millis(300);
const QR_STEPS: [&str; 3] =
    ["1. Open Telegram on your phone", "2. Go to Settings > Devices > Add Device", "3. Scan this image to log in"];
/// Logical pixels; the code is drawn with 2 image pixels per point for Retina screens.
const QR_SIZE: f32 = 240.;
/// Logical pixels of an attached photo's preview.
const ATTACHMENT_SIZE: f32 = 96.;
const SWITCHER_WIDTH: f32 = 480.;
/// How far below the title bar a dialog sits.
const MODAL_TOP: f32 = 60.;
const DELETE_WIDTH: f32 = 440.;
const SWITCHER_LIST_HEIGHT: f32 = 420.;
/// Light modules around the code, as the QR standard asks.
const QR_QUIET_ZONE: usize = 4;
const API_SETUP_STEPS: [&str; 4] = [
    "Telegram requires every app to have its own API ID. This is a one-time setup:",
    "1. Sign in at my.telegram.org and open API development tools.",
    "2. Create an app: title and short name plaingram, platform Desktop.",
    "3. Paste its api_id and api_hash below. They are stored in your system keychain.",
];

pub struct Plaingram {
    client: Rc<dyn Backend>,
    /// Kept to rebuild this view from scratch when TDLib closes.
    events: Events,
    pub(crate) auth: Option<AuthStep>,
    /// The QR code for [`AuthStep::Qr`].
    qr_code: Option<Arc<RenderImage>>,
    /// The signed-in user and their phone number.
    account: Option<(i64, SafeText)>,
    login: Entity<InputState>,
    api_hash: Entity<InputState>,
    pub(crate) compose: Entity<InputState>,
    pub(crate) scroller: Entity<MessageScrollerState>,
    users: HashMap<i64, SafeUser>,
    chats: HashMap<i64, SafeChat>,
    positions: HashMap<ChatListId, HashMap<i64, i64>>,
    folders: Vec<(ChatListId, SharedString)>,
    folder_ix: usize,
    chat_list: Vec<i64>,
    /// Chats by when they were last opened, the most recent first.
    recent: Vec<i64>,
    pub(crate) open_chat: Option<i64>,
    /// Topics of the open chat if it is a forum.
    topics: Vec<SafeTopic>,
    topic: Option<i32>,
    pub(crate) messages: Vec<SafeMessage>,
    older: Older,
    /// What each message in `messages` replies to, by message id.
    replies: HashMap<i64, SafeReply>,
    photos: HashMap<i32, Arc<RenderImage>>,
    pub(crate) requested_photos: HashSet<i32>,
    /// How far each photo or file being downloaded has come, from 0 to 1.
    pub(crate) download_progress: HashMap<i32, f32>,
    pub(crate) overlay: Option<Overlay>,
    /// Photos picked or dropped for the next message.
    pub(crate) attachments: Vec<PathBuf>,
    /// The chat switcher, open over everything.
    pub(crate) switcher: Option<Entity<CommandState>>,
    /// The message whose Delete was picked, until the user chooses how.
    pub(crate) deleting: Option<Deleting>,
    overlay_focus: FocusHandle,
    /// The user whose profile is open beside the chat.
    pub(crate) profile: Option<i64>,
    pub(crate) profiles: HashMap<i64, SafeProfile>,
    /// What Telegram shared about people who messaged first, by user id.
    account_info: HashMap<i64, SafeAccountInfo>,
    /// The first and last rows on screen in the frame being drawn.
    visible_rows: Rc<Cell<Option<(usize, usize)>>>,
    /// When each recent frame was drawn and its top row, for the scroll velocity.
    scroll_samples: VecDeque<(Instant, usize)>,
    error: Option<SafeText>,
    _subscriptions: [Subscription; 5],
    _tasks: [Task<()>; 2],
}

/// What covers the window below the title bar.
pub(crate) enum Overlay {
    Photo(Viewer),
    /// `confirm_log_out` asks before logging out.
    Settings {
        confirm_log_out: bool,
    },
}

/// A message to delete, with how Telegram allows it once known.
pub(crate) struct Deleting {
    chat_id: i64,
    message_id: i64,
    options: Option<DeleteOptions>,
}

/// A photo shown over the whole window.
#[derive(Clone)]
pub(crate) struct Viewer {
    pub(crate) photo: PhotoRef,
    /// The file name a chat photo is saved as; profile photos cannot be saved.
    pub(crate) save_as: Option<String>,
}

/// Whether messages older than the loaded ones can still be fetched.
#[derive(Clone, Copy, PartialEq)]
enum Older {
    Available,
    Loading,
    Exhausted,
}

impl Plaingram {
    pub fn new(client: Rc<dyn Backend>, events: Events, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let login = cx.new(|cx| InputState::new(window, cx));
        let api_hash = cx.new(|cx| InputState::new(window, cx).placeholder("API hash").masked(true));
        let compose = cx.new(|cx| InputState::new(window, cx).placeholder("Message"));
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let on_enter = |action: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            move |this: &mut Self,
                  _: &Entity<InputState>,
                  event: &InputEvent,
                  window: &mut Window,
                  cx: &mut Context<Self>| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    action(this, window, cx);
                }
            }
        };
        let app = cx.weak_entity();
        let subscriptions = [
            cx.subscribe_in(&login, window, on_enter(Self::submit_login)),
            cx.subscribe_in(&api_hash, window, on_enter(Self::submit_login)),
            cx.subscribe_in(&compose, window, on_enter(Self::send)),
            cx.observe(&scroller, |this, _, cx| this.prefetch(cx)),
            cx.intercept_keystrokes(move |event, window, cx| {
                if event.keystroke.key == "k" && event.keystroke.modifiers == Modifiers::secondary_key(cx) {
                    match app.update(cx, |app, cx| app.toggle_switcher(window, cx)) {
                        Ok(()) => cx.stop_propagation(),
                        Err(error) => tracing::error!("cannot toggle the chat switcher: {error:#}"),
                    }
                }
            }),
        ];
        let receiver = events.clone();
        let events_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this.update_in(cx, |app, window, cx| app.apply(event, window, cx)).is_err() {
                    return;
                }
            }
        });
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(60)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        });
        Self {
            client,
            events,
            auth: None,
            qr_code: None,
            account: None,
            login,
            api_hash,
            compose,
            scroller,
            users: HashMap::new(),
            chats: HashMap::new(),
            positions: HashMap::new(),
            folders: folder_tabs(&[], 0),
            folder_ix: 0,
            chat_list: Vec::new(),
            recent: Vec::new(),
            open_chat: None,
            topics: Vec::new(),
            topic: None,
            messages: Vec::new(),
            older: Older::Available,
            replies: HashMap::new(),
            photos: HashMap::new(),
            requested_photos: HashSet::new(),
            download_progress: HashMap::new(),
            overlay: None,
            attachments: Vec::new(),
            switcher: None,
            deleting: None,
            overlay_focus: cx.focus_handle(),
            profile: None,
            profiles: HashMap::new(),
            account_info: HashMap::new(),
            visible_rows: Rc::default(),
            scroll_samples: VecDeque::new(),
            error: None,
            _subscriptions: subscriptions,
            _tasks: [events_task, clock],
        }
    }

    fn apply(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Auth(step) => self.set_auth(step, window, cx),
            Event::User(user) => _ = self.users.insert(user.id, user),
            Event::Chat(chat) => _ = self.chats.insert(chat.id, chat),
            Event::ChatTitle { chat_id, title } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.title = title;
                }
            }
            Event::ChatPosition { chat_id, list, order } => {
                let orders = self.positions.entry(list).or_default();
                if order == 0 {
                    orders.remove(&chat_id);
                } else {
                    orders.insert(chat_id, order);
                }
                if list == self.folder() {
                    self.sort_chats();
                }
            }
            Event::Folders { folders, main_position } => {
                let selected = self.folder();
                self.folders = folder_tabs(&folders, main_position);
                self.folder_ix = self.folders.iter().position(|(list, _)| *list == selected).unwrap_or(main_position);
                self.sort_chats();
            }
            Event::History { chat_id, topic, messages } if self.open_chat == Some(chat_id) && self.topic == topic => {
                let count = messages.len();
                self.messages = messages;
                self.older = Older::Available;
                self.scroll_samples.clear();
                self.scroller.update(cx, |scroller, cx| scroller.reset(count, cx));
            }
            Event::OlderHistory { chat_id, topic, before, messages }
                if self.open_chat == Some(chat_id)
                    && self.topic == topic
                    && self.messages.first().map(|m| m.id) == Some(before) =>
            {
                self.older = if messages.is_empty() { Older::Exhausted } else { Older::Available };
                let count = messages.len();
                self.scroll_samples.iter_mut().for_each(|(_, top)| *top += count);
                self.messages.splice(0..0, messages);
                self.scroller.update(cx, |scroller, cx| _ = scroller.prepend(count, cx));
            }
            Event::Topics { chat_id, topics } if self.open_chat == Some(chat_id) => {
                self.topics = topics;
                if !self.topics.is_empty() {
                    self.select_topic(0, cx);
                }
            }
            Event::Topic { chat_id, topic } if self.open_chat == Some(chat_id) => {
                match self.topics.iter_mut().find(|t| t.id == topic.id) {
                    Some(slot) => *slot = topic,
                    None => self.topics.push(topic),
                }
            }
            Event::Reply { chat_id, message_id, reply } if self.open_chat == Some(chat_id) => {
                self.replies.insert(message_id, reply);
            }
            Event::Message(message) if self.open_chat == Some(message.chat_id) && self.topic == message.topic => {
                if !message.outgoing {
                    self.client.mark_read(message.chat_id, vec![message.id]);
                }
                self.messages.push(message);
                self.scroller.update(cx, |scroller, cx| _ = scroller.append(1, cx));
            }
            Event::MessageSent { old_id, message } => {
                if let Some(slot) = self.messages.iter_mut().find(|m| m.id == old_id) {
                    *slot = message;
                }
            }
            Event::DownloadProgress { file_id, progress } => _ = self.download_progress.insert(file_id, progress),
            Event::Photo { file_id, image } => {
                self.download_progress.remove(&file_id);
                match render_image(image) {
                    Ok(image) => _ = self.photos.insert(file_id, image),
                    Err(error) => self.show_error(&format!("{error:#}")),
                }
            }
            Event::Profile(profile) => {
                if let Some(photo) = &profile.photo {
                    self.request_photo(photo.file_id);
                }
                self.profiles.insert(profile.user_id, profile);
            }
            Event::AccountInfo { user_id, info } => _ = self.account_info.insert(user_id, info),
            Event::Saved { file_id, path } => {
                self.download_progress.remove(&file_id);
                let note = Notification::new().message(format!("Saved to {path}")).placement(Anchor::BottomLeft);
                window.push_notification(note, cx);
            }
            Event::RecentChats(chats) => {
                for chat_id in chats {
                    if !self.recent.contains(&chat_id) {
                        self.recent.push(chat_id);
                    }
                }
            }
            Event::DeleteOptions { chat_id, message_id, options } => {
                if let Some(deleting) =
                    self.deleting.as_mut().filter(|d| d.chat_id == chat_id && d.message_id == message_id)
                {
                    deleting.options = Some(options);
                }
            }
            Event::MessagesDeleted { chat_id, message_ids } if self.open_chat == Some(chat_id) => {
                for id in message_ids {
                    if let Some(ix) = self.messages.iter().position(|m| m.id == id) {
                        self.messages.remove(ix);
                        self.scroller.update(cx, |scroller, cx| _ = scroller.splice(ix..ix + 1, 0, cx));
                    }
                }
            }
            Event::Account { user_id, phone } => self.account = Some((user_id, phone)),
            Event::Error(error) => self.error = Some(error),
            Event::History { .. }
            | Event::OlderHistory { .. }
            | Event::Topics { .. }
            | Event::Topic { .. }
            | Event::Reply { .. }
            | Event::Message(_)
            | Event::MessagesDeleted { .. } => {}
        }
        cx.notify();
    }

    fn set_auth(&mut self, step: AuthStep, window: &mut Window, cx: &mut Context<Self>) {
        let (placeholder, masked) = match &step {
            AuthStep::Closed => {
                *self = Self::new(self.client.clone(), self.events.clone(), window, cx);
                return;
            }
            AuthStep::Qr { link } => {
                match qr_code(link) {
                    Ok(image) => self.qr_code = Some(image),
                    Err(error) => self.show_error(&format!("{error:#}")),
                }
                (String::new(), false)
            }
            AuthStep::ApiCredentials => ("API ID".to_string(), false),
            AuthStep::Phone => ("Phone number, e.g. +15550100".to_string(), false),
            AuthStep::Code => ("Login code".to_string(), false),
            AuthStep::Password { hint } => (format!("Password (hint: {hint})"), true),
            AuthStep::Ready => (String::new(), false),
        };
        self.login.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(placeholder, window, cx);
            input.set_masked(masked, window, cx);
        });
        self.api_hash.update(cx, |input, cx| input.set_value("", window, cx));
        self.auth = Some(step);
    }

    fn submit_login(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let value = self.login.read(cx).value().to_string();
        match &self.auth {
            Some(AuthStep::ApiCredentials) => {
                self.client.submit_api_credentials(value, self.api_hash.read(cx).value().to_string())
            }
            Some(AuthStep::Phone) => self.client.submit_phone(value.trim().into()),
            Some(AuthStep::Code) => self.client.submit_code(value.trim().into()),
            Some(AuthStep::Password { .. }) => self.client.submit_password(value),
            Some(AuthStep::Qr { .. } | AuthStep::Ready | AuthStep::Closed) | None => {}
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat_id) = self.open_chat else { return };
        let text = self.compose.read(cx).value().trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        if !is_allowed(&text) {
            self.show_error(&format!("Not sent, characters outside the allowlist: {text}"));
            cx.notify();
            return;
        }
        match self.attachments.is_empty() {
            true => self.client.send_text(chat_id, self.topic, text),
            false => self.client.send_photos(chat_id, self.topic, std::mem::take(&mut self.attachments), text),
        }
        self.compose.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub(crate) fn open(&mut self, chat_id: i64, cx: &mut Context<Self>) {
        self.open_chat = Some(chat_id);
        self.recent.retain(|&id| id != chat_id);
        self.recent.insert(0, chat_id);
        self.attachments.clear();
        self.deleting = None;
        self.topics.clear();
        self.clear_history(None, cx);
        if self.chats.get(&chat_id).is_some_and(|chat| chat.kind == ChatKind::Forum) {
            self.client.load_topics(chat_id);
        } else {
            self.client.open_chat(chat_id, None);
        }
    }

    pub(crate) fn select_topic(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(chat_id) = self.open_chat else { return };
        let topic = Some(self.topics[ix].id);
        self.clear_history(topic, cx);
        self.client.open_chat(chat_id, topic);
    }

    fn clear_history(&mut self, topic: Option<i32>, cx: &mut Context<Self>) {
        self.topic = topic;
        self.messages.clear();
        self.older = Older::Available;
        self.replies.clear();
        self.scroller.update(cx, |scroller, cx| scroller.reset(0, cx));
        cx.notify();
    }

    /// Requests older messages once the reader has scrolled up to within [`PREFETCH_ROWS`] of the oldest.
    fn prefetch(&mut self, cx: &mut Context<Self>) {
        let scroller = self.scroller.read(cx);
        let near_top = scroller.is_scrolled_up() && scroller.logical_scroll_top().item_ix < PREFETCH_ROWS;
        if let (true, Older::Available, Some(chat_id), Some(oldest)) =
            (near_top, self.older, self.open_chat, self.messages.first())
        {
            self.older = Older::Loading;
            self.client.load_older(chat_id, self.topic, oldest.id);
        }
    }

    fn show_error(&mut self, message: &str) {
        tracing::error!("{message}");
        self.error = Some(sanitize(message));
    }

    /// Requests the photos on screen, and those the scroll velocity brings on screen within [`PHOTO_LOOKAHEAD`].
    fn load_photos_in_view(&mut self, cx: &mut Context<Self>) {
        let Some((first, last)) = self.visible_rows.get() else { return };
        let now = Instant::now();
        self.scroll_samples.retain(|&(time, _)| now - time < VELOCITY_WINDOW);
        self.scroll_samples.push_back((now, first));
        let (since, from) = self.scroll_samples[0];
        let rows_per_second = match (now - since).as_secs_f32() {
            0. => 0.,
            seconds => (first as f32 - from as f32) / seconds,
        };
        let ahead = ((rows_per_second.abs() * PHOTO_LOOKAHEAD.as_secs_f32()).ceil() as usize).min(PHOTO_LOOKAHEAD_ROWS);
        let rows = match rows_per_second < 0. {
            true => first.saturating_sub(ahead)..last + 1,
            false => first..last + 1 + ahead,
        };
        let rows = rows.start.min(self.messages.len())..rows.end.min(self.messages.len());
        let photos: Vec<_> = self.messages[rows]
            .iter()
            .filter_map(|message| match &message.content {
                SafeContent::Photo { photo, .. } => Some(photo.file_id),
                _ => None,
            })
            .collect();
        let mut requested = false;
        for file_id in photos {
            requested |= self.request_photo(file_id);
        }
        if requested {
            cx.notify();
        }
    }

    /// Starts downloading a photo unless it was requested before; returns whether it did.
    fn request_photo(&mut self, file_id: i32) -> bool {
        let new = self.requested_photos.insert(file_id);
        if new {
            self.client.load_photo(file_id);
        }
        new
    }

    pub(crate) fn open_overlay(&mut self, overlay: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        if let Overlay::Photo(viewer) = &overlay {
            self.request_photo(viewer.photo.file_id);
        }
        self.overlay = Some(overlay);
        self.overlay_focus.focus(window, cx);
        cx.notify();
    }

    fn close_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
        cx.notify();
    }

    /// Opens the chat switcher, or closes it when it is open.
    fn toggle_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_some() {
            return self.close_switcher(window, cx);
        }
        if self.auth == Some(AuthStep::Ready) {
            let switcher = cx.new(|cx| CommandState::new(window, cx));
            switcher.update(cx, |switcher, cx| switcher.focus(window, cx));
            self.switcher = Some(switcher);
            cx.notify();
        }
    }

    fn close_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = None;
        self.focus_compose(window, cx);
        cx.notify();
    }

    fn focus_compose(&self, window: &mut Window, cx: &mut App) {
        self.compose.focus_handle(cx).focus(window, cx);
    }

    pub(crate) fn ask_delete(&mut self, chat_id: i64, message_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.client.load_delete_options(chat_id, message_id);
        self.deleting = Some(Deleting { chat_id, message_id, options: None });
        self.overlay_focus.focus(window, cx);
        cx.notify();
    }

    fn close_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.deleting = None;
        self.focus_compose(window, cx);
        cx.notify();
    }

    fn delete(&mut self, for_everyone: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(deleting) = self.deleting.take() {
            self.client.delete_message(deleting.chat_id, deleting.message_id, for_everyone);
        }
        self.close_delete(window, cx);
    }

    fn jump_to(&mut self, chat_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = None;
        self.open(chat_id, cx);
        self.close_switcher(window, cx);
    }

    /// Chats whose name starts with `query`, then those that only contain it, ignoring case. Each part goes from the
    /// most recently opened, then in Telegram's order.
    pub(crate) fn switcher_chats(&self, query: &str) -> Vec<i64> {
        let query = query.trim().to_lowercase();
        let mut ranked: Vec<_> = self
            .chats
            .values()
            .filter_map(|chat| {
                let title = self.chat_title(chat).to_string().to_lowercase();
                if !title.contains(&query) {
                    return None;
                }
                let recent = self.recent.iter().position(|&id| id == chat.id);
                let order = self.positions.values().filter_map(|orders| orders.get(&chat.id)).max().copied();
                (recent.is_some() || order.is_some())
                    .then(|| (!title.starts_with(&query), recent.unwrap_or(usize::MAX), Reverse(order), chat.id))
            })
            .collect();
        ranked.sort_unstable();
        ranked.into_iter().map(|(.., id)| id).collect()
    }

    pub(crate) fn attach(&mut self, files: impl IntoIterator<Item = PathBuf>, cx: &mut Context<Self>) {
        self.attachments.extend(files);
        cx.notify();
    }

    fn pick_photos(&mut self, cx: &mut Context<Self>) {
        let options =
            PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) };
        let picked = cx.prompt_for_paths(options);
        cx.spawn(async move |this, cx| {
            let picked = picked.await.map_err(anyhow::Error::from).and_then(|picked| picked);
            this.update(cx, |this, cx| match picked {
                Ok(files) => this.attach(files.into_iter().flatten(), cx),
                Err(error) => this.show_error(&format!("cannot pick photos: {error:#}")),
            })
        })
        .detach();
    }

    fn save_file(&mut self, file_id: i32, name: String, cx: &mut Context<Self>) {
        self.download_progress.insert(file_id, 0.);
        self.client.save_file(file_id, name);
        cx.notify();
    }

    /// Opens the profile of `user_id`, or closes it when it is the one already open.
    pub(crate) fn toggle_profile(&mut self, user_id: i64, cx: &mut Context<Self>) {
        self.profile = (self.profile != Some(user_id)).then_some(user_id);
        if self.profile.is_some() {
            self.client.load_profile(user_id);
        }
        cx.notify();
    }

    /// Contacts whose name or handle looks like the name or handle of `user_id`, when that user is not a contact.
    pub(crate) fn lookalikes(&self, user_id: i64) -> Vec<&SafeUser> {
        let Some(user) = self.users.get(&user_id).filter(|user| !user.is_contact) else { return Vec::new() };
        let skeletons = |u: &SafeUser| -> Vec<String> {
            [Some(&u.name), u.username.as_ref()]
                .into_iter()
                .flatten()
                .map(|text| skeleton(&text.to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        };
        let suspect = skeletons(user);
        self.users.values().filter(|c| c.is_contact && skeletons(c).iter().any(|s| suspect.contains(s))).collect()
    }

    fn folder(&self) -> ChatListId {
        self.folders[self.folder_ix].0
    }

    pub(crate) fn select_folder(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.folder_ix = ix;
        self.sort_chats();
        cx.notify();
    }

    fn sort_chats(&mut self) {
        let orders = self.positions.get(&self.folder());
        let mut chats: Vec<_> = orders.into_iter().flatten().map(|(&id, &order)| (Reverse(order), id)).collect();
        chats.sort_unstable();
        self.chat_list = chats.into_iter().map(|(_, id)| id).collect();
    }

    /// Telegram titles the chat with yourself with your name; its apps call it Saved Messages.
    fn chat_title(&self, chat: &SafeChat) -> SafeText {
        match chat.kind {
            ChatKind::Private { user_id } if self.is_me(user_id) => sanitize("Saved Messages"),
            _ => chat.title.clone(),
        }
    }

    fn is_me(&self, user_id: i64) -> bool {
        self.account.as_ref().is_some_and(|(me, _)| *me == user_id)
    }

    fn chat_note(&self, chat: &SafeChat) -> String {
        match chat.kind {
            ChatKind::Private { user_id } if self.is_me(user_id) => "only you".to_string(),
            ChatKind::Group => "group".to_string(),
            ChatKind::Forum => "group with topics".to_string(),
            ChatKind::Private { user_id } => self.person_note(user_id),
            ChatKind::Secret { user_id } => format!("secret · {}", self.person_note(user_id)),
        }
    }

    fn person_note(&self, user_id: i64) -> String {
        let Some(user) = self.users.get(&user_id) else { return format!("id {user_id}") };
        let handle = user.username.as_ref().map_or_else(|| format!("id {user_id}"), |name| format!("@{name}"));
        let bot = if user.is_bot { " · bot" } else { "" };
        let contact = if user.is_contact { "" } else { " · not a contact" };
        format!("{handle}{bot}{contact}")
    }

    fn render_login(&self, cx: &mut Context<Self>) -> Div {
        let title = match self.auth {
            None | Some(AuthStep::Closed) => "Connecting...",
            Some(AuthStep::ApiCredentials) => "Register plaingram with Telegram",
            Some(AuthStep::Qr { .. }) => "Scan From Mobile Telegram",
            _ => "Sign in",
        };
        let setup = matches!(self.auth, Some(AuthStep::ApiCredentials));
        let qr = matches!(self.auth, Some(AuthStep::Qr { .. }));
        let asking = setup || matches!(self.auth, Some(AuthStep::Phone | AuthStep::Code | AuthStep::Password { .. }));
        let input = |state: &Entity<InputState>| div().w(px(320.)).child(Input::new(state));
        let steps = |steps: &[&'static str]| {
            v_flex()
                .gap_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .children(steps.iter().map(|s| div().child(*s)))
        };
        let switch = |id: &'static str, label: &'static str, log_in: fn(&dyn Backend)| {
            Button::new(id).link().label(label).on_click(cx.listener(move |this, _, _, _| log_in(&*this.client)))
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .when(qr, |el| {
                el.children(self.qr_code.clone().map(|code| img(ImageSource::Render(code)).size(px(QR_SIZE))))
            })
            .child(div().text_xl().child(title))
            .when(setup, |el| el.child(steps(&API_SETUP_STEPS)))
            .when(qr, |el| {
                el.child(steps(&QR_STEPS))
                    .child(switch("use-phone", "Log in using phone number", |c| c.log_in_with_phone()))
            })
            .when(asking, |el| el.child(input(&self.login)))
            .when(setup, |el| el.child(input(&self.api_hash)))
            .when(asking, |el| {
                el.child(
                    Button::new("continue")
                        .primary()
                        .label("Continue")
                        .on_click(cx.listener(|this, _, window, cx| this.submit_login(window, cx))),
                )
            })
            .when(matches!(self.auth, Some(AuthStep::Phone | AuthStep::Code)), |el| {
                el.child(switch("use-qr", "Log in using QR code", |c| c.log_in_with_qr()))
            })
    }

    fn render_chats(&self, cx: &mut Context<Self>) -> Div {
        let rows = cx.processor(|this, range: Range<usize>, _, cx| {
            range.map(|ix| this.render_chat_row(ix, cx)).collect::<Vec<_>>()
        });
        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(tab_bar(
                "folders",
                self.folder_ix,
                self.folders.iter().map(|(_, name)| name.clone()),
                cx.listener(|this, ix: &usize, _, cx| this.select_folder(*ix, cx)),
            ))
            .child(uniform_list("chats", self.chat_list.len(), rows).size_full())
    }

    fn render_chat_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let chat = &self.chats[&self.chat_list[ix]];
        let id = chat.id;
        v_flex()
            .id(ix)
            .w_full()
            .h(px(CHAT_ROW_HEIGHT))
            .px_3()
            .justify_center()
            .cursor_pointer()
            .when(self.open_chat == Some(id), |el| el.bg(cx.theme().list_active))
            .hover(|style| style.bg(cx.theme().list_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open(id, cx)))
            .child(chat_label(&self.chat_title(chat), self.chat_note(chat), cx))
    }

    fn render_conversation(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self.open_chat.and_then(|id| self.chats.get(&id)) else {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Select a chat")
                .into_any_element();
        };
        let app = cx.entity();
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().secondary))
            .on_drop(
                cx.listener(|this, dropped: &ExternalPaths, _, cx| this.attach(dropped.paths().iter().cloned(), cx)),
            )
            .child({
                let header = h_flex().id("chat-header").p_3().border_b_1().border_color(cx.theme().border);
                let header = match chat.kind {
                    ChatKind::Private { user_id } | ChatKind::Secret { user_id } => header
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_profile(user_id, cx))),
                    ChatKind::Group | ChatKind::Forum => header,
                };
                header.child(name_label(&self.chat_title(chat), cx))
            })
            .when(!self.topics.is_empty(), |el| {
                let selected = self.topics.iter().position(|t| Some(t.id) == self.topic).unwrap_or_default();
                el.child(tab_bar(
                    "topics",
                    selected,
                    self.topics.iter().map(|t| t.name.to_string().into()),
                    cx.listener(|this, ix: &usize, _, cx| this.select_topic(*ix, cx)),
                ))
            })
            .child(
                MessageScroller::new("messages", self.scroller.clone(), {
                    let visible_rows = self.visible_rows.clone();
                    let app = app.clone();
                    move |ix, _, cx| {
                        let visible_rows = visible_rows.clone();
                        let on_screen = canvas(
                            move |bounds, window, _| {
                                if window.content_mask().bounds.intersects(&bounds) {
                                    let (first, last) = visible_rows.get().unwrap_or((ix, ix));
                                    visible_rows.set(Some((first.min(ix), last.max(ix))));
                                }
                            },
                            |_, _, _, _| {},
                        );
                        div()
                            .relative()
                            .child(app.read(cx).render_message(ix, &app, cx))
                            .child(on_screen.absolute().size_full())
                    }
                })
                .with_jump_button_renderer(|button| button.label("Latest"))
                .flex_1(),
            )
            .when(!self.attachments.is_empty(), |el| el.child(self.render_attachments(cx)))
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(Button::new("attach").ghost().label("+").dropdown_menu_with_anchor(Anchor::BottomLeft, {
                        move |menu, _, _| {
                            let app = app.clone();
                            menu.item(PopupMenuItem::new("Add image").on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| this.pick_photos(cx));
                            }))
                        }
                    }))
                    .child(Input::new(&self.compose).flex_1())
                    .child(
                        Button::new("send")
                            .primary()
                            .label("Send")
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .into_any_element()
    }

    /// Previews of the photos going with the next message, each with 'x' to leave it out.
    fn render_attachments(&self, cx: &mut Context<Self>) -> Div {
        h_flex().px_2().pt_2().gap_2().border_t_1().border_color(cx.theme().border).children(
            self.attachments.iter().enumerate().map(|(ix, file)| {
                let name = sanitize(&file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
                v_flex()
                    .w(px(ATTACHMENT_SIZE))
                    .gap_1()
                    .text_xs()
                    .child(
                        img(file.clone())
                            .size(px(ATTACHMENT_SIZE))
                            .object_fit(ObjectFit::Contain)
                            .with_fallback(|| div().child("no preview").into_any_element()),
                    )
                    .child(h_flex().child(div().flex_1().min_w_0().truncate().child(safe_text(&name, cx))).child(
                        Button::new(("remove-attachment", ix)).ghost().label("x").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.attachments.remove(ix);
                                cx.notify();
                            },
                        )),
                    ))
            }),
        )
    }

    fn render_message(&self, ix: usize, app: &Entity<Self>, cx: &App) -> AnyElement {
        let Some(message) = self.messages.get(ix) else { return div().into_any_element() };
        let bubble = if message.outgoing { cx.theme().accent } else { cx.theme().secondary };
        let (download, chat_id, message_id, menu_app) =
            (download(&message.content, message.date, app), message.chat_id, message.id, app.clone());
        v_flex()
            .px_4()
            .py_1()
            .map(|el| if message.outgoing { el.items_end() } else { el.items_start() })
            .child(self.render_sender(ix, message, app, cx))
            .child(
                v_flex()
                    .id(("message", message.id.unsigned_abs() as usize))
                    .max_w(px(520.))
                    .p_2()
                    .gap_2()
                    .bg(bubble)
                    .when_some(self.replies.get(&message.id), |el, reply| el.child(self.render_reply(reply, cx)))
                    .child(self.render_content(&message.content, Some((app, message.date)), cx))
                    .context_menu(move |menu, _, _| {
                        let menu = match download.clone() {
                            Some(download) => {
                                menu.item(PopupMenuItem::new("Download").on_click(move |_, _, cx| download(cx)))
                            }
                            None => menu,
                        };
                        let app = menu_app.clone();
                        menu.item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| this.ask_delete(chat_id, message_id, window, cx))
                        }))
                    }),
            )
            .into_any_element()
    }

    /// `message` is the app and the message date, which make photos open on click; reply quotes pass `None` and show
    /// photos as a label.
    fn render_content(&self, content: &SafeContent, message: Option<(&Entity<Self>, i32)>, cx: &App) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        match content {
            SafeContent::Text(text) => safe_text(text, cx).into_any_element(),
            SafeContent::Photo { photo, caption } => v_flex()
                .gap_1()
                .child(match message {
                    None => div().text_color(muted).child("[photo]").into_any_element(),
                    Some((app, date)) => {
                        let viewer = Viewer { photo: *photo, save_as: Some(photo_name(date)) };
                        let app = app.clone();
                        div()
                            .id(("photo", photo.file_id.unsigned_abs() as usize))
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                app.update(cx, |this, cx| this.open_overlay(Overlay::Photo(viewer.clone()), window, cx))
                            })
                            .child(self.render_photo(photo, PHOTO_MAX_WIDTH, cx))
                            .into_any_element()
                    }
                })
                .when(!caption.is_empty(), |el| el.child(safe_text(caption, cx)))
                .into_any_element(),
            SafeContent::Service(text) => {
                div().italic().text_color(muted).child(safe_text(text, cx)).into_any_element()
            }
            SafeContent::Hidden(kind) => div().text_color(muted).child(format!("[hidden: {kind}]")).into_any_element(),
            SafeContent::File { kind, file } => {
                let label = h_flex()
                    .gap_1()
                    .text_color(muted)
                    .child(format!("[hidden: {kind}]"))
                    .child(safe_text(&file.name, cx))
                    .child(format!("· {}", size_label(file.size)))
                    .when_some(self.download_progress.get(&file.file_id), |el, progress| {
                        el.child(format!("· saving {:.0}%", progress * 100.))
                    });
                label.into_any_element()
            }
        }
    }

    fn render_reply(&self, reply: &SafeReply, cx: &App) -> Div {
        let name = reply.sender.and_then(|sender| self.sender_name(sender));
        v_flex()
            .pl_2()
            .border_l_2()
            .border_color(cx.theme().foreground)
            .text_sm()
            .children(name.map(|name| name_label(name, cx)))
            .child(div().line_clamp(2).text_color(cx.theme().muted_foreground).child(self.render_content(
                &reply.content,
                None,
                cx,
            )))
    }

    fn sender_name(&self, sender: Sender) -> Option<&SafeText> {
        match sender {
            Sender::User(id) => self.users.get(&id).map(|u| &u.name),
            Sender::Chat(id) => self.chats.get(&id).map(|c| &c.title),
        }
    }

    /// Clicking a person's name opens their profile.
    fn render_sender(&self, ix: usize, message: &SafeMessage, app: &Entity<Self>, cx: &App) -> Div {
        let time = timestamp(message.date, Local::now());
        let row = h_flex().gap_2().text_xs();
        let muted = |text: String| div().text_color(cx.theme().muted_foreground).child(text);
        if message.outgoing {
            return row.child("You").child(muted(time));
        }
        let name = self.sender_name(message.sender).map(|name| name_label(name, cx));
        match message.sender {
            Sender::User(user_id) => {
                let app = app.clone();
                row.child(
                    h_flex()
                        .id(("sender", ix))
                        .gap_2()
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(move |_, _, cx| app.update(cx, |this, cx| this.toggle_profile(user_id, cx)))
                        .children(name)
                        .child(muted(format!("{} · {time}", self.person_note(user_id)))),
                )
            }
            Sender::Chat(chat_id) => row.children(name).child(muted(format!("chat {chat_id} · {time}"))),
        }
    }

    /// Who someone is and how established their account looks, beside the chat.
    fn render_profile(&self, user_id: i64, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let (user, profile, info) =
            (self.users.get(&user_id), self.profiles.get(&user_id), self.account_info.get(&user_id));
        let now = Local::now();
        let unknown = || "unknown".to_string();
        let loading = || "loading...".to_string();
        let photo_changed = info.and_then(|i| i.photo_changed).or(profile.and_then(|p| p.photo_dates.first().copied()));
        let facts = [
            (
                "Registered",
                info.and_then(|i| i.registered).map_or_else(unknown, |(month, year)| month_year(month, year)),
            ),
            (
                "Phone country",
                info.map(|i| i.phone_country.to_string()).filter(|c| !c.is_empty()).unwrap_or_else(unknown),
            ),
            ("Renamed", info.and_then(|i| i.name_changed).map_or_else(unknown, |date| age(date, now))),
            ("Photo changed", photo_changed.map_or_else(unknown, |date| age(date, now))),
            (
                "Profile photos",
                profile.map_or_else(loading, |p| match p.photo_dates.last() {
                    Some(&first) => format!("{}, first {}", p.photo_count, age(first, now)),
                    None => "none".into(),
                }),
            ),
            ("Groups in common", profile.map_or_else(loading, |p| p.groups_in_common.to_string())),
            ("User ID", user_id.to_string()),
        ];
        let (alarms, labels): (Vec<_>, Vec<_>) = profile
            .map(|p| p.labels.as_slice())
            .unwrap_or_default()
            .iter()
            .partition(|l| matches!(**l, "SCAM" | "FAKE"));
        let status = user.map(|u| if u.is_contact { "contact" } else { "not a contact" }).into_iter();
        v_flex()
            .id("profile")
            .overflow_y_scroll()
            .w(px(PROFILE_WIDTH))
            .flex_none()
            .h_full()
            .p_3()
            .gap_3()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                h_flex().justify_between().text_color(muted).child("Profile").child(
                    Button::new("close-profile").label("Close").on_click(cx.listener(|this, _, _, cx| {
                        this.profile = None;
                        cx.notify();
                    })),
                ),
            )
            .when_some(profile.and_then(|p| p.photo), |el, photo| {
                el.child(
                    div()
                        .id("profile-photo")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_overlay(Overlay::Photo(Viewer { photo, save_as: None }), window, cx)
                        }))
                        .child(self.render_photo(&photo, PROFILE_PHOTO_WIDTH, cx)),
                )
            })
            .children(user.map(|user| identity(user, cx)))
            .child(div().text_xs().text_color(muted).child(status.chain(labels.into_iter().copied()).collect::<Vec<_>>().join(" · ")))
            .children(alarms.into_iter().map(|label| warning(format!("Telegram marks this account {label}"), cx)))
            .children(self.lookalikes(user_id).into_iter().map(|contact| {
                let handle = contact.username.as_ref().map_or_else(|| format!("id {}", contact.id), |u| format!("@{u}"));
                warning(format!("looks like your contact {} ({handle})", contact.name), cx)
            }))
            .when_some(profile.filter(|p| !p.bio.is_empty()), |el, p| el.child(safe_text(&p.bio, cx)))
            .children(facts.into_iter().map(|(label, value)| fact(label, value, muted)))
            .when(info.is_none(), |el| {
                el.child(div().text_xs().text_color(muted).child(
                    "Telegram shares registration date, phone country and renames only for people who message you first.",
                ))
            })
    }

    /// Covers the window below the title bar until Close or Escape. Chat photos also get Download.
    fn render_overlay(&self, overlay: &Overlay, cx: &mut Context<Self>) -> Stateful<Div> {
        let (download, content) = match overlay {
            Overlay::Photo(viewer) => {
                let photo = viewer.photo;
                let download = viewer.save_as.clone().map(|name| {
                    Button::new("download-photo")
                        .label("Download")
                        .on_click(cx.listener(move |this, _, _, _| this.client.save_photo(photo.file_id, name.clone())))
                });
                let image = match self.photos.get(&photo.file_id) {
                    Some(image) => img(ImageSource::Render(image.clone()))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                    None => self.render_photo(&photo, PHOTO_MAX_WIDTH, cx),
                };
                (download, div().flex_1().min_h_0().p_4().flex().items_center().justify_center().child(image))
            }
            Overlay::Settings { confirm_log_out } => (None, self.render_settings(*confirm_log_out, cx)),
        };
        div()
            .id("overlay")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .track_focus(&self.overlay_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_overlay(cx);
                }
            }))
            .child(
                h_flex().p_2().gap_2().justify_end().children(download).child(
                    Button::new("close-overlay")
                        .label("Close")
                        .on_click(cx.listener(|this, _, _, cx| this.close_overlay(cx))),
                ),
            )
            .child(content)
    }

    fn render_settings(&self, confirm_log_out: bool, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let settings = |confirm_log_out| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                this.overlay = Some(Overlay::Settings { confirm_log_out });
                cx.notify();
            })
        };
        let log_out = match confirm_log_out {
            false => h_flex()
                .child(Button::new("log-out").danger().label("Log out").on_click(settings(true)))
                .into_any_element(),
            true => v_flex()
                .gap_2()
                .child("Log out? plaingram deletes its copy of your chats from this computer. They stay on Telegram.")
                .child(
                    h_flex()
                        .gap_2()
                        .child(Button::new("confirm-log-out").danger().label("Log out").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.client.log_out();
                                this.close_overlay(cx);
                            },
                        )))
                        .child(Button::new("cancel-log-out").label("Cancel").on_click(settings(false))),
                )
                .into_any_element(),
        };
        let account = self.account.as_ref();
        v_flex().flex_1().min_h_0().items_center().p_4().child(
            v_flex()
                .w(px(PROFILE_WIDTH))
                .gap_3()
                .child(div().text_xl().child("Settings"))
                .children(account.and_then(|(user_id, _)| self.users.get(user_id)).map(|user| identity(user, cx)))
                .child(fact(
                    "Phone",
                    account.map_or_else(|| "loading...".into(), |(_, phone)| phone.to_string()),
                    muted,
                ))
                .child(fact("Version", env!("CARGO_PKG_VERSION").into(), muted))
                .child(log_out),
        )
    }

    /// Type to filter; arrows and Enter or a click jump to a chat; Escape or a click outside closes.
    fn render_switcher(&self, switcher: &Entity<CommandState>, cx: &mut Context<Self>) -> Div {
        let chats = Rc::new(self.switcher_chats(&switcher.read(cx).query(cx)));
        let items: Vec<_> = chats
            .iter()
            .map(|id| {
                let chat = &self.chats[id];
                let (title, note) = (self.chat_title(chat), self.chat_note(chat));
                CommandItem::new().child(move |_, cx| chat_label(&title, note.clone(), cx))
            })
            .collect();
        let app = cx.entity();
        let command = Command::new(switcher)
            .filterable(false)
            .placeholder("Jump to a chat")
            .max_h(px(SWITCHER_LIST_HEIGHT))
            .items(items)
            .on_query({
                let app = app.clone();
                move |_, _, cx| app.update(cx, |_, cx| cx.notify())
            })
            .on_confirm(move |ix, window, cx| app.update(cx, |this, cx| this.jump_to(chats[ix.row], window, cx)));
        let switcher = div()
            .w(px(SWITCHER_WIDTH))
            .capture_action(cx.listener(|this, _: &Cancel, window, cx| {
                this.close_switcher(window, cx);
                cx.stop_propagation();
            }))
            .child(command);
        modal(switcher, Self::close_switcher, cx)
    }

    /// Asks how to delete a message, offering only what Telegram allows for it.
    fn render_delete(&self, deleting: &Deleting, cx: &mut Context<Self>) -> Div {
        let options = deleting.options;
        let choice = |id: &'static str, label: &'static str, for_everyone: bool| {
            Button::new(id)
                .danger()
                .label(label)
                .on_click(cx.listener(move |this, _, window, cx| this.delete(for_everyone, window, cx)))
        };
        let question = match options {
            Some(DeleteOptions { for_me: false, for_everyone: false }) => {
                "Telegram does not allow deleting this message."
            }
            _ => "Delete this message?",
        };
        let dialog = v_flex()
            .id("delete-message")
            .w(px(DELETE_WIDTH))
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .track_focus(&self.overlay_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_delete(window, cx);
                }
            }))
            .child(question)
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .when(options.is_some_and(|o| o.for_everyone), |el| {
                        el.child(choice("delete-for-everyone", "Delete for everyone", true))
                    })
                    .when(options.is_some_and(|o| o.for_me), |el| {
                        el.child(choice("delete-for-me", "Delete for me", false))
                    })
                    .child(
                        Button::new("cancel-delete")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| this.close_delete(window, cx))),
                    ),
            );
        modal(dialog, Self::close_delete, cx)
    }

    fn render_photo(&self, photo: &PhotoRef, max_width: f32, cx: &App) -> AnyElement {
        let scale = (max_width / photo.width as f32).min(1.);
        let (width, height) = (px(photo.width as f32 * scale), px(photo.height as f32 * scale));
        match self.photos.get(&photo.file_id) {
            Some(image) => img(ImageSource::Render(image.clone())).w(width).h(height).into_any_element(),
            None => div()
                .relative()
                .w(width)
                .h(height)
                .bg(cx.theme().muted)
                .when(self.requested_photos.contains(&photo.file_id), |el| {
                    let progress = self.download_progress.get(&photo.file_id).copied().unwrap_or_default();
                    el.child(
                        h_flex()
                            .absolute()
                            .bottom_2()
                            .left_2()
                            .right_2()
                            .gap_2()
                            .items_center()
                            .text_xs()
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(4.))
                                    .bg(cx.theme().background)
                                    .child(div().h_full().w(relative(progress)).bg(cx.theme().foreground)),
                            )
                            .child(format!("{:.0}%", progress * 100.)),
                    )
                })
                .into_any_element(),
        }
    }
}

impl Render for Plaingram {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_rows.set(None);
        cx.defer_in(window, |this, _, cx| this.load_photos_in_view(cx));
        let body = match self.auth {
            Some(AuthStep::Ready) => h_flex()
                .flex_1()
                .min_h_0()
                .child(self.render_chats(cx))
                .child(self.render_conversation(cx))
                .when_some(self.profile, |el, user_id| el.child(self.render_profile(user_id, cx)))
                .into_any_element(),
            _ => self.render_login(cx).into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .child(
                TitleBar::new().child(div().text_sm().text_color(cx.theme().muted_foreground).child("plaingram")).when(
                    self.auth == Some(AuthStep::Ready),
                    |el| {
                        el.child(div().pr_2().child(Button::new("settings").ghost().label("Settings").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.open_overlay(Overlay::Settings { confirm_log_out: false }, window, cx)
                            }),
                        )))
                    },
                ),
            )
            .when_some(self.error.as_ref(), |el, error| {
                el.child(
                    h_flex()
                        .p_2()
                        .gap_2()
                        .bg(cx.theme().danger)
                        .text_color(cx.theme().danger_foreground)
                        .child(div().flex_1().child(safe_text(error, cx)))
                        .child(Button::new("dismiss").label("Dismiss").on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        }))),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(body)
                    .when_some(self.overlay.as_ref(), |el, overlay| el.child(self.render_overlay(overlay, cx)))
                    .when_some(self.deleting.as_ref(), |el, deleting| el.child(self.render_delete(deleting, cx)))
                    .when_some(self.switcher.as_ref(), |el, switcher| el.child(self.render_switcher(switcher, cx))),
            )
    }
}

/// Dims the window under `content`, which sits near its top; a click outside calls `close`.
fn modal(
    content: impl IntoElement,
    close: fn(&mut Plaingram, &mut Window, &mut Context<Plaingram>),
    cx: &Context<Plaingram>,
) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .justify_center()
        .items_start()
        .pt(px(MODAL_TOP))
        .bg(cx.theme().overlay)
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| close(this, window, cx)))
        .child(div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(content))
}

type Download = Rc<dyn Fn(&mut App)>;

/// What a message's Download does: photos are re-encoded, other files saved as sent.
fn download(content: &SafeContent, date: i32, app: &Entity<Plaingram>) -> Option<Download> {
    let app = app.clone();
    match content {
        SafeContent::Photo { photo, .. } => {
            let (file_id, name) = (photo.file_id, photo_name(date));
            Some(Rc::new(move |cx| app.read(cx).client.save_photo(file_id, name.clone())))
        }
        SafeContent::File { file, .. } => {
            let (file_id, name) = (file.file_id, file.name.to_string());
            Some(Rc::new(move |cx| app.update(cx, |this, cx| this.save_file(file_id, name.clone(), cx))))
        }
        SafeContent::Text(_) | SafeContent::Service(_) | SafeContent::Hidden(_) => None,
    }
}

/// A chat's name with its badges, over what kind of chat it is.
fn chat_label(title: &SafeText, note: String, cx: &App) -> Div {
    v_flex().child(name_label(title, cx)).child(div().text_xs().text_color(cx.theme().muted_foreground).child(note))
}

/// A name and @handle, each with its badges.
fn identity(user: &SafeUser, cx: &App) -> Div {
    v_flex()
        .gap_1()
        .child(name_label(&user.name, cx))
        .when_some(user.username.as_ref(), |el, handle| el.child(h_flex().child("@").child(name_label(handle, cx))))
}

/// A labelled value in the profile or settings.
fn fact(label: &'static str, value: String, muted: Hsla) -> Div {
    h_flex()
        .gap_2()
        .child(div().w(px(150.)).flex_none().text_color(muted).child(label))
        .child(div().flex_1().min_w_0().child(value))
}

fn qr_code(link: &str) -> Result<Arc<RenderImage>> {
    render_image(qr_pixels(link)?)
}

/// `link` as black modules on white, `2 * QR_SIZE` pixels square.
fn qr_pixels(link: &str) -> Result<DecodedImage> {
    let code = qrcode::QrCode::new(link)?;
    let (modules, colors) = (code.width(), code.to_colors());
    let (side, total) = (2 * QR_SIZE as usize, modules + 2 * QR_QUIET_ZONE);
    let in_code = QR_QUIET_ZONE..QR_QUIET_ZONE + modules;
    let dark = |x: usize, y: usize| {
        let (x, y) = (x * total / side, y * total / side);
        in_code.contains(&x)
            && in_code.contains(&y)
            && colors[(y - QR_QUIET_ZONE) * modules + x - QR_QUIET_ZONE] == qrcode::Color::Dark
    };
    let bgra =
        (0..side * side).flat_map(|i| if dark(i % side, i / side) { [0, 0, 0, 255] } else { [255; 4] }).collect();
    Ok(DecodedImage { width: u32::try_from(side)?, height: u32::try_from(side)?, bgra })
}

fn render_image(image: DecodedImage) -> Result<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(image.width, image.height, image.bgra)
        .context("decoded photo has the wrong buffer size")?;
    Ok(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

fn folder_tabs(folders: &[SafeFolder], main_position: usize) -> Vec<(ChatListId, SharedString)> {
    let mut tabs: Vec<_> = folders.iter().map(|f| (ChatListId::Folder(f.id), f.name.to_string().into())).collect();
    tabs.insert(main_position, (ChatListId::Main, "All chats".into()));
    tabs.push((ChatListId::Archive, "Archive".into()));
    tabs
}

fn tab_bar(
    id: &'static str,
    selected: usize,
    labels: impl Iterator<Item = SharedString>,
    on_click: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> TabBar {
    TabBar::new(id).underline().px_2().menu(true).selected_index(selected).on_click(on_click).children(labels)
}

/// Relative for the last 24 hours, then the local date and time.
fn timestamp(date: i32, now: DateTime<Local>) -> String {
    let age = now.timestamp() - i64::from(date);
    match age {
        ..60 => "just now".into(),
        60..3600 => format!("{}m ago", age / 60),
        3600..86400 => format!("{}h ago", age / 3600),
        _ => local(date).format("%Y-%m-%d %H:%M").to_string(),
    }
}

/// Like Telegram Desktop's names for saved photos: `photo_2026-11-16_23-52-12.jpg`.
fn photo_name(date: i32) -> String {
    format!("photo_{}.jpg", local(date).format("%Y-%m-%d_%H-%M-%S"))
}

/// `2.4 MB`, in powers of 1000 like macOS Finder.
fn size_label(bytes: i64) -> String {
    match bytes {
        ..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} KB", bytes as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", bytes as f64 / 1e6),
        _ => format!("{:.1} GB", bytes as f64 / 1e9),
    }
}

/// A past date and how many days ago it was, for spotting new accounts and fresh renames.
fn age(date: i32, now: DateTime<Local>) -> String {
    match (now.timestamp() - i64::from(date)) / 86400 {
        ..1 => timestamp(date, now),
        1 => format!("{} (1 day ago)", local(date).format("%Y-%m-%d")),
        days => format!("{} ({days} days ago)", local(date).format("%Y-%m-%d")),
    }
}

/// Folds case, punctuation and look-alike characters, so "a1ice_smith_" and "Alice Smith" compare equal.
fn skeleton(text: &str) -> String {
    let text = text.to_lowercase().replace("rn", "m");
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| match c {
            '0' => 'o',
            '1' | 'i' => 'l',
            c => c,
        })
        .collect()
}

fn month_year(month: u32, year: i32) -> String {
    NaiveDate::from_ymd_opt(year, month, 1).expect("month is 1 to 12").format("%B %Y").to_string()
}

fn local(date: i32) -> DateTime<Local> {
    DateTime::from_timestamp(date.into(), 0).expect("every i32 timestamp is in range").with_timezone(&Local)
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone as _};

    use super::{qr_pixels, skeleton, timestamp};

    /// Decodes the drawn code the way a phone camera would, so a wrong module size or quiet zone shows up here.
    #[test]
    fn qr_codes_scan_back_to_the_login_link() {
        let link = "tg://login?token=AQKm8fNnZrbRgP0x3sGg9lHn2vYq7kHbXzJc1tW4eQdUoLsA-_";
        let image = qr_pixels(link).unwrap();
        let (width, height) = (image.width as usize, image.height as usize);
        let mut prepared =
            rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| image.bgra[(y * width + x) * 4]);
        let grids = prepared.detect_grids();
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].decode().unwrap().1, link);
    }

    #[test]
    fn timestamps_are_relative_for_a_day_then_absolute() {
        const NOW: i32 = 1_800_000_000;
        let ago = |secs: i32| timestamp(NOW - secs, Local.timestamp_opt(NOW.into(), 0).unwrap());
        assert_eq!(ago(-5), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1m ago");
        assert_eq!(ago(3599), "59m ago");
        assert_eq!(ago(3600), "1h ago");
        assert_eq!(ago(86399), "23h ago");
        let absolute = ago(86400);
        assert!(absolute.starts_with("2027-01-1") && absolute.len() == 16, "{absolute}");
    }

    #[test]
    fn skeletons_fold_look_alike_names_and_handles() {
        assert_eq!(skeleton("Alice Smith"), skeleton("alice_smith_"));
        assert_eq!(skeleton("a1ice.smith"), skeleton("AIice Smith"));
        assert_eq!(skeleton("B0b Jones"), skeleton("bobjones"));
        assert_eq!(skeleton("Mike"), skeleton("rnike"));
        assert_ne!(skeleton("Alice Smith"), skeleton("Alice Smyth"));
        assert_eq!(skeleton("\u{1F60A} ???"), "");
    }
}
