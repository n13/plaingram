use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use tg_core::{
    AuthStep, Backend, ChatKind, ChatListId, DecodedImage, Event, Events, PhotoRef, SafeChat, SafeContent, SafeFolder,
    SafeMessage, SafeReply, SafeText, SafeTopic, SafeUser, Sender, is_allowed, sanitize,
};

use crate::safe_text::{name_label, safe_text};

const SIDEBAR_WIDTH: f32 = 300.;
const CHAT_ROW_HEIGHT: f32 = 56.;
const PHOTO_MAX_WIDTH: f32 = 360.;
const API_SETUP_STEPS: [&str; 4] = [
    "Telegram requires every app to have its own API ID. This is a one-time setup:",
    "1. Sign in at my.telegram.org and open API development tools.",
    "2. Create an app: title and short name plaingram, platform Desktop.",
    "3. Paste its api_id and api_hash below. They are stored in the macOS Keychain.",
];

pub struct Plaingram {
    client: Box<dyn Backend>,
    auth: Option<AuthStep>,
    login: Entity<InputState>,
    api_hash: Entity<InputState>,
    compose: Entity<InputState>,
    scroller: Entity<MessageScrollerState>,
    users: HashMap<i64, SafeUser>,
    chats: HashMap<i64, SafeChat>,
    positions: HashMap<ChatListId, HashMap<i64, i64>>,
    folders: Vec<(ChatListId, SharedString)>,
    folder_ix: usize,
    chat_list: Vec<i64>,
    open_chat: Option<i64>,
    /// Topics of the open chat if it is a forum.
    topics: Vec<SafeTopic>,
    topic: Option<i32>,
    messages: Vec<SafeMessage>,
    /// What each message in `messages` replies to, by message id.
    replies: HashMap<i64, SafeReply>,
    photos: HashMap<i32, Arc<RenderImage>>,
    requested_photos: HashSet<i32>,
    error: Option<SafeText>,
    _subscriptions: [Subscription; 3],
    _events: Task<()>,
}

