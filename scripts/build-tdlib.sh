#!/usr/bin/env bash
set -euo pipefail
TD_COMMIT=c8828d50f5eac2fc7089ce1089d7ce9095c61b5e
DIR="$(cd "$(dirname "$0")/.." && pwd)/.tdlib"
rm -rf "$DIR"
mkdir -p "$DIR/src"
git -C "$DIR/src" init -q
git -C "$DIR/src" fetch -q --depth 1 https://github.com/tdlib/td.git "$TD_COMMIT"
git -C "$DIR/src" checkout -q --detach FETCH_HEAD
cmake -S "$DIR/src" -B "$DIR/build" -DCMAKE_BUILD_TYPE=Release \
  -DOPENSSL_ROOT_DIR="$(brew --prefix openssl@3)" -DCMAKE_INSTALL_PREFIX="$DIR/install"
cmake --build "$DIR/build" --target install -j "$(sysctl -n hw.ncpu)"
cp "$(brew --prefix openssl@3)/lib/libssl.a" "$(brew --prefix openssl@3)/lib/libcrypto.a" "$DIR/install/lib/"
