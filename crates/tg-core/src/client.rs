use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::thread;

use anyhow::{Context, Result, anyhow, bail, ensure};
use async_channel::Receiver;
use base64::prelude::{BASE64_STANDARD, Engine};
use td::enums::{
    AuthorizationState, Chat, ChatList, Chats, File, ForumTopics, InputMessageContent, Message, MessageReplyTo,
    MessageTopic, Messages, Update,
};
use td::{functions, types};
use tokio::runtime::Runtime;

use crate::image::decode_jpeg;
use crate::model::{AuthStep, Backend, Event};
use crate::policy;
use crate::text::{is_allowed, report, sanitize, sanitize_name};

const HISTORY_SIZE: usize = 50;
const OLDER_SIZE: usize = 100;
const TDLIB_PAGE_LIMIT: usize = 100;
const KEYCHAIN_SERVICE: &str = "plaingram";
const KEYCHAIN_DB_KEY: &str = "tdlib-database-key";
const KEYCHAIN_API_CREDENTIALS: &str = "api-credentials";

pub type Events = Receiver<Event>;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).build().expect("failed to start tokio runtime")
});

/// Release builds bake in the project's API credentials from these variables at compile time.
const BUILT_IN: Option<(&str, &str)> = match (option_env!("PLAINGRAM_API_ID"), option_env!("PLAINGRAM_API_HASH")) {
    (Some(id), Some(hash)) => Some((id, hash)),
    (None, None) => None,
    _ => panic!("set both PLAINGRAM_API_ID and PLAINGRAM_API_HASH, or neither"),
};

/// The app's api_id/api_hash from my.telegram.org: entered by the user and kept in the keychain, or [`BUILT_IN`].
struct ApiCredentials {
    id: i32,
    hash: String,
}

impl ApiCredentials {
    fn parse(id: &str, hash: &str) -> Result<Self> {
        let id: i32 = id.trim().parse().context("API ID must be a number")?;
        let hash = hash.trim();
        ensure!(id > 0, "API ID must be positive");
        ensure!(hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()), "API hash must be 32 hex characters");
        Ok(Self { id, hash: hash.into() })
    }

    fn load() -> Result<Option<Self>> {
        let Some(stored) = keychain_read(KEYCHAIN_API_CREDENTIALS)? else {
            return BUILT_IN.map(|(id, hash)| Self::parse(id, hash)).transpose();
        };
        let (id, hash) =
            std::str::from_utf8(&stored)?.split_once(':').context("stored API credentials are malformed")?;
        Self::parse(id, hash).map(Some)
    }

    fn save(&self) -> Result<()> {
        Ok(keychain(KEYCHAIN_API_CREDENTIALS)?.set_secret(format!("{}:{}", self.id, self.hash).as_bytes())?)
    }
}

#[derive(Default)]
struct Allowed {
    chats: HashSet<i64>,
    bots: HashSet<i64>,
    /// Supergroup ids.
    forums: HashSet<i64>,
}

/// Every TDLib object is converted by [`policy`] before it leaves this type.
#[derive(Clone)]
pub struct Client {
    id: i32,
    data_dir: PathBuf,
    events: async_channel::Sender<Event>,
    allowed: Arc<Mutex<Allowed>>,
}

impl Client {
    pub fn start() -> Result<(Self, Events)> {
        td::set_log_verbosity(1).map_err(td_error)?;
        let data_dir = crate::paths::data_dir()?;
        let (events, receiver) = async_channel::unbounded();
        let client = Self { id: td::create_client(), data_dir, events, allowed: Arc::default() };
        let worker = client.clone();
        thread::Builder::new().name("tdlib-updates".into()).spawn(move || worker.receive_loop())?;
        let id = client.id;
        client.spawn(async move { functions::get_authorization_state(id).await.map(drop).map_err(td_error) });
        Ok((client, receiver))
    }
}

impl Backend for Client {
    fn submit_api_credentials(&self, api_id: String, api_hash: String) {
        let this = self.clone();
        self.spawn(async move {
            let credentials = ApiCredentials::parse(&api_id, &api_hash)?;
            credentials.save()?;
            this.set_parameters(credentials).await
        });
    }

    fn submit_phone(&self, phone: String) {
        let id = self.id;
        self.spawn(async move {
            match functions::set_authentication_phone_number(phone, None, id).await {
                Err(e) if e.message == "API_ID_INVALID" => {
                    match keychain(KEYCHAIN_API_CREDENTIALS)?.delete_credential() {
                        Ok(()) => bail!("Telegram rejected the API ID and hash; restart plaingram to enter them again"),
                        Err(keyring::Error::NoEntry) => {
                            bail!("Telegram rejected the API ID built into this copy of plaingram; get a newer release")
                        }
                        Err(e) => Err(e.into()),
                    }
                }
                result => result.map_err(td_error),
            }
        });
    }

