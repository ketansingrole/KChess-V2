#!/bin/sh
# Cargo's runner on macOS (`.cargo/config.toml`): `cargo run -p kchess-gpui` starts the GPUI app
# through here. The linker signs each build ad hoc, so macOS treats every rebuild as a new app and
# forgets "Always Allow" for the profile's keychain key. Signing the app, and the core service
# built beside it, with a stable identifier and your Apple Development identity keeps one keychain
# answer across rebuilds. Without such an identity (or with KCHESS_DEV_SIGNING_IDENTITY=-) the
# builds run as the linker signed them.

bin="$1"

# Signs $1 as identifier $2, unless it already is (a rebuild resets the signature to the linker's).
sign() {
  if [ -n "$identity" ] && [ "$identity" != "-" ] && [ -f "$1" ] &&
    ! codesign -dv "$1" 2>&1 | grep -q "^Identifier=$2$"; then
    if ! codesign --force --sign "$identity" --identifier "$2" "$1" 2>/dev/null; then
      echo "[kchess] Could not sign $(basename "$1") with \"$identity\"; the keychain may ask again." >&2
    fi
  fi
}

if [ "$(basename "$bin")" = "KChess" ]; then
  identity="${KCHESS_DEV_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null |
    sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -n 1)}"
  sign "$bin" com.kchess.gpui.dev
  # The shared core service reads the keychain when the app runs through it (docs/shared-core.md).
  sign "$(dirname "$bin")/kchess-service" com.kchess.service.dev
fi

exec "$@"
