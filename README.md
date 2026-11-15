# plaingram

Unofficial, allowlist-only desktop client for the Telegram API: 1:1, secret and group chats with text and photos. Everything else is hidden.

## Security model

- `tg-core` is the only crate that sees TDLib types. The UI only receives `SafeText`/`SafeMessage`, and `SafeText` can only be built by `sanitize()`.
- `sanitize()` keeps ASCII digits, punctuation and whitespace, the typographic punctuation Apple keyboards insert (`‘ ’ “ ” • – — …`), a few signs (`€ ₿ ·` …), and every emoji in Unicode's `emoji-test.txt` (vendored in `crates/tg-core/data`) except letter and digit lookalikes (🅰 🅱 🅾 🅿 Ⓜ ℹ 🔟, keycaps, lone regional indicators). Letters come from a fixed set of alphabets (Latin with accents, Greek, Cyrillic, Hebrew, Arabic, Devanagari, Thai, Hangul, CJK). Each word, separated by ASCII whitespace, takes the alphabet of its first letter, and letters from any other alphabet in it are replaced, so `Pаypal` with a Cyrillic `а` shows as `P?ypal`. Names of people and chats and usernames are stricter: the first letter decides for the whole name, so the lone `Ω` in `Alex Ω` is replaced too, and each flagged name is logged. Every other grapheme, including invisible control codes, fancy-font and fullwidth letters, becomes a `?` drawn in a highlighted box. Names get a badge per reason listing the replaced characters, and each replacement is logged once to `~/Library/Logs/plaingram.log`. No Unicode normalization, so homoglyphs stay visible.
- Allowed chats: private (non-bot), secret, basic groups, supergroups that are not channels; forum supergroups show their topics as tabs. Allowed content: text, photos, group service events, and a quote of the message each reply refers to. Anything else renders as `[hidden: <kind>]`.
- Photos: TDLib photo sizes up to 1280px, JPEG only (magic bytes checked), decoded by `zune-jpeg` built without SIMD, so the decoder crate is `#![forbid(unsafe_code)]`. GPUI only receives decoded pixels.
- Outgoing text must pass the same allowlist; link previews are always disabled.
- The TDLib database is encrypted with a random key stored in the macOS Keychain.

## Layout

- `crates/td`: Rust bindings generated at build time from the `td_api.tl` of the TDLib commit pinned in `scripts/build-tdlib.sh`, linked statically (TDLib + OpenSSL), so the binary has no TDLib dylib to swap. Upgrading TDLib means bumping `TD_COMMIT` and rebuilding.
- `crates/td-gen`: vendored `tdlib-rs-gen` 1.4.0 (MIT/Apache-2.0) with a recursion fix needed by TDLib 1.8.70 (`richTextButton` refers to `inlineButton` as a bare type).
- `crates/tg-core`: allowlist policy, sanitizer, JPEG decoding, TDLib client. The `demo` feature adds an offline `Demo` backend with fixture chats (impostor names, homoglyph links, hidden stickers).
- `crates/tg-ui`: GPUI app with a green-on-black terminal theme (`assets/terminal-theme.json`, Menlo, which keeps `0`/`O` and `1`/`l`/`I` distinct).

## Run

```sh
./scripts/build-tdlib.sh   # builds TDLib 1.8.70 from source into .tdlib/
cargo run -p tg-ui
```

As a Mac app with its icon (`crates/tg-ui/assets/icon.svg`):

```sh
./scripts/bundle.sh           # release build, icon, Info.plist, signed with your Apple Development certificate
open target/release/plaingram.app
```

On first launch the app asks for its API ID and hash (create an app at https://my.telegram.org, API development tools) and stores them in the Keychain, then signs in with phone, code and 2FA password. Create the account and finish any email/registration steps in the official app first.

## Develop

```sh
cargo run -p tg-ui --features demo   # offline UI with fixture data, no account needed
cargo test --workspace               # includes a headless GPUI render of the demo screens
```

The screenshot test renders off-screen through Metal and writes `target/screenshots/*.png`; it needs no Screen Recording permission.