impl Plaingram {
    pub fn new(client: Box<dyn Backend>, events: Events, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let login = cx.new(|cx| InputState::new(window, cx));
        let api_hash = cx.new(|cx| InputState::new(window, cx).placeholder("API hash").masked(true));
        let compose = cx.new(|cx| InputState::new(window, cx).placeholder("Message"));
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
        let subscriptions = [
            cx.subscribe_in(&login, window, on_enter(Self::submit_login)),
            cx.subscribe_in(&api_hash, window, on_enter(Self::submit_login)),
            cx.subscribe_in(&compose, window, on_enter(Self::send)),
        ];
        let events = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update_in(cx, |app, window, cx| app.apply(event, window, cx)).is_err() {
                    return;
                }
            }
        });
        Self {
            client,
            auth: None,
            login,
            api_hash,
            compose,
            scroller: cx.new(|cx| MessageScrollerState::new(0, cx)),
            users: HashMap::new(),
            chats: HashMap::new(),
            positions: HashMap::new(),
            folders: folder_tabs(&[], 0),
            folder_ix: 0,
            chat_list: Vec::new(),
            open_chat: None,
            topics: Vec::new(),
            topic: None,
            messages: Vec::new(),
            replies: HashMap::new(),
            photos: HashMap::new(),
            requested_photos: HashSet::new(),
            error: None,
            _subscriptions: subscriptions,
            _events: events,
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
                self.request_photos(&messages);
                let count = messages.len();
                self.messages = messages;
                self.scroller.update(cx, |scroller, cx| scroller.reset(count, cx));
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
                self.request_photos(std::slice::from_ref(&message));
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
            Event::Photo { file_id, image } => match render_image(image) {
                Ok(image) => _ = self.photos.insert(file_id, image),
                Err(error) => self.show_error(&format!("{error:#}")),
            },
            Event::Error(error) => self.error = Some(error),
            Event::History { .. }
            | Event::Topics { .. }
            | Event::Topic { .. }
            | Event::Reply { .. }
            | Event::Message(_) => {}
        }
        cx.notify();
    }

    fn set_auth(&mut self, step: AuthStep, window: &mut Window, cx: &mut Context<Self>) {
        let (placeholder, masked) = match &step {
            AuthStep::ApiCredentials => ("API ID".to_string(), false),
            AuthStep::Phone => ("Phone number, e.g. +15550100".to_string(), false),
            AuthStep::Code => ("Login code".to_string(), false),
            AuthStep::Password { hint } => (format!("Password (hint: {hint})"), true),
            AuthStep::Ready | AuthStep::Closed => (String::new(), false),
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
            Some(AuthStep::Ready | AuthStep::Closed) | None => {}
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat_id) = self.open_chat else { return };
        let text = self.compose.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        if !is_allowed(&text) {
            self.show_error(&format!("Not sent, characters outside the allowlist: {text}"));
            cx.notify();
            return;
        }
        self.client.send_text(chat_id, self.topic, text);
        self.compose.update(cx, |input, cx| input.set_value("", window, cx));
    }

    pub(crate) fn open(&mut self, chat_id: i64, cx: &mut Context<Self>) {
        self.open_chat = Some(chat_id);
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
        self.replies.clear();
        self.scroller.update(cx, |scroller, cx| scroller.reset(0, cx));
        cx.notify();
    }

    fn show_error(&mut self, message: &str) {
        tracing::error!("{message}");
        self.error = Some(sanitize(message));
    }

    fn request_photos(&mut self, messages: &[SafeMessage]) {
        for message in messages {
            if let SafeContent::Photo { photo, .. } = &message.content
                && self.requested_photos.insert(photo.file_id)
            {
                self.client.load_photo(photo.file_id);
            }
        }
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

    fn person_note(&self, user_id: i64) -> String {
        let Some(user) = self.users.get(&user_id) else { return format!("id {user_id}") };
        let handle = user.username.as_ref().map_or_else(|| format!("id {user_id}"), |name| format!("@{name}"));
        let bot = if user.is_bot { " · bot" } else { "" };
        let contact = if user.is_contact { "" } else { " · not a contact" };
        format!("{handle}{bot}{contact}")
    }

    fn render_login(&self, cx: &mut Context<Self>) -> Div {
        let title = match self.auth {
            None => "Connecting...",
            Some(AuthStep::ApiCredentials) => "Register plaingram with Telegram",
            Some(AuthStep::Closed) => "Session closed",
            _ => "Sign in",
        };
        let setup = matches!(self.auth, Some(AuthStep::ApiCredentials));
        let asking = setup || matches!(self.auth, Some(AuthStep::Phone | AuthStep::Code | AuthStep::Password { .. }));
        let input = |state: &Entity<InputState>| div().w(px(320.)).child(Input::new(state));
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .child(div().text_xl().child(title))
            .when(setup, |el| {
                el.child(
                    v_flex()
                        .gap_1()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .children(API_SETUP_STEPS.map(|step| div().child(step))),
                )
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
        let note = match chat.kind {
            ChatKind::Group => "group".to_string(),
            ChatKind::Forum => "group with topics".to_string(),
            ChatKind::Private { user_id } => self.person_note(user_id),
            ChatKind::Secret { user_id } => format!("secret · {}", self.person_note(user_id)),
        };
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
            .child(name_label(&chat.title, cx))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(note))
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
            .h_full()
            .child(h_flex().p_3().border_b_1().border_color(cx.theme().border).child(name_label(&chat.title, cx)))
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
                MessageScroller::new("messages", self.scroller.clone(), move |ix, _, cx| {
                    app.read(cx).render_message(ix, cx)
                })
                .flex_1(),
            )
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
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

    fn render_message(&self, ix: usize, cx: &App) -> AnyElement {
        let Some(message) = self.messages.get(ix) else { return div().into_any_element() };
        let bubble = if message.outgoing { cx.theme().accent } else { cx.theme().secondary };
        v_flex()
            .px_4()
            .py_1()
            .map(|el| if message.outgoing { el.items_end() } else { el.items_start() })
            .child(self.render_sender(message, cx))
            .child(
                v_flex()
                    .max_w(px(520.))
                    .p_2()
                    .gap_2()
                    .bg(bubble)
                    .when_some(self.replies.get(&message.id), |el, reply| el.child(self.render_reply(reply, cx)))
                    .child(self.render_content(&message.content, false, cx)),
            )
            .into_any_element()
    }

    /// `compact` shows photos as a label, for reply quotes.
    fn render_content(&self, content: &SafeContent, compact: bool, cx: &App) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        match content {
            SafeContent::Text(text) => safe_text(text, cx).into_any_element(),
            SafeContent::Photo { photo, caption } => v_flex()
                .gap_1()
                .child(match compact {
                    true => div().text_color(muted).child("[photo]").into_any_element(),
                    false => self.render_photo(photo, cx),
                })
                .when(!caption.is_empty(), |el| el.child(safe_text(caption, cx)))
                .into_any_element(),
            SafeContent::Service(text) => {
                div().italic().text_color(muted).child(safe_text(text, cx)).into_any_element()
            }
            SafeContent::Hidden(kind) => div().text_color(muted).child(format!("[hidden: {kind}]")).into_any_element(),
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
                true,
                cx,
            )))
    }

    fn sender_name(&self, sender: Sender) -> Option<&SafeText> {
        match sender {
            Sender::User(id) => self.users.get(&id).map(|u| &u.name),
            Sender::Chat(id) => self.chats.get(&id).map(|c| &c.title),
        }
    }

    fn render_sender(&self, message: &SafeMessage, cx: &App) -> Div {
        let row = h_flex().gap_2().text_xs();
        if message.outgoing {
            return row.child("You");
        }
        let note = match message.sender {
            Sender::User(id) => self.person_note(id),
            Sender::Chat(id) => format!("chat {id}"),
        };
        row.children(self.sender_name(message.sender).map(|name| name_label(name, cx)))
            .child(div().text_color(cx.theme().muted_foreground).child(note))
    }

    fn render_photo(&self, photo: &PhotoRef, cx: &App) -> AnyElement {
        let scale = (PHOTO_MAX_WIDTH / photo.width as f32).min(1.);
        let (width, height) = (px(photo.width as f32 * scale), px(photo.height as f32 * scale));
        match self.photos.get(&photo.file_id) {
            Some(image) => img(ImageSource::Render(image.clone())).w(width).h(height).into_any_element(),
            None => div().w(width).h(height).bg(cx.theme().muted).into_any_element(),
        }
    }
}

impl Render for Plaingram {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.auth {
            Some(AuthStep::Ready) => h_flex()
                .flex_1()
                .min_h_0()
                .child(self.render_chats(cx))
                .child(self.render_conversation(cx))
                .into_any_element(),
            _ => self.render_login(cx).into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .child(TitleBar::new().child(div().text_sm().text_color(cx.theme().muted_foreground).child("plaingram")))
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
            .child(body)
    }
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
