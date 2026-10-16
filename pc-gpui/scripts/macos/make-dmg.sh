#!/usr/bin/env bash
# 把已签名的 mini-todo.app 做成拖放安装的 dmg（只能在 macOS 上运行）。
#
#   make-dmg.sh <app 路径> <dmg 输出路径>
#   bash pc-gpui/scripts/macos/make-dmg.sh dist/mini-todo.app "dist/mini-todo_${VERSION}_universal.dmg"
#
# - 卷名 mini-todo；卷里是 mini-todo.app 与指向 /Applications 的软链（拖放安装）
# - UDZO（zlib 压缩、只读，与 2.5.0 相同）、HFS+
# - hdiutil 显式给 -size（内容 × 2 + 64 MB）：-srcfolder 自动估算的临时卷偏小时会随机报
#   "No space left on device / could not access /Volumes/…"（mini-term 三次踩到，runner-images#4288 / #7522 / #12323）。
#   失败时 detach 残留的卷、尺寸翻倍、等 15 秒重试，最多 5 次
# - 做完挂载回读校验：codesign --verify --deep --strict、lipo -archs、/Applications 软链、hdiutil verify
set -euo pipefail

die() {
  echo "::error::make-dmg.sh: $*" >&2
  exit 1
}

if [ "$#" -ne 2 ]; then
  echo "用法: $0 <app 路径> <dmg 输出路径>" >&2
  exit 2
fi

[ "$(uname -s)" = "Darwin" ] || die "只能在 macOS 上运行（需要 hdiutil / codesign）"

APP=${1%/}
DMG=$2
VOLNAME="mini-todo"

[ -d "$APP" ] || die "找不到 $APP"
APP_BASENAME=$(basename "$APP")
case "$APP_BASENAME" in
  *.app) ;;
  *) die "$APP 不是 .app 目录" ;;
esac
EXE="$APP/Contents/MacOS/$(plutil -extract CFBundleExecutable raw -o - "$APP/Contents/Info.plist")"
[ -x "$EXE" ] || die "找不到可执行文件 $EXE"
# 打包前先确认签名完好（bundle.sh 签完之后不能再改 bundle）
codesign --verify --deep --strict "$APP" || die "$APP 的签名校验失败"

TMP_PARENT="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
STAGE=$(mktemp -d "$TMP_PARENT/dmg-root.XXXXXX")
MNT=""

cleanup() {
  if [ -n "$MNT" ] && [ -d "$MNT" ]; then
    hdiutil detach "$MNT" -force >/dev/null 2>&1 || true
    rmdir "$MNT" 2>/dev/null || true
  fi
  rm -rf "$STAGE"
}
trap cleanup EXIT

# ditto 而不是 cp：保留签名、扩展属性与软链
ditto "$APP" "$STAGE/$APP_BASENAME"
ln -s /Applications "$STAGE/Applications"

DMG_DIR=$(dirname "$DMG")
mkdir -p "$DMG_DIR"
DMG="$(cd "$DMG_DIR" && pwd)/$(basename "$DMG")"
rm -f "$DMG"

echo "::group::disk usage"
df -h "$DMG_DIR" "$TMP_PARENT" || true
du -sh "$STAGE" || true
echo "::endgroup::"

SIZE_MB=$(( $(du -sm "$STAGE" | cut -f1) * 2 + 64 ))
attempt=1
while true; do
  echo "hdiutil create（第 $attempt 次，size=${SIZE_MB}m）"
  if hdiutil create -volname "$VOLNAME" -srcfolder "$STAGE" -fs HFS+ -format UDZO \
       -size "${SIZE_MB}m" -ov "$DMG"; then
    break
  fi
  echo "::warning::hdiutil create 第 $attempt 次失败（size=${SIZE_MB}m）"
  if [ "$attempt" -ge 5 ]; then
    die "hdiutil create 连续 5 次失败"
  fi
  # 上一次失败可能留下挂着的临时卷（/Volumes/mini-todo、/Volumes/mini-todo 1 …）
  for v in "/Volumes/$VOLNAME" "/Volumes/$VOLNAME "[0-9]*; do
    if [ -d "$v" ]; then
      hdiutil detach "$v" -force >/dev/null 2>&1 || true
    fi
  done
  rm -f "$DMG"
  SIZE_MB=$(( SIZE_MB * 2 ))
  attempt=$(( attempt + 1 ))
  sleep 15
done

hdiutil verify "$DMG"

# 挂载回读校验
MNT=$(mktemp -d "$TMP_PARENT/dmg-mnt.XXXXXX")
hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$MNT" "$DMG"
ls -la "$MNT"
codesign --verify --deep --strict --verbose=2 "$MNT/$APP_BASENAME"
MNT_EXE="$MNT/$APP_BASENAME/Contents/MacOS/$(basename "$EXE")"
[ -x "$MNT_EXE" ] || die "dmg 里找不到可执行文件 $MNT_EXE"
echo "dmg archs: $(lipo -archs "$MNT_EXE")"
[ -L "$MNT/Applications" ] || die "dmg 里没有 Applications 软链"
[ "$(readlink "$MNT/Applications")" = "/Applications" ] || die "Applications 软链没有指向 /Applications"
cmp -s "$EXE" "$MNT_EXE" || die "dmg 里的可执行文件与 $EXE 不一致"
hdiutil detach "$MNT" || hdiutil detach "$MNT" -force
rmdir "$MNT" 2>/dev/null || true
MNT=""

ls -la "$DMG"
shasum -a 256 "$DMG"
echo "dmg: $DMG"