    fn submit_code(&self, code: String) {
        let id = self.id;
        self.spawn(async move { functions::check_authentication_code(code, id).await.map_err(td_error) });
    }

    fn submit_password(&self, password: String) {
        let id = self.id;
        self.spawn(async move { functions::check_authentication_password(password, id).await.map_err(td_error) });
    }

    fn open_chat(&self, chat_id: i64, topic: Option<i32>) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            functions::open_chat(chat_id, this.id).await.map_err(td_error)?;
            let history = this.history(chat_id, topic, 0, HISTORY_SIZE).await?;
            this.mark_read_now(chat_id, history.iter().map(|m| m.id).collect()).await?;
            this.emit(Event::History { chat_id, topic, messages: history.iter().map(policy::message).collect() });
            history.iter().for_each(|message| this.load_reply(message));
            Ok(())
        });
    }

    fn load_older(&self, chat_id: i64, topic: Option<i32>, before: i64) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            let history = this.history(chat_id, topic, before, OLDER_SIZE).await?;
            let messages = history.iter().map(policy::message).collect();
            this.emit(Event::OlderHistory { chat_id, topic, before, messages });
            history.iter().for_each(|message| this.load_reply(message));
            Ok(())
        });
    }

    fn load_topics(&self, chat_id: i64) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            let mut topics = Vec::new();
            let (mut date, mut message_id, mut topic_id) = (0, 0, 0);
            loop {
                let ForumTopics::ForumTopics(page) =
                    functions::get_forum_topics(chat_id, String::new(), date, message_id, topic_id, 100, this.id)
                        .await
                        .map_err(td_error)?;
                let done = page.topics.is_empty();
                topics.extend(page.topics.iter().map(|t| policy::topic(&t.info)));
                if done || topics.len() >= page.total_count.try_into()? {
                    break;
                }
                (date, message_id, topic_id) =
                    (page.next_offset_date, page.next_offset_message_id, page.next_offset_forum_topic_id);
            }
            this.emit(Event::Topics { chat_id, topics });
            Ok(())
        });
    }

    fn mark_read(&self, chat_id: i64, message_ids: Vec<i64>) {
        let this = self.clone();
        self.spawn(async move { this.mark_read_now(chat_id, message_ids).await });
    }

    fn send_text(&self, chat_id: i64, topic: Option<i32>, text: String) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            ensure!(is_allowed(&text), "message contains characters outside the allowlist");
            let content = InputMessageContent::InputMessageText(types::InputMessageText {
                text: types::FormattedText { text, entities: Vec::new() },
                link_preview_options: Some(types::LinkPreviewOptions { is_disabled: true, ..Default::default() }),
                clear_draft: true,
            });
            let topic = topic.map(|forum_topic_id| MessageTopic::Forum(types::MessageTopicForum { forum_topic_id }));
            functions::send_message(chat_id, topic, None, None, content, this.id).await.map_err(td_error)?;
            Ok(())
        });
    }

    fn load_photo(&self, file_id: i32) {
        let this = self.clone();
        self.spawn(async move {
            let File::File(file) = functions::download_file(file_id, 1, 0, 0, true, this.id).await.map_err(td_error)?;
            ensure!(file.local.is_downloading_completed, "photo {file_id} did not finish downloading");
            let image = tokio::task::spawn_blocking(move || decode_jpeg(&std::fs::read(&file.local.path)?)).await??;
            this.emit(Event::Photo { file_id, image });
            Ok(())
        });
    }
}

impl Client {
    /// Up to `count` messages older than `from` (0 for the newest), oldest first.
    async fn history(
        &self,
        chat_id: i64,
        topic: Option<i32>,
        mut from: i64,
        count: usize,
    ) -> Result<Vec<types::Message>> {
        let mut history = Vec::new();
        while history.len() < count {
            let limit = (count - history.len()).min(TDLIB_PAGE_LIMIT).try_into()?;
            let page = match topic {
                Some(topic) => functions::get_forum_topic_history(chat_id, topic, from, 0, limit, self.id).await,
                None => functions::get_chat_history(chat_id, from, 0, limit, false, self.id).await,
            };
            let Messages::Messages(page) = page.map_err(td_error)?;
            let page: Vec<_> = page.messages.into_iter().flatten().collect();
            let Some(last) = page.last() else { break };
            from = last.id;
            history.extend(page);
        }
        history.reverse();
        Ok(history)
    }

    async fn mark_read_now(&self, chat_id: i64, message_ids: Vec<i64>) -> Result<()> {
        self.ensure_allowed(chat_id)?;
        functions::view_messages(chat_id, message_ids, None, true, self.id).await.map_err(td_error)
    }

