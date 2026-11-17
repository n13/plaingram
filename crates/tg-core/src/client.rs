use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::thread;

use anyhow::{Context, Result, anyhow, bail, ensure};
use async_channel::Receiver;
use base64::prelude::{BASE64_STANDARD, Engine};
use td::enums::{
    AuthorizationState, BasicGroup, Chat, ChatActionBar, ChatList, ChatMemberStatus, ChatPhotos, ChatType, Chats, File,
    ForumTopics, InputFile, InputMessageContent, Message, MessageProperties, MessageReplyTo, MessageSender,
    MessageTopic, Messages, Supergroup, Update, User, UserFullInfo,
};
use td::{functions, types};
use tokio::runtime::Runtime;

use crate::image::{decode_jpeg, encode_jpeg, prepare_upload};
use crate::model::{AuthStep, Backend, DeleteOptions, Event};
use crate::paths;
use crate::policy;
use crate::text::{is_allowed, sanitize, sanitize_name};

const HISTORY_SIZE: usize = 50;
const OLDER_SIZE: usize = 100;
/// How many recently opened chats TDLib keeps.
const RECENT_CHATS: i32 = 50;
const TDLIB_PAGE_LIMIT: usize = 100;
/// TDLib's download priorities: photos on screen and saved files go before photos about to come on screen.
const PRIORITY_ON_SCREEN: i32 = 32;
const PRIORITY_AHEAD: i32 = 1;
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
    /// The current TDLib instance; a new one starts whenever the last one closes.
    id: Arc<AtomicI32>,
    /// Whether to sign in with a phone number and a code instead of the default QR code.
    use_phone: Arc<AtomicBool>,
    data_dir: PathBuf,
    events: async_channel::Sender<Event>,
    allowed: Arc<Mutex<Allowed>>,
    /// Re-encoded photos being sent, by the temporary id of their message, to delete once the send ends.
    uploads: Arc<Mutex<HashMap<i64, PathBuf>>>,
}

