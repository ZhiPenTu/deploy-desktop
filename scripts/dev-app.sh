#!/usr/bin/env bash
# 构建并（可选）启动 macOS 开发版 .app。
#
# 签名顺序很重要：先把 Info.plist、图标、可执行文件全部写完，最后统一做
# ad-hoc 签名并校验。签名之后再覆盖 Contents/MacOS 里的可执行文件，会让
# macOS 的 taskgated 在启动瞬间以 "Code Signature Invalid" 直接 SIGKILL。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="DeployDev"
BUNDLE_ID="com.deploydesktop.devtool"
EXECUTABLE="deploy-desktop-dev"
APP_ROOT="$ROOT/src-tauri/target/dev-app"
APP_BUNDLE="$APP_ROOT/$APP_NAME.app"

usage() {
  cat <<'USAGE'
用法：scripts/dev-app.sh [--release] [--launch] [--offline]

  --release  使用 release 构建（默认 debug）
  --launch   构建完成后打开应用
  --offline  cargo 使用离线模式，依赖已缓存时可用
USAGE
}

profile="debug"
launch=0
cargo_args=()

for arg in "$@"; do
  case "$arg" in
    --release) profile="release" ;;
    --launch) launch=1 ;;
    --offline) cargo_args+=(--offline) ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "未知参数：$arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ "$profile" = "release" ]; then
  cargo_args+=(--release)
fi

if [ "$(uname -s)" != "Darwin" ]; then
  echo "该脚本仅用于 macOS 开发版 .app，当前系统：$(uname -s)" >&2
  exit 1
fi

VERSION="$(cd "$ROOT" && node -p "require('./package.json').version")"

echo "[1/4] 构建前端资源"
(cd "$ROOT" && npm run build)

echo "[2/4] 构建桌面端（${profile}）"
if [ "${#cargo_args[@]}" -gt 0 ]; then
  cargo build --manifest-path "$ROOT/src-tauri/Cargo.toml" "${cargo_args[@]}"
else
  cargo build --manifest-path "$ROOT/src-tauri/Cargo.toml"
fi

BIN="$ROOT/src-tauri/target/$profile/deploy-desktop"
if [ ! -x "$BIN" ]; then
  echo "未找到构建产物：$BIN" >&2
  exit 1
fi

echo "[3/4] 组装 $APP_NAME.app"
mkdir -p "$APP_ROOT"
STAGE="$(mktemp -d "$APP_ROOT/.stage.XXXXXX")"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
install -m 755 "$BIN" "$STAGE/Contents/MacOS/$EXECUTABLE"
cp "$ROOT/src-tauri/icons/icon.icns" "$STAGE/Contents/Resources/icon.icns"

cat > "$STAGE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>$APP_NAME</string>
  <key>CFBundleName</key>
  <string>$APP_NAME</string>
  <key>CFBundleExecutable</key>
  <string>$EXECUTABLE</string>
  <key>CFBundleIdentifier</key>
  <string>$BUNDLE_ID</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSLocalNetworkUsageDescription</key>
  <string>用于通过 SSH 连接局域网内的部署目标服务器，完成离线部署与运维操作。</string>
</dict>
</plist>
PLIST

echo "[4/4] 签名并校验"
codesign --force --deep --sign - "$STAGE"
codesign --verify --deep --strict --verbose=2 "$STAGE"

rm -rf "$APP_BUNDLE"
mv "$STAGE" "$APP_BUNDLE"
trap - EXIT

xattr -dr com.apple.quarantine "$APP_BUNDLE" 2>/dev/null || true

if ! codesign --verify --deep --strict "$APP_BUNDLE" 2>/dev/null; then
  echo "签名校验失败，已中止启动：$APP_BUNDLE" >&2
  exit 1
fi

echo "已生成：$APP_BUNDLE"

if [ "$launch" -eq 1 ]; then
  open "$APP_BUNDLE"
fi