    /// Emits what `message` replies to, if anything.
    fn load_reply(&self, message: &types::Message) {
        let Some(MessageReplyTo::Message(reply_to)) = message.reply_to.clone() else { return };
        let (chat_id, message_id, id) = (message.chat_id, message.id, self.id);
        let this = self.clone();
        self.spawn(async move {
            let reply = if reply_to.message_id == 0 {
                let sender = reply_to.origin.as_ref().and_then(policy::origin);
                policy::reply(sender, reply_to.content.as_ref(), reply_to.quote.as_ref())
            } else {
                match functions::get_replied_message(chat_id, message_id, id).await {
                    Ok(Message::Message(original)) => policy::reply(
                        Some(policy::sender(&original.sender_id)),
                        Some(&original.content),
                        reply_to.quote.as_ref(),
                    ),
                    Err(e) if e.code == 404 => policy::reply(None, None, None),
                    Err(e) => return Err(td_error(e)),
                }
            };
            this.emit(Event::Reply { chat_id, message_id, reply });
            Ok(())
        });
    }

    fn receive_loop(self) {
        loop {
            if let Some(update) = td::receive(2.0)
                && let Err(error) = self.handle(update)
            {
                self.fail(error);
            }
        }
    }

    fn handle(&self, update: Update) -> Result<()> {
        match update {
            Update::AuthorizationState(u) => self.on_auth(u.authorization_state)?,
            Update::User(u) => {
                let user = policy::user(&u.user);
                if user.is_bot {
                    self.allowed().bots.insert(user.id);
                }
                if user.name.has_replacements() {
                    self.spawn(report_groups_in_common(self.id, u.user));
                }
                self.emit(Event::User(user));
            }
            Update::Supergroup(u) => {
                let mut allowed = self.allowed();
                if u.supergroup.is_forum {
                    allowed.forums.insert(u.supergroup.id);
                } else {
                    allowed.forums.remove(&u.supergroup.id);
                }
            }
            Update::NewChat(u) => {
                let chat = {
                    let mut allowed = self.allowed();
                    let chat = policy::chat(&u.chat, &allowed.bots, &allowed.forums);
                    chat.inspect(|c| _ = allowed.chats.insert(c.id))
                };
                if let Some(chat) = chat {
                    self.emit(Event::Chat(chat));
                    self.emit_positions(u.chat.id, &u.chat.positions);
                }
            }
            Update::ChatTitle(u) if self.is_allowed(u.chat_id) => {
                self.emit(Event::ChatTitle { chat_id: u.chat_id, title: sanitize_name(&u.title) });
            }
            Update::ChatPosition(u) if self.is_allowed(u.chat_id) => {
                self.emit_positions(u.chat_id, std::slice::from_ref(&u.position));
            }
            Update::ChatLastMessage(u) if self.is_allowed(u.chat_id) => self.emit_positions(u.chat_id, &u.positions),
            Update::ChatDraftMessage(u) if self.is_allowed(u.chat_id) => self.emit_positions(u.chat_id, &u.positions),
            Update::ForumTopicInfo(u) if self.is_allowed(u.info.chat_id) => {
                self.emit(Event::Topic { chat_id: u.info.chat_id, topic: policy::topic(&u.info) });
            }
            Update::ChatFolders(u) => {
                for folder in &u.chat_folders {
                    let list = ChatList::Folder(types::ChatListFolder { chat_folder_id: folder.id });
                    self.spawn(load_all_chats(self.id, list));
                }
                let main_position = u.main_chat_list_position.try_into()?;
                ensure!(main_position <= u.chat_folders.len(), "'All chats' position {main_position} is out of range");
                self.emit(Event::Folders {
                    folders: u.chat_folders.iter().map(policy::folder).collect(),
                    main_position,
                });
            }
            Update::NewMessage(u) if self.is_allowed(u.message.chat_id) => {
                self.emit(Event::Message(policy::message(&u.message)));
                self.load_reply(&u.message);
            }
            Update::MessageSendSucceeded(u) if self.is_allowed(u.message.chat_id) => {
                self.emit(Event::MessageSent { old_id: u.old_message_id, message: policy::message(&u.message) });
            }
            Update::File(u) if u.file.local.is_downloading_active => {
                self.emit(Event::PhotoProgress { file_id: u.file.id, progress: policy::download_progress(&u.file) });
            }
            Update::MessageSendFailed(u) => bail!("sending failed: {}", u.error.message),
            _ => {}
        }
        Ok(())
    }

