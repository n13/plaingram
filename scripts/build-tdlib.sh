#!/usr/bin/env bash
set -euo pipefail
TD_COMMIT=c8828d50f5eac2fc7089ce1089d7ce9095c61b5e
DIR="$(cd "$(dirname "$0")/.." && pwd)/.tdlib"
case "$(uname -s)" in
  Darwin) OPENSSL="$(brew --prefix openssl@3)"; OPENSSL_LIB="$OPENSSL/lib" ;;
  Linux) OPENSSL=/usr; OPENSSL_LIB="$(pkg-config --variable=libdir openssl)" ;;
  *) echo "plaingram builds on macOS and Linux only" >&2; exit 1 ;;
esac
rm -rf "$DIR"
mkdir -p "$DIR/src"
git -C "$DIR/src" init -q
git -C "$DIR/src" fetch -q --depth 1 https://github.com/tdlib/td.git "$TD_COMMIT"
git -C "$DIR/src" checkout -q --detach FETCH_HEAD
cmake -S "$DIR/src" -B "$DIR/build" -DCMAKE_BUILD_TYPE=Release \
  -DOPENSSL_ROOT_DIR="$OPENSSL" -DCMAKE_INSTALL_PREFIX="$DIR/install"
cmake --build "$DIR/build" --target install -j "$(getconf _NPROCESSORS_ONLN)"
cp "$OPENSSL_LIB/libssl.a" "$OPENSSL_LIB/libcrypto.a" "$DIR/install/lib/"
cp "$DIR/src/td/generate/scheme/td_api.tl" "$DIR/install/"
