## Install

**macOS (Apple Silicon):** download `plaingram-macos-arm64.zip`, unzip it and move `plaingram.app` to Applications. The app is not notarized by Apple yet, so macOS blocks the first launch: try to open it once, then go to System Settings → Privacy & Security and click **Open Anyway**. macOS asks for Keychain access again after each update.

**Linux (x86_64):** download `plaingram-linux-x86_64.tar.gz`, extract it and run `plaingram-linux/plaingram`. It needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 or later) and a Secret Service keyring such as GNOME Keyring or KWallet, which holds the database key.

These files are built from the tagged source by the "Build and release" GitHub Actions workflow. `SHA256SUMS` lists their checksums; check a download with `shasum -a 256 -c SHA256SUMS --ignore-missing` on macOS or `sha256sum -c SHA256SUMS --ignore-missing` on Linux.
