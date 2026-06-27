#!/bin/bash
# 创建带背景图和拖拽引导布局的 macOS DMG
# 用法: bash scripts/create-dmg.sh [app路径] [输出dmg路径]
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_PATH="${1:-$ROOT_DIR/target/release/bundle/macos/Mira.app}"
BACKGROUND="$ROOT_DIR/scripts/assets/dmg-background.png"
BACKGROUND_2X="$ROOT_DIR/scripts/assets/dmg-background@2x.png"

if [ ! -d "$APP_PATH" ]; then
  echo "错误: 找不到 .app: $APP_PATH" >&2
  exit 1
fi
if [ ! -f "$BACKGROUND" ]; then
  echo "错误: 找不到背景图: $BACKGROUND" >&2
  exit 1
fi

APP_NAME=$(basename "$APP_PATH" .app)
VERSION=$(defaults read "$APP_PATH/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || echo "0.0.0")
# 检测 universal 构建（路径包含 universal-apple-darwin），否则按当前 CPU 架构命名
case "$APP_PATH" in
  *universal-apple-darwin*) ARCH="universal" ;;
  *) case "$(uname -m)" in
       arm64) ARCH="aarch64" ;;
       *) ARCH="$(uname -m)" ;;
     esac ;;
esac
DMG_DIR="$ROOT_DIR/target/release/bundle/dmg"
DMG_NAME="${APP_NAME}_${VERSION}_${ARCH}.dmg"
DMG_PATH="${2:-$DMG_DIR/$DMG_NAME}"
TMP_DMG="/tmp/${APP_NAME}-build-$$.dmg"
ATTACH_PLIST="/tmp/${APP_NAME}-attach-$$.plist"
VOL_NAME="${DMG_VOLUME_NAME:-$APP_NAME Installer}"
MOUNT_POINT=""

echo "==> 创建可读写 DMG"
hdiutil create -ov -volname "$VOL_NAME" -fs HFS+ -size 300m "$TMP_DMG" >/dev/null

echo "==> 挂载"
hdiutil attach -readwrite -nobrowse -noautoopen -plist "$TMP_DMG" > "$ATTACH_PLIST"
MOUNT_POINT=$(/usr/libexec/PlistBuddy -c "Print :system-entities:0:mount-point" "$ATTACH_PLIST")
if [ -z "$MOUNT_POINT" ] || [ ! -d "$MOUNT_POINT" ]; then
  echo "错误: 无法确定 DMG 挂载点" >&2
  exit 1
fi

cleanup() {
  [ -n "$MOUNT_POINT" ] && hdiutil detach "$MOUNT_POINT" -force >/dev/null 2>&1 || true
  rm -f "$TMP_DMG"
  rm -f "$ATTACH_PLIST"
}
trap cleanup EXIT

echo "==> 拷贝文件"
cp -R "$APP_PATH" "$MOUNT_POINT/"
ln -sfh /Applications "$MOUNT_POINT/Applications"
mkdir -p "$MOUNT_POINT/.background"
cp "$BACKGROUND" "$MOUNT_POINT/.background/background.png"
[ -f "$BACKGROUND_2X" ] && cp "$BACKGROUND_2X" "$MOUNT_POINT/.background/background@2x.png"

echo "==> 设置窗口布局"
osascript <<APPLESCRIPT >/dev/null
tell application "Finder"
    set dmgFolder to (POSIX file "$MOUNT_POINT") as alias
    open dmgFolder
    delay 1

    set theWindow to container window of dmgFolder
    set current view of theWindow to icon view
    try
        set toolbar visible of theWindow to false
    end try
    try
        set statusbar visible of theWindow to false
    end try
    set the bounds of theWindow to {0, 0, 660, 400}

    set theViewOptions to icon view options of theWindow
    set arrangement of theViewOptions to not arranged
    set icon size of theViewOptions to 80
    set background picture of theViewOptions to (POSIX file "$MOUNT_POINT/.background/background.png") as alias

    set position of item "$APP_NAME" of dmgFolder to {160, 200}
    set position of item "Applications" of dmgFolder to {500, 200}
end tell
APPLESCRIPT

echo "==> 等待 Finder 索引"
sleep 2

echo "==> 卸载"
hdiutil detach "$MOUNT_POINT" -force >/dev/null
trap - EXIT

echo "==> 转换为压缩只读 DMG"
mkdir -p "$(dirname "$DMG_PATH")"
rm -f "$DMG_PATH"
hdiutil convert -ov -format UDZO "$TMP_DMG" -o "$DMG_PATH" >/dev/null
rm -f "$TMP_DMG"
rm -f "$ATTACH_PLIST"

echo ""
echo "✓ DMG 已生成: $DMG_PATH"
