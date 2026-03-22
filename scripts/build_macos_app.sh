#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="KChess"
BUNDLE_ID="${BUNDLE_ID:-com.kchess.app}"
ICON_BUNDLE_SOURCE="${ROOT_DIR}/assets/app/kchess.icon"
BUILD_DIR="${ROOT_DIR}/target/macos-bundle"
APP_DIR="${BUILD_DIR}/${APP_NAME}.app"
ICON_MASTER_PNG="${ICON_BUNDLE_SOURCE}/Assets/Image.png"
ICNS_PATH="${BUILD_DIR}/${APP_NAME}.icns"

if [[ ! -d "${ICON_BUNDLE_SOURCE}" ]]; then
  echo "Missing icon bundle source: ${ICON_BUNDLE_SOURCE}" >&2
  exit 1
fi

if [[ ! -f "${ICON_MASTER_PNG}" ]]; then
  echo "Missing source image in icon bundle: ${ICON_MASTER_PNG}" >&2
  exit 1
fi

mkdir -p "${BUILD_DIR}"
rm -rf "${BUILD_DIR}/${APP_NAME}.iconset" "${BUILD_DIR}/icon-master.png"

echo "[1/5] Building release binary..."
cargo build --release --bin "${APP_NAME}"

echo "[2/5] Preparing app resources..."
echo "      Generating ${APP_NAME}.icns from ${ICON_MASTER_PNG}"
WORK_ICONS_DIR="${BUILD_DIR}/icns-work"
rm -rf "${WORK_ICONS_DIR}"
mkdir -p "${WORK_ICONS_DIR}"

MASTER_SQUARE="${WORK_ICONS_DIR}/master-square.png"
cp "${ICON_MASTER_PNG}" "${MASTER_SQUARE}"
WIDTH="$(sips -g pixelWidth "${MASTER_SQUARE}" | awk '/pixelWidth/ {print $2}')"
HEIGHT="$(sips -g pixelHeight "${MASTER_SQUARE}" | awk '/pixelHeight/ {print $2}')"
if [[ "${WIDTH}" != "${HEIGHT}" ]]; then
  if (( WIDTH < HEIGHT )); then
    SIDE="${WIDTH}"
  else
    SIDE="${HEIGHT}"
  fi
  sips --cropToHeightWidth "${SIDE}" "${SIDE}" "${MASTER_SQUARE}" >/dev/null
fi
sips --resampleHeightWidth 1024 1024 "${MASTER_SQUARE}" >/dev/null

gen_png() {
  local size="$1"
  local out="$2"
  sips --resampleHeightWidth "${size}" "${size}" "${MASTER_SQUARE}" --out "${out}" >/dev/null
}

gen_png 16 "${WORK_ICONS_DIR}/16.png"
gen_png 32 "${WORK_ICONS_DIR}/32.png"
gen_png 64 "${WORK_ICONS_DIR}/64.png"
gen_png 128 "${WORK_ICONS_DIR}/128.png"
gen_png 256 "${WORK_ICONS_DIR}/256.png"
gen_png 512 "${WORK_ICONS_DIR}/512.png"
gen_png 1024 "${WORK_ICONS_DIR}/1024.png"

python3 - "${ICNS_PATH}" "${WORK_ICONS_DIR}" <<'PY'
import struct
import sys
from pathlib import Path

out_path = Path(sys.argv[1])
icons_dir = Path(sys.argv[2])

icon_types = [
    ("icp4", "16.png"),
    ("icp5", "32.png"),
    ("icp6", "64.png"),
    ("ic07", "128.png"),
    ("ic08", "256.png"),
    ("ic09", "512.png"),
    ("ic10", "1024.png"),
]

chunks = []
for icon_type, filename in icon_types:
    data = (icons_dir / filename).read_bytes()
    chunk_len = 8 + len(data)
    chunks.append(icon_type.encode("ascii") + struct.pack(">I", chunk_len) + data)

total_len = 8 + sum(len(chunk) for chunk in chunks)
payload = b"icns" + struct.pack(">I", total_len) + b"".join(chunks)
out_path.write_bytes(payload)
PY

if [[ ! -f "${ICNS_PATH}" ]]; then
  echo "Failed to generate icns at ${ICNS_PATH}" >&2
  exit 1
fi

echo "[3/5] Building .app bundle..."
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS" "${APP_DIR}/Contents/Resources"

cp "${ROOT_DIR}/target/release/${APP_NAME}" "${APP_DIR}/Contents/MacOS/${APP_NAME}"
cp "${ICNS_PATH}" "${APP_DIR}/Contents/Resources/${APP_NAME}.icns"
cp -R "${ROOT_DIR}/assets" "${APP_DIR}/Contents/Resources/assets"

cat > "${APP_DIR}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIconFile</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
  </dict>
</plist>
EOF

echo "[4/5] Clearing quarantine xattr (if present)..."
xattr -d com.apple.quarantine "${APP_DIR}" 2>/dev/null || true

echo "[5/5] Done."
echo "App bundle: ${APP_DIR}"
echo "Run with: open \"${APP_DIR}\""
