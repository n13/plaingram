# plaingram

Unofficial, super minimal Telegram client for Mac/Linux; Security first.

Design goals:
- Text and images, groups, and folders work
- Cmd-K (Ctrl-K on Linux) jumps to any chat: names that start with what you type come first, then names that contain it, each in the order you last opened them
- Starts in the chat you had open, Cmd-H hides and Cmd-Q quits
- Unread counts beside each chat, from Telegram. A message counts as read only once it is on screen while plaingram is in front
- Reduce attack surface by removing mini apps, bots, animations, etc
- Prevent impersonation and scammers by flagging all suspicious names, e.g. mixed alphabet and weird unicode characters
- Mark look-alike letters in names, such as the capital I in `AIma` posing as `Alma`, and warn wherever a name looks like a contact's or a group's, or borrows a name scammers use

I am pretty confident impersonation particularly becomes impossible with the last feature, the impersonator will just end up with a bunch of ?? in their username and a big red warning label.

## Security model

- `tg-core` is the only crate that sees TDLib types. The UI only receives `SafeText`/`SafeMessage`, and `SafeText` can only be built by `sanitize()`.
- `sanitize()` keeps ASCII digits, punctuation and whitespace, the typographic punctuation Apple keyboards insert (`‘ ’ “ ” • – — …`), a few signs (`€ ₿ ·` …), and every emoji in Unicode's `emoji-test.txt` (vendored in `crates/tg-core/data`) except letter and digit lookalikes (🅰 🅱 🅾 🅿 Ⓜ ℹ 🔟, keycaps, lone regional indicators). Letters come from a fixed set of alphabets (Latin with accents, Greek, Cyrillic, Hebrew, Arabic, Devanagari, Thai, Hangul, CJK). Each word, separated by ASCII whitespace, takes the alphabet of its first letter, and letters from any other alphabet in it are replaced, so `Pаypal` with a Cyrillic `а` shows as `P?ypal`. Names of people and chats and usernames are stricter: the first letter decides for the whole name, so the lone `Ω` in `Alex Ω` is replaced too. Every other grapheme, including invisible control codes, fancy-font and fullwidth letters, becomes a `?` drawn in a highlighted box. Names get a badge per reason listing the replaced characters, and each replacement is logged once to `~/Library/Logs/plaingram.log`. No Unicode normalization, so homoglyphs stay visible.
- Names, chat titles and @handles also mark what passes for something else where it stands, underlined in the same box with a badge saying what it looks like. A capital `I` after the start of a word with lowercase letters shows as `i` (`AIma` as `Aima`), and a lowercase `l` starting a word or in a word without lowercase letters shows as `L` (`ADMlN` as `ADMLN`). `0` and `1` beside letters, `O`, `I` and `l` beside digits, a `|` touching a word and `rn` (which reads as `m`) are marked as they are, and so are letters that Unicode's confusables list (UTS 39) has as twins of plain Latin ones: single ones in Latin words (`ı`, `þ`), and whole words of them in other alphabets (Cyrillic `Арр`). The rules err on the side of caution, so real names such as `McIntyre` and `Bernard` are marked too. Messages are not marked.
- Look-alike warnings: names and @handles are folded to their UTS 39 skeleton, ignoring case, the dot on `i` and everything but letters and digits. Anyone whose folded name or handle matches a contact's or a group's gets a warning, and so does a group whose title matches, on the chat header, beside each of their messages and replies, on their profile, and as a `look-alike` badge in the chat list. Names scammers borrow (Telegram, Durov, Admin, Support, PayPal, Binance, wallets, exchanges and big tech; see `BORROWED_NAMES`) get a warning anywhere in a folded name, unless Telegram verified the account or it is Telegram's own.
- Allowed chats: private (non-bot), secret, basic groups, supergroups that are not channels; forum supergroups show their topics as tabs. Allowed content: text, photos, group service events, and a quote of the message each reply refers to. Anything else renders as `[hidden: <kind>]`.
- Photos: TDLib photo sizes up to 1280px, JPEG only (magic bytes checked), decoded by `zune-jpeg` built without SIMD, so the decoder crate is `#![forbid(unsafe_code)]`. plaingram has its own vendored copy (`vendor/zune-jpeg`), because GPUI and resvg turn SIMD on in the crates.io copy and Cargo merges features within a copy. GPUI only receives decoded pixels.
- Sending photos: Add image under the + beside the message box, or dropping files on a chat, attaches them, with a preview; Send posts each as a photo, the message text as caption of the first. Each is turned upright as the camera recorded, scaled to at most 2560 px (Telegram's largest photo size) and re-encoded as a new JPEG, so no metadata such as location, camera or time leaves the computer. Any format the `image` crate reads works everywhere; on macOS anything else, such as iPhone HEIC, is converted by `sips`. These are your own files, so they go through the regular SIMD decoders.
- Saving: clicking a chat or profile photo opens it over the window. Chat photos can be downloaded, from that view or by right-click; profile photos cannot. Documents, videos, audio and voice messages stay `[hidden: <kind>]`, but right-click saves their file. Everything goes to `~/Downloads`, never over an existing file, with path separators stripped from the name and, on macOS, the quarantine flag set, so Gatekeeper checks anything run from it. Photos are re-encoded by `jpeg-encoder` (also `forbid(unsafe_code)`) from the decoded pixels, so a saved photo holds no byte from its sender: nothing hidden in the original reaches the disk, where Finder previews and Preview would parse it. Other files are saved as sent, so open them with care.
- Reactions show under each message with their counts, filled where you reacted. Custom emoji reactions are pictures, so they show as `custom emoji`, and paid ones as a star.
- Deleting: right-click a message, then Delete. plaingram asks TDLib what this account may do with it and offers only that: Delete for everyone, Delete for me, or neither. Messages others delete disappear too.
- Groups: the ⋮ menu beside a group's name leaves it. Where this account is the group's owner, or an admin who may both delete messages and ban members, right-clicking someone else's message also offers Delete all and ban, which bans them for good and deletes all their messages in the group. Every deletion, ban and leave asks first.
- Outgoing text must pass the same allowlist; link previews are always disabled.
- Profiles: clicking a sender, or a private chat's header, opens their profile with the same badges on name and handle, the look-alike warnings, Telegram's SCAM/FAKE labels, and dates of profile photos. Registration month, phone country and the last rename only appear for strangers who messaged you first, because Telegram shares them nowhere else.
- The TDLib database is encrypted with a random key stored in the system keychain (macOS Keychain, or the Secret Service on Linux: GNOME Keyring or KWallet).

## Layout

- `crates/td`: Rust bindings generated at build time from the `td_api.tl` of the TDLib commit pinned in `scripts/build-tdlib.sh`, linked statically (TDLib + OpenSSL), so the binary has no TDLib dylib to swap. Upgrading TDLib means bumping `TD_COMMIT` and rebuilding.
- `crates/td-gen`: vendored `tdlib-rs-gen` 1.4.0 (MIT/Apache-2.0) with a recursion fix needed by TDLib 1.8.70 (`richTextButton` refers to `inlineButton` as a bare type).
- `crates/tg-core`: allowlist policy, sanitizer, JPEG decoding, TDLib client. The `demo` feature adds an offline `Demo` backend with fixture chats (impostor names, homoglyph links, hidden stickers).
- `crates/tg-ui`: GPUI app with a green-on-black terminal theme (`assets/terminal-theme.json`) in a monospace font (Menlo on macOS, DejaVu Sans Mono on Linux) that keeps `0`/`O`, `1`/`l`/`I` and `rn`/`m` apart.

## Download

Releases for macOS (Apple Silicon) and Linux (x86_64) are on the [Releases page](../../releases), with install notes. They are built from the tagged source by GitHub Actions (`.github/workflows/release.yml`) and include plaingram's Telegram API ID, so they sign in right away. Sign-in shows a QR code to scan in Telegram on your phone (Settings > Devices), like the official desktop apps; "Log in using phone number" switches to a login code instead. Create your account and finish any email or registration steps in the official app first. Settings, the gear at the top right, shows the signed-in account, can show the frame rate (FPS) in the title bar, and logs out, which deletes the chats, photos and files TDLib stored for it.

## Build from source

macOS needs Xcode command line tools and `brew install cmake gperf openssl@3 jq`. Ubuntu 22.04 or newer needs `sudo apt install build-essential cmake gperf pkg-config libssl-dev zlib1g-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev`. Rust is installed by `rustup` from `rust-toolchain.toml`.

```sh
./scripts/build-tdlib.sh   # builds TDLib 1.8.70 and OpenSSL-linked static libraries into .tdlib/install
./scripts/bundle.sh        # macOS: target/release/plaingram.app, Linux: target/release/plaingram-linux/
```

A build from source has no API ID built in. On first launch it asks for one: create an app at https://my.telegram.org (API development tools) and paste its api_id and api_hash, which are stored in the system keychain. To build in an ID instead, set `PLAINGRAM_API_ID` and `PLAINGRAM_API_HASH` when building. Release builds get them from the repository secrets of the same names, and a tag push fails if they are missing; pull request builds never include them.

## Develop

```sh
cargo run -p tg-ui --features demo   # offline UI with fixture data, no account needed
cargo test --workspace               # includes a headless GPUI render of the demo screens
```

The screenshot test (macOS only) renders off-screen through Metal and writes `target/screenshots/*.png`; it needs no Screen Recording permission. The app logs to `~/Library/Logs/plaingram.log` on macOS and `~/.local/state/plaingram/plaingram.log` on Linux.

To release, run `./scripts/release.sh` on a clean `main`: it bumps the patch version (or sets the one given, e.g. `./scripts/release.sh 0.2.0`), commits, tags and pushes, and the tag push builds and publishes the release. The workflow checks that the tag matches the version in `Cargo.toml`.

## License

MIT, see [LICENSE](LICENSE). `crates/td-gen` keeps its original MIT OR Apache-2.0 license, and release builds statically link TDLib (Boost Software License 1.0) and OpenSSL (Apache-2.0).
