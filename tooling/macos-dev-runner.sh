#!/bin/sh
# Cargo's runner on macOS (`.cargo/config.toml`): `cargo run -p kchess-gpui` starts the GPUI app
# through here. The linker signs each build ad hoc, so macOS treats every rebuild as a new app and
# forgets "Always Allow" for the profile's keychain key. Signing the app with a stable identifier
# and your Apple Development identity keeps one keychain answer across rebuilds. Without such an
# identity (or with KCHESS_DEV_SIGNING_IDENTITY=-) the build runs as the linker signed it.

bin="$1"
identifier="com.kchess.gpui.dev"

if [ "$(basename "$bin")" = "KChess" ]; then
  identity="${KCHESS_DEV_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null |
    sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -n 1)}"
  # A rebuild resets the signature to the linker's; an unchanged binary keeps ours.
  if [ -n "$identity" ] && [ "$identity" != "-" ] &&
    ! codesign -dv "$bin" 2>&1 | grep -q "^Identifier=$identifier$"; then
    if ! codesign --force --sign "$identity" --identifier "$identifier" "$bin" 2>/dev/null; then
      echo "[kchess] Could not sign the dev build with \"$identity\"; the keychain may ask again." >&2
    fi
  fi
fi

exec "$@"
