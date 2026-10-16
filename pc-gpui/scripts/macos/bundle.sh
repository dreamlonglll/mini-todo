#!/usr/bin/env bash
# 把 cargo 构建出的 mini-todo 可执行文件组装成 mini-todo.app，并对整个 bundle 做 ad-hoc 签名（只能在 macOS 上运行）。
#
#   bundle.sh <arm64 或单架构二进制> <x86_64 二进制|-> <输出目录>
#
#   # universal（release.yml / pc-gpui-ci.yml）
#   bash pc-gpui/scripts/macos/bundle.sh \
#     pc-gpui/target/aarch64-apple-darwin/release/mini-todo \
#     pc-gpui/target/x86_64-apple-darwin/release/mini-todo dist
#   # 单架构（e2e：debug 构建也要包成 .app，bundle id 才有效、系统通知等才能用）
#   bash pc-gpui/scripts/macos/bundle.sh pc-gpui/target/debug/mini-todo - "$RUNNER_TEMP/bundle"
#
# 产物：<输出目录>/mini-todo.app/Contents/{MacOS/mini-todo, Resources/icon.icns, Info.plist, _CodeSignature/}
#
# 身份与 2.5.0（Tauri 版）mac 包一致（见 .trellis/spec/pc-gpui/release.md 的 macOS 一节）：
# - CFBundleIdentifier = com.tauri-app.mini-todo（= crates/app/src/lib.rs 的 APP_IDENTIFIER）、.app 名 / CFBundleName /
#   CFBundleExecutable 都是 mini-todo —— 用户把新包拖进「应用程序」时直接替换旧版，通知授权、开机自启 LaunchAgent 的路径都不变
# - 不设 LSUIElement（2.5.0 也没有，Dock 图标照常显示；隐藏 Dock 图标只能用 GPUI 的 set_activation_policy）
# - CFBundleShortVersionString / CFBundleVersion 按 Apple 规定只能是数字和点：取版本号 `-` / `+` 之前的部分（3.0.1-pre → 3.0.1），
#   口径与 Windows 安装器的 VIProductVersion 相同；完整版本号由应用内关于页显示
# - 版本号只从 pc-gpui/Cargo.toml 的 [workspace.package] version 读（唯一来源）
# - 隐私用途说明 NS{Desktop,Documents,Downloads}FolderUsageDescription：桌面收纳箱把桌面上的项目移进收纳箱 / 移回原位置时，
#   万一触发「文件与文件夹」授权框，框里显示的中文说明（正常路径不列举这些受保护目录，不会弹框）。不是身份键，改了不影响升级
#
# 签名：没有 Apple 开发者账号，最后一步 `codesign --force --sign -` 整包 ad-hoc 签名（封 Info.plist / Resources，
# 并重签 lipo 合并后的每个切片）。签完不要再改 bundle 里的任何文件。不开 hardened runtime、不打时间戳（只有公证才需要）。
set -euo pipefail

die() {
  echo "::error::bundle.sh: $*" >&2
  exit 1
}

if [ "$#" -ne 3 ]; then
  echo "用法: $0 <arm64 或单架构二进制> <x86_64 二进制|-> <输出目录>" >&2
  exit 2
fi

BIN_A=$1
BIN_B=$2
OUT_DIR=$3

[ "$(uname -s)" = "Darwin" ] || die "只能在 macOS 上运行（需要 lipo / plutil / codesign）"

# 路径按脚本自身位置解析：可以从任何目录调用
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PC_GPUI_DIR=$(cd "$SCRIPT_DIR/../.." && pwd)
CARGO_TOML="$PC_GPUI_DIR/Cargo.toml"
ICON="$PC_GPUI_DIR/crates/app/assets/icon.icns"

BUNDLE_ID="com.tauri-app.mini-todo"
APP_NAME="mini-todo"

[ -f "$CARGO_TOML" ] || die "找不到 $CARGO_TOML"
[ -f "$ICON" ] || die "找不到图标 $ICON"
[ -f "$BIN_A" ] || die "找不到可执行文件 $BIN_A"
if [ "$BIN_B" != "-" ]; then
  [ -f "$BIN_B" ] || die "找不到可执行文件 $BIN_B"
fi

VERSION=$(awk -F'"' '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f && /^version *=/{print $2; exit}' "$CARGO_TOML")
[ -n "$VERSION" ] || die "读不到 $CARGO_TOML 的 [workspace.package] version"
SHORT="${VERSION%%[-+]*}"
[[ "$SHORT" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]] || die "版本号 $VERSION 的数字部分 '$SHORT' 不是 1~3 段整数"
echo "version: $VERSION (CFBundleShortVersionString / CFBundleVersion = $SHORT)"

mkdir -p "$OUT_DIR"
OUT_DIR=$(cd "$OUT_DIR" && pwd)
APP="$OUT_DIR/$APP_NAME.app"
EXE="$APP/Contents/MacOS/$APP_NAME"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

if [ "$BIN_B" = "-" ]; then
  cp "$BIN_A" "$EXE"
else
  lipo -create -output "$EXE" "$BIN_A" "$BIN_B"
fi
chmod 755 "$EXE"

ARCHS=$(lipo -archs "$EXE")
echo "archs: $ARCHS"
if [ "$BIN_B" != "-" ]; then
  for arch in arm64 x86_64; do
    case " $ARCHS " in
      *" $arch "*) ;;
      *) die "universal 二进制缺少 $arch 切片（lipo -archs: $ARCHS）" ;;
    esac
  done
fi

cp "$ICON" "$APP/Contents/Resources/icon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key>
  <string>${APP_NAME}</string>
  <key>CFBundleName</key>
  <string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key>
  <string>${APP_NAME}</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleShortVersionString</key>
  <string>${SHORT}</string>
  <key>CFBundleVersion</key>
  <string>${SHORT}</string>
  <key>CFBundleDevelopmentRegion</key>
  <string>zh_CN</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSDesktopFolderUsageDescription</key>
  <string>mini-todo 需要访问桌面文件夹，才能把桌面上的项目收进收纳箱或移回桌面。</string>
  <key>NSDocumentsFolderUsageDescription</key>
  <string>mini-todo 需要访问“文稿”文件夹，才能把收纳箱里的项目移回它原来所在的位置。</string>
  <key>NSDownloadsFolderUsageDescription</key>
  <string>mini-todo 需要访问“下载”文件夹，才能把收纳箱里的项目移回它原来所在的位置。</string>
</dict>
</plist>
PLIST

plutil -lint "$APP/Contents/Info.plist"
# 身份自查：读回关键键（plutil -extract 读不到时报错退出）
for key in CFBundleIdentifier CFBundleExecutable CFBundleShortVersionString; do
  echo "Info.plist $key = $(plutil -extract "$key" raw -o - "$APP/Contents/Info.plist")"
done
[ "$(plutil -extract CFBundleIdentifier raw -o - "$APP/Contents/Info.plist")" = "$BUNDLE_ID" ] ||
  die "Info.plist 的 CFBundleIdentifier 不是 $BUNDLE_ID"

# ad-hoc 签名整个 bundle（最后一步）
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv --verbose=4 "$APP" 2>&1 | grep -E '^(Identifier|Format|CodeDirectory|Signature|Sealed Resources|Info.plist)' || true

echo "bundle: $APP"
