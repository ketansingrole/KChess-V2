#!/bin/sh
# Cargo's runner on macOS (`.cargo/config.toml`): `cargo run -p kchess-gpui` starts the GPUI app
# through here. The linker signs each build ad hoc, so macOS treats every rebuild as a new app and
# forgets "Always Allow" for the profile's keychain key. Signing the app, and the core service
# built beside it, with a stable identifier and your Apple Development identity keeps one keychain
# answer across rebuilds. Without such an identity (or with KCHESS_DEV_SIGNING_IDENTITY=-) the
# builds run as the linker signed them.
#
# A debug build also runs from a bundle beside it, target/debug/KChess.app: Mission Control and the
# app switcher take an app's icon from its bundle, and a bare executable shows a blank document. A
# release build stays bare, since the app takes a release build in a bundle for an installation
# that updates itself (apps/gpui/src/updates.rs).

bin="$1"

# Links the build into target/debug/KChess.app, from where it finds the service as it would beside
# the build. The bundle's identifier is not the packaged app's, so the two never meet.
bundle() {
  contents="$1/Contents"
  mkdir -p "$contents/MacOS" "$contents/Resources" || return 1
  # Signing replaces the bundle's copy, so what is newer here is a new build.
  [ ! "$bin" -nt "$contents/MacOS/KChess" ] || ln -f "$bin" "$contents/MacOS/KChess" || return 1
  [ ! "$service" -nt "$contents/MacOS/kchess-service" ] ||
    ln -f "$service" "$contents/MacOS/kchess-service" || return 1
  cp -f "$(dirname "$0")/../apps/gpui/build/icon.icns" "$contents/Resources/icon.icns" || return 1
  cat >"$contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>KChess</string>
  <key>CFBundleIdentifier</key>
  <string>com.kchess.gpui.dev</string>
  <key>CFBundleExecutable</key>
  <string>KChess</string>
  <key>CFBundleIconFile</key>
  <string>icon</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>KChess uses your microphone to recognize spoken chess moves and square names on this device.</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST
}

# Signs $1 as identifier $2, unless it already is, soundly (a rebuild resets the signature to the
# linker's, and a build signed alone seals nothing of the bundle it is linked into).
sign() {
  if [ -n "$identity" ] && [ "$identity" != "-" ] && [ -e "$1" ] &&
    { ! codesign -dv "$1" 2>&1 | grep -q "^Identifier=$2$" || ! codesign --verify "$1" 2>/dev/null; }; then
    if ! codesign --force --sign "$identity" --identifier "$2" "$1" 2>/dev/null; then
      echo "[kchess] Could not sign $(basename "$1") with \"$identity\"; the keychain may ask again." >&2
    fi
  fi
}

if [ "$(basename "$bin")" = "KChess" ]; then
  identity="${KCHESS_DEV_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null |
    sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -n 1)}"
  # The shared core service reads the keychain when the app runs through it (docs/shared-core.md).
  service="$(dirname "$bin")/kchess-service"
  sign "$service" com.kchess.service.dev
  app="$bin.app"
  if [ "$(basename "$(dirname "$bin")")" = "debug" ] && bundle "$app"; then
    sign "$app" com.kchess.gpui.dev
    shift
    exec "$app/Contents/MacOS/KChess" "$@"
  fi
  sign "$bin" com.kchess.gpui.dev
fi

exec "$@"