impl Client {
    pub fn start() -> Result<(Self, Events)> {
        td::set_log_verbosity(1).map_err(td_error)?;
        let data_dir = paths::data_dir()?;
        let (events, receiver) = async_channel::unbounded();
        let client = Self {
            id: Arc::default(),
            use_phone: Arc::default(),
            data_dir,
            events,
            allowed: Arc::default(),
            uploads: Arc::default(),
        };
        client.start_tdlib();
        let worker = client.clone();
        thread::Builder::new().name("tdlib-updates".into()).spawn(move || worker.receive_loop())?;
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

    fn log_in_with_qr(&self) {
        self.use_phone.store(false, Ordering::Relaxed);
        let id = self.id();
        self.spawn(async move { login_request(functions::request_qr_code_authentication(Vec::new(), id).await) });
    }

    fn log_in_with_phone(&self) {
        self.use_phone.store(true, Ordering::Relaxed);
        // TDLib takes no phone number once it shows a QR code, so this closes it and a fresh instance asks for one.
        self.close_tdlib();
    }

    fn submit_phone(&self, phone: String) {
        let id = self.id();
        self.spawn(async move { login_request(functions::set_authentication_phone_number(phone, None, id).await) });
    }

    fn submit_code(&self, code: String) {
        let id = self.id();
        self.spawn(async move { functions::check_authentication_code(code, id).await.map_err(td_error) });
    }

    fn submit_password(&self, password: String) {
        let id = self.id();
        self.spawn(async move { functions::check_authentication_password(password, id).await.map_err(td_error) });
    }

    fn log_out(&self) {
        self.use_phone.store(false, Ordering::Relaxed);
        self.close_tdlib();
    }

    fn open_chat(&self, chat_id: i64, topic: Option<i32>) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            functions::open_chat(chat_id, this.id()).await.map_err(td_error)?;
            let history = this.history(chat_id, topic, 0, HISTORY_SIZE).await?;
            this.emit(Event::History { chat_id, topic, messages: history.iter().map(policy::message).collect() });
            history.iter().for_each(|message| this.load_reply(message));
            let Chat::Chat(chat) = functions::get_chat(chat_id, this.id()).await.map_err(td_error)?;
            let status = match chat.r#type {
                ChatType::Supergroup(group) => {
                    let Supergroup::Supergroup(group) =
                        functions::get_supergroup(group.supergroup_id, this.id()).await.map_err(td_error)?;
                    group.status
                }
                ChatType::BasicGroup(group) => {
                    let BasicGroup::BasicGroup(group) =
                        functions::get_basic_group(group.basic_group_id, this.id()).await.map_err(td_error)?;
                    group.status
                }
                ChatType::Private(_) | ChatType::Secret(_) => return Ok(()),
            };
            let can_ban = match status {
                ChatMemberStatus::Creator(_) => true,
                ChatMemberStatus::Administrator(admin) => {
                    admin.rights.can_delete_messages && admin.rights.can_restrict_members
                }
                _ => false,
            };
            this.emit(Event::Moderator { chat_id, can_ban });
            Ok(())
        });
    }

    fn ban_sender(&self, chat_id: i64, user_id: i64) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            let member = MessageSender::User(types::MessageSenderUser { user_id });
            functions::ban_chat_member(chat_id, member.clone(), 0, true, this.id()).await.map_err(td_error)?;
            let Chat::Chat(chat) = functions::get_chat(chat_id, this.id()).await.map_err(td_error)?;
            if matches!(chat.r#type, ChatType::Supergroup(_)) {
                functions::delete_chat_messages_by_sender(chat_id, member, this.id()).await.map_err(td_error)?;
            }
            Ok(())
        });
    }

    fn leave_chat(&self, chat_id: i64) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            functions::leave_chat(chat_id, this.id()).await.map_err(td_error)
        });
    }

    fn close_chat(&self, chat_id: i64) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            functions::close_chat(chat_id, this.id()).await.map_err(td_error)
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
                    functions::get_forum_topics(chat_id, String::new(), date, message_id, topic_id, 100, this.id())
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

    fn load_delete_options(&self, chat_id: i64, message_id: i64) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            let MessageProperties::MessageProperties(properties) =
                functions::get_message_properties(chat_id, message_id, this.id()).await.map_err(td_error)?;
            let options = DeleteOptions {
                for_me: properties.can_be_deleted_only_for_self,
                for_everyone: properties.can_be_deleted_for_all_users,
            };
            this.emit(Event::DeleteOptions { chat_id, message_id, options });
            Ok(())
        });
    }

    fn delete_message(&self, chat_id: i64, message_id: i64, for_everyone: bool) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            functions::delete_messages(chat_id, vec![message_id], for_everyone, this.id()).await.map_err(td_error)
        });
    }

    fn mark_read(&self, chat_id: i64, message_ids: Vec<i64>) {
        let this = self.clone();
        self.spawn(async move {
            this.ensure_allowed(chat_id)?;
            functions::view_messages(chat_id, message_ids, None, true, this.id()).await.map_err(td_error)
        });
    }

    fn send_text(&self, chat_id: i64, topic: Option<i32>, text: String) {
        let this = self.clone();
        self.spawn(async move {
            let content = InputMessageContent::InputMessageText(types::InputMessageText {
                text: formatted(text)?,
                link_preview_options: Some(types::LinkPreviewOptions { is_disabled: true, ..Default::default() }),
                clear_draft: true,
            });
            this.send(chat_id, topic, content).await.map(drop)
        });
    }

    fn send_photos(&self, chat_id: i64, topic: Option<i32>, files: Vec<PathBuf>, caption: String) {
        let this = self.clone();
        self.spawn(async move {
            let mut caption = Some(formatted(caption)?).filter(|c| !c.text.is_empty());
            for file in files {
                let out = paths::upload_path(&this.data_dir)?;
                let (path, local) = (out.clone(), out.to_str().context("upload path is not UTF-8")?.to_owned());
                let (width, height) = tokio::task::spawn_blocking(move || prepare_upload(&file, &path)).await??;
                let content = InputMessageContent::InputMessagePhoto(types::InputMessagePhoto {
                    photo: types::InputPhoto {
                        photo: InputFile::Local(types::InputFileLocal { path: local }),
                        thumbnail: None,
                        video: None,
                        added_sticker_file_ids: Vec::new(),
                        width: width.try_into()?,
                        height: height.try_into()?,
                    },
                    caption: caption.take(),
                    show_caption_above_media: false,
                    self_destruct_type: None,
                    has_spoiler: false,
                });
                let message = this.send(chat_id, topic, content).await?;
                this.uploads.lock().expect("uploads lock poisoned").insert(message.id, out);
            }
            Ok(())
        });
    }

    fn load_photo(&self, file_id: i32, on_screen: bool) {
        let priority = if on_screen { PRIORITY_ON_SCREEN } else { PRIORITY_AHEAD };
        let this = self.clone();
        self.spawn(async move {
            let path = this.download(file_id, priority).await?;
            let image = tokio::task::spawn_blocking(move || decode_jpeg(&std::fs::read(path)?)).await??;
            this.emit(Event::Photo { file_id, image });
            Ok(())
        });
    }

    fn hurry_photo(&self, file_id: i32) {
        let id = self.id();
        self.spawn(async move {
            functions::download_file(file_id, PRIORITY_ON_SCREEN, 0, 0, false, id).await.map(drop).map_err(td_error)
        });
    }

    fn save_photo(&self, file_id: i32, name: String) {
        self.save(file_id, move |source| {
            let jpeg = encode_jpeg(&decode_jpeg(&std::fs::read(source)?)?)?;
            paths::save_download(&name, &mut jpeg.as_slice())
        });
    }

    fn save_file(&self, file_id: i32, name: String) {
        self.save(file_id, move |source| {
            let name = match name.is_empty() {
                true => source.file_name().context("downloaded file has no name")?.to_string_lossy().into_owned(),
                false => name,
            };
            paths::save_download(&name, &mut std::fs::File::open(&source)?)
        });
    }

    fn load_profile(&self, user_id: i64) {
        let id = self.id();
        let this = self.clone();
        self.spawn(async move {
            let User::User(user) = functions::get_user(user_id, id).await.map_err(td_error)?;
            let UserFullInfo::UserFullInfo(full) =
                functions::get_user_full_info(user_id, id).await.map_err(td_error)?;
            let ChatPhotos::ChatPhotos(photos) =
                functions::get_user_profile_photos(user_id, 0, 100, id).await.map_err(td_error)?;
            this.emit(Event::Profile(policy::profile(&user, &full, &photos)));
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
                Some(topic) => functions::get_forum_topic_history(chat_id, topic, from, 0, limit, self.id()).await,
                None => functions::get_chat_history(chat_id, from, 0, limit, false, self.id()).await,
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

    /// Emits what `message` replies to, if anything.
    fn load_reply(&self, message: &types::Message) {
        let Some(MessageReplyTo::Message(reply_to)) = message.reply_to.clone() else { return };
        let (chat_id, message_id, id) = (message.chat_id, message.id, self.id());
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
                    self.emit_account_info(u.chat.id, u.chat.action_bar.as_ref());
                }
            }
            Update::ChatActionBar(u) if self.is_allowed(u.chat_id) => {
                self.emit_account_info(u.chat_id, u.action_bar.as_ref());
            }
            Update::ChatTitle(u) if self.is_allowed(u.chat_id) => {
                self.emit(Event::ChatTitle { chat_id: u.chat_id, title: sanitize_name(&u.title) });
            }
            Update::ChatReadInbox(u) if self.is_allowed(u.chat_id) => {
                self.emit(Event::ChatUnread { chat_id: u.chat_id, count: u.unread_count });
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
                    self.spawn(load_all_chats(self.id(), list));
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
            Update::MessageSendSucceeded(u) => {
                self.remove_upload(u.old_message_id)?;
                if self.is_allowed(u.message.chat_id) {
                    self.emit(Event::MessageSent { old_id: u.old_message_id, message: policy::message(&u.message) });
                }
            }
            Update::File(u) if u.file.local.is_downloading_active => {
                self.emit(Event::DownloadProgress { file_id: u.file.id, progress: policy::download_progress(&u.file) });
            }
            Update::MessageSendFailed(u) => {
                self.remove_upload(u.old_message_id)?;
                bail!("sending failed: {}", u.error.message)
            }
            Update::DeleteMessages(u) if u.is_permanent && self.is_allowed(u.chat_id) => {
                self.emit(Event::MessagesDeleted { chat_id: u.chat_id, message_ids: u.message_ids });
            }
            Update::MessageInteractionInfo(u) if self.is_allowed(u.chat_id) => {
                let reactions = policy::reactions(u.interaction_info.as_ref());
                self.emit(Event::Reactions { chat_id: u.chat_id, message_id: u.message_id, reactions });
            }
            _ => {}
        }
        Ok(())
    }

    fn on_auth(&self, state: AuthorizationState) -> Result<()> {
        tracing::info!("TDLib client {}: {}", self.id(), policy::variant_name(&state));
        let step = match state {
            AuthorizationState::WaitTdlibParameters => match ApiCredentials::load()? {
                Some(credentials) => {
                    self.spawn(self.clone().set_parameters(credentials));
                    return Ok(());
                }
                None => AuthStep::ApiCredentials,
            },
            AuthorizationState::WaitPhoneNumber if self.use_phone.load(Ordering::Relaxed) => AuthStep::Phone,
            AuthorizationState::WaitPhoneNumber => {
                self.log_in_with_qr();
                return Ok(());
            }
            AuthorizationState::WaitOtherDeviceConfirmation(state) => {
                ensure!(state.link.starts_with("tg://login?token="), "TDLib sent an unexpected QR code link");
                AuthStep::Qr { link: state.link }
            }
            AuthorizationState::WaitCode(_) => AuthStep::Code,
            AuthorizationState::WaitPassword(p) => AuthStep::Password { hint: sanitize(&p.password_hint) },
            AuthorizationState::Ready => {
                self.spawn(self.clone().emit_account());
                self.spawn(self.clone().emit_recent_chats());
                self.spawn(load_all_chats(self.id(), ChatList::Main));
                self.spawn(load_all_chats(self.id(), ChatList::Archive));
                AuthStep::Ready
            }
            AuthorizationState::Closed => {
                *self.allowed() = Allowed::default();
                self.start_tdlib();
                AuthStep::Closed
            }
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
            self.id(),
        )
        .await
        .map_err(td_error)
    }

    /// Creates a TDLib instance, whose first update asks for its parameters.
    fn start_tdlib(&self) {
        let id = td::create_client();
        self.id.store(id, Ordering::Relaxed);
        self.spawn(async move { functions::get_authorization_state(id).await.map(drop).map_err(td_error) });
    }

    /// Logs out, which deletes TDLib's database and files and closes it; [`AuthorizationState::Closed`] starts a new one.
    fn close_tdlib(&self) {
        let id = self.id();
        self.spawn(async move { functions::log_out(id).await.map_err(td_error) });
    }

    fn id(&self) -> i32 {
        self.id.load(Ordering::Relaxed)
    }

    /// TDLib remembers every chat `openChat` opened, in its database, so the order survives restarts and logging out
    /// deletes it.
    async fn emit_recent_chats(self) -> Result<()> {
        let Chats::Chats(recent) =
            functions::get_recently_opened_chats(RECENT_CHATS, self.id()).await.map_err(td_error)?;
        self.emit(Event::RecentChats(recent.chat_ids.into_iter().filter(|&id| self.is_allowed(id)).collect()));
        Ok(())
    }

    async fn emit_account(self) -> Result<()> {
        let User::User(me) = functions::get_me(self.id()).await.map_err(td_error)?;
        self.emit(Event::Account { user_id: me.id, phone: sanitize(&format!("+{}", me.phone_number)) });
        Ok(())
    }

    /// Downloads a file completely and returns where TDLib keeps it.
    async fn download(&self, file_id: i32, priority: i32) -> Result<PathBuf> {
        let File::File(file) =
            functions::download_file(file_id, priority, 0, 0, true, self.id()).await.map_err(td_error)?;
        ensure!(file.local.is_downloading_completed, "file {file_id} did not finish downloading");
        Ok(file.local.path.into())
    }

    /// Downloads a file, writes it out with `save` off the async runtime, and reports where it went.
    fn save(&self, file_id: i32, save: impl FnOnce(PathBuf) -> Result<PathBuf> + Send + 'static) {
        let this = self.clone();
        self.spawn(async move {
            let source = this.download(file_id, PRIORITY_ON_SCREEN).await?;
            let path = tokio::task::spawn_blocking(move || save(source)).await??;
            this.emit(Event::Saved { file_id, path: sanitize(&path.display().to_string()) });
            Ok(())
        });
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

    /// Emits what Telegram's bar for a stranger's private chat says about them. A private chat has its user's id.
    fn emit_account_info(&self, chat_id: i64, bar: Option<&ChatActionBar>) {
        if chat_id > 0
            && let Some(ChatActionBar::ReportAddBlock(bar)) = bar
            && let Some(info) = &bar.account_info
        {
            self.emit(Event::AccountInfo { user_id: chat_id, info: policy::account_info(info) });
        }
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

    async fn send(&self, chat_id: i64, topic: Option<i32>, content: InputMessageContent) -> Result<types::Message> {
        self.ensure_allowed(chat_id)?;
        let topic = topic.map(|forum_topic_id| MessageTopic::Forum(types::MessageTopicForum { forum_topic_id }));
        let Message::Message(message) =
            functions::send_message(chat_id, topic, None, None, content, self.id()).await.map_err(td_error)?;
        Ok(message)
    }

    fn remove_upload(&self, message_id: i64) -> Result<()> {
        if let Some(path) = self.uploads.lock().expect("uploads lock poisoned").remove(&message_id) {
            std::fs::remove_file(&path).with_context(|| format!("cannot delete {}", path.display()))?;
        }
        Ok(())
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

/// The result of the first request that reaches Telegram, which rejects a bad API ID there. A rejected ID from the
/// keychain is deleted, so the next start asks for it again.
fn login_request(result: Result<(), types::Error>) -> Result<()> {
    match result {
        Err(e) if e.message == "API_ID_INVALID" => match keychain(KEYCHAIN_API_CREDENTIALS)?.delete_credential() {
            Ok(()) => bail!("Telegram rejected the API ID and hash; restart plaingram to enter them again"),
            Err(keyring::Error::NoEntry) => {
                bail!("Telegram rejected the API ID built into this copy of plaingram; get a newer release")
            }
            Err(e) => Err(e.into()),
        },
        result => result.map_err(td_error),
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

fn formatted(text: String) -> Result<types::FormattedText> {
    ensure!(is_allowed(&text), "message contains characters outside the allowlist");
    Ok(types::FormattedText { text, entities: Vec::new() })
}

fn td_error(error: types::Error) -> anyhow::Error {
    anyhow!("TDLib error {}: {}", error.code, error.message)
}