    fn on_auth(&self, state: AuthorizationState) -> Result<()> {
        let step = match state {
            AuthorizationState::WaitTdlibParameters => match ApiCredentials::load()? {
                Some(credentials) => {
                    self.spawn(self.clone().set_parameters(credentials));
                    return Ok(());
                }
                None => AuthStep::ApiCredentials,
            },
            AuthorizationState::WaitPhoneNumber => AuthStep::Phone,
            AuthorizationState::WaitCode(_) => AuthStep::Code,
            AuthorizationState::WaitPassword(p) => AuthStep::Password { hint: sanitize(&p.password_hint) },
            AuthorizationState::Ready => {
                self.spawn(load_all_chats(self.id, ChatList::Main));
                self.spawn(load_all_chats(self.id, ChatList::Archive));
                AuthStep::Ready
            }
            AuthorizationState::Closed => AuthStep::Closed,
            AuthorizationState::LoggingOut | AuthorizationState::Closing => return Ok(()),
            other => bail!("unsupported login step {}; finish it in the official app", policy::variant_name(&other)),
        };
        self.emit(Event::Auth(step));
        Ok(())
    }

    async fn set_parameters(self, credentials: ApiCredentials) -> Result<()> {
        let dir = |name: &str| self.data_dir.join(name).to_str().map(str::to_owned).context("data dir is not UTF-8");
        functions::set_tdlib_parameters(
            false,
            dir("db")?,
            dir("files")?,
            database_key()?,
            true,
            true,
            true,
            true,
            credentials.id,
            credentials.hash,
            "en".into(),
            "Desktop".into(),
            "macOS".into(),
            env!("CARGO_PKG_VERSION").into(),
            self.id,
        )
        .await
        .map_err(td_error)
    }

    fn spawn(&self, task: impl Future<Output = Result<()>> + Send + 'static) {
        let this = self.clone();
        RUNTIME.spawn(async move {
            if let Err(error) = task.await {
                this.fail(error);
            }
        });
    }

    fn fail(&self, error: anyhow::Error) {
        tracing::error!("{error:#}");
        self.emit(Event::Error(sanitize(&format!("{error:#}"))));
    }

    fn emit_positions(&self, chat_id: i64, positions: &[types::ChatPosition]) {
        for p in positions {
            self.emit(Event::ChatPosition { chat_id, list: policy::chat_list(&p.list), order: p.order });
        }
    }

    fn emit(&self, event: Event) {
        if self.events.try_send(event).is_err() {
            tracing::error!("event receiver is gone");
        }
    }

    fn allowed(&self) -> MutexGuard<'_, Allowed> {
        self.allowed.lock().expect("allowlist lock poisoned")
    }

    fn is_allowed(&self, chat_id: i64) -> bool {
        self.allowed().chats.contains(&chat_id)
    }

    fn ensure_allowed(&self, chat_id: i64) -> Result<()> {
        ensure!(self.is_allowed(chat_id), "chat {chat_id} is not allowed");
        Ok(())
    }
}

async fn load_all_chats(id: i32, list: ChatList) -> Result<()> {
    loop {
        match functions::load_chats(Some(list.clone()), 100, id).await {
            Ok(()) => {}
            Err(e) if e.code == 404 => return Ok(()),
            Err(e) => return Err(td_error(e)),
        }
    }
}

/// Logs the groups shared with a user whose name was flagged, so they can be found.
async fn report_groups_in_common(id: i32, user: types::User) -> Result<()> {
    let groups = match functions::get_groups_in_common(user.id, 0, 100, id).await {
        Ok(Chats::Chats(common)) => {
            let mut titles = Vec::new();
            for chat_id in common.chat_ids {
                let Chat::Chat(chat) = functions::get_chat(chat_id, id).await.map_err(td_error)?;
                titles.push(chat.title);
            }
            format!("in groups {titles:?}")
        }
        Err(e) if e.message == "Have no access to the user" => "in groups Telegram won't list".into(),
        Err(e) => return Err(td_error(e)),
    };
    report(format!("flagged name: {:?} is user {} {groups}", policy::full_name(&user), user.id));
    Ok(())
}

fn database_key() -> Result<String> {
    let key = match keychain_read(KEYCHAIN_DB_KEY)? {
        Some(key) => key,
        None => {
            let mut key = vec![0; 32];
            getrandom::fill(&mut key)?;
            keychain(KEYCHAIN_DB_KEY)?.set_secret(&key)?;
            key
        }
    };
    Ok(BASE64_STANDARD.encode(key))
}

fn keychain(name: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, name)
}

fn keychain_read(name: &str) -> Result<Option<Vec<u8>>> {
    match keychain(name)?.get_secret() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn td_error(error: types::Error) -> anyhow::Error {
    anyhow!("TDLib error {}: {}", error.code, error.message)
}
