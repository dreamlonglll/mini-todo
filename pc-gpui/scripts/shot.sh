#!/usr/bin/env bash
# Xvfb + lavapipe 下运行真实的 GPUI 应用（debug 构建、隔离数据目录、样例数据），按 MINI_TODO_DEBUG_OPEN 打开窗口，
# 执行可选的 xdotool 交互后截图。用法与参数见同目录 README.md。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"

OPEN_SPEC="main"
NAME="shot"
WINDOW_TITLE=""
DATA_JSON="$SCRIPT_DIR/sample-data.json"
DATA_DIR=""
BUILD=1
DISPLAY_NUM=":94"
SCREEN_SIZE="1920x1080x24"
WAIT_SECS=4
KEEP=0
OUT_DIR=""
ACTIONS=()

usage() {
  sed -n '/^## shot.sh/,/^## /p' "$SCRIPT_DIR/README.md" | sed '$d'
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --open) OPEN_SPEC="$2"; shift 2 ;;
    --name) NAME="$2"; shift 2 ;;
    --window) WINDOW_TITLE="$2"; shift 2 ;;
    --data) DATA_JSON="$2"; shift 2 ;;
    --data-dir) DATA_DIR="$2"; shift 2 ;;
    --no-build) BUILD=0; shift ;;
    --display) DISPLAY_NUM="$2"; shift 2 ;;
    --screen) SCREEN_SIZE="$2"; shift 2 ;;
    --wait) WAIT_SECS="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --hover|--click|--rclick|--dblclick|--key|--type|--sleep|--target)
      ACTIONS+=("$1" "$2"); shift 2 ;;
    -h|--help) usage 0 ;;
    -*) echo "未知参数: $1" >&2; usage 1 ;;
    *) OUT_DIR="$1"; shift ;;
  esac
done
[[ -n "$OUT_DIR" ]] || { echo "缺少输出目录" >&2; usage 1; }
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

for tool in Xvfb xdotool import convert; do
  command -v "$tool" >/dev/null || { echo "缺少命令: $tool" >&2; exit 1; }
done

if [[ $BUILD -eq 1 ]]; then
  (cd "$WORKSPACE" && cargo build -p mini-todo && cargo build -p mini-todo-core --example seed)
fi
APP="$WORKSPACE/target/debug/mini-todo"
SEED="$WORKSPACE/target/debug/examples/seed"

# ---- 隔离数据目录 + 样例数据 ----
CLEAN_DATA=0
if [[ -z "$DATA_DIR" ]]; then
  DATA_DIR="$(mktemp -d -t mini-todo-shot.XXXXXX)"
  CLEAN_DATA=1
  "$SEED" "$DATA_DIR" "$DATA_JSON"
  # 子任务 Markdown 引用的示例图（与参考截图 harness 的柱状图相同的数据）
  mkdir -p "$DATA_DIR/images"
  CHART_ARGS=(-size 480x240 xc:'#F8FAFC' -stroke '#CBD5E1' -draw 'line 40,200 464,200' -stroke none -fill '#3B82F6')
  x=48
  for v in 42 68 55 80 73 91; do
    bh=$(( v * 160 / 100 ))
    CHART_ARGS+=(-draw "roundrectangle $x,$(( 200 - bh )) $(( x + 48 )),200 4,4")
    x=$(( x + 72 ))
  done
  convert "${CHART_ARGS[@]}" "$DATA_DIR/images/sample-chart.png" 2>/dev/null || true
fi

window_id() { # 按标题精确匹配（xdotool 的正则对中文不可靠，逐个比较标题）
  local wid
  for wid in $(xdotool search --onlyvisible --name '.' 2>/dev/null || true); do
    if [[ "$(xdotool getwindowname "$wid" 2>/dev/null || true)" == "$1" ]]; then
      echo "$wid"
      return
    fi
  done
}

# ---- Xvfb ----
Xvfb "$DISPLAY_NUM" -screen 0 "$SCREEN_SIZE" -nolisten tcp >/dev/null 2>&1 &
XVFB_PID=$!
APP_PID=""
cleanup() {
  [[ -n "$APP_PID" ]] && kill "$APP_PID" 2>/dev/null || true
  kill "$XVFB_PID" 2>/dev/null || true
  [[ $CLEAN_DATA -eq 1 && $KEEP -eq 0 ]] && rm -rf "$DATA_DIR"
}
trap cleanup EXIT
for _ in $(seq 1 100); do DISPLAY="$DISPLAY_NUM" xdotool getdisplaygeometry >/dev/null 2>&1 && break; sleep 0.1; done

export DISPLAY="$DISPLAY_NUM"
unset WAYLAND_DISPLAY
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
export GPUI_X11_SCALE_FACTOR=1
export TZ=Asia/Shanghai
export MINI_TODO_DATA_DIR="$DATA_DIR"
export MINI_TODO_FENCES_ROOT="$DATA_DIR/fences"
export MINI_TODO_DEBUG_OPEN="$OPEN_SPEC"

"$APP" >"$OUT_DIR/$NAME-app.log" 2>&1 &
APP_PID=$!

# 等主窗口出现，再给渲染 / 数据加载留时间
for _ in $(seq 1 150); do
  xdotool search --onlyvisible --name '^Mini Todo$' >/dev/null 2>&1 && break
  sleep 0.1
done
sleep "$WAIT_SECS"
# Xvfb 初始光标在屏幕中央，会让主窗口中间那一行显示悬停样式；GPUI 在光标移出窗口后不清除悬停，
# 所以把光标停在主窗口标题栏的空白处（版本号下方，没有悬停样式）
MAIN_WID="$(window_id "Mini Todo")"
if [[ -n "$MAIN_WID" ]]; then
  eval "$(xdotool getwindowgeometry --shell "$MAIN_WID")"
  # 先移进一行（刷新悬停状态），再停到空白处
  xdotool mousemove $(( X + 200 )) $(( Y + 135 )) 2>/dev/null || true
  sleep 0.3
  xdotool mousemove $(( X + 100 )) $(( Y + 45 )) 2>/dev/null || true
fi
sleep 0.5


TARGET="${WINDOW_TITLE:-Mini Todo}"
run_action() {
  local kind="$1" arg="$2" wid gx gy
  if [[ "$kind" == "--target" ]]; then TARGET="$arg"; return; fi
  if [[ "$kind" == "--sleep" ]]; then sleep "$arg"; return; fi
  wid="$(window_id "$TARGET")"
  [[ -n "$wid" ]] || { echo "找不到窗口: $TARGET" >&2; return; }
  eval "$(xdotool getwindowgeometry --shell "$wid")"
  gx=$X; gy=$Y
  case "$kind" in
    --hover) xdotool mousemove $(( gx + ${arg%,*} )) $(( gy + ${arg#*,} )) ;;
    --click) xdotool mousemove $(( gx + ${arg%,*} )) $(( gy + ${arg#*,} )) click 1 ;;
    --rclick) xdotool mousemove $(( gx + ${arg%,*} )) $(( gy + ${arg#*,} )) click 3 ;;
    --dblclick) xdotool mousemove $(( gx + ${arg%,*} )) $(( gy + ${arg#*,} )) click --repeat 2 1 ;;
    --key) xdotool key --window "$wid" "$arg" ;;
    --type) xdotool type --window "$wid" "$arg" ;;
  esac
  sleep 0.6
}
for (( i = 0; i < ${#ACTIONS[@]}; i += 2 )); do
  run_action "${ACTIONS[i]}" "${ACTIONS[i+1]}"
done
sleep 0.5

# ---- 截图：整屏一张，再按窗口裁剪 ----
import -window root "$OUT_DIR/$NAME-screen.png"
slug_of() {
  case "$1" in
    "Mini Todo") echo main ;;
    "待办详情"|"新建待办") echo editor ;;
    "查看子任务"|"编辑子任务") echo subtask ;;
    "设置") echo settings ;;
    "已完成") echo completed ;;
    "通知") echo notification ;;
    收纳箱*) echo fence ;;
    *) echo "window" ;;
  esac
}
declare -A SEEN=()
for wid in $(xdotool search --onlyvisible --name '.' 2>/dev/null); do
  title="$(xdotool getwindowname "$wid" 2>/dev/null || true)"
  [[ -n "$title" ]] || continue
  if [[ -n "$WINDOW_TITLE" && "$title" != "$WINDOW_TITLE" ]]; then continue; fi
  eval "$(xdotool getwindowgeometry --shell "$wid")"
  slug="$(slug_of "$title")"
  n="${SEEN[$slug]:-0}"; SEEN[$slug]=$(( n + 1 ))
  [[ $n -gt 0 ]] && slug="$slug-$n"
  convert "$OUT_DIR/$NAME-screen.png" -crop "${WIDTH}x${HEIGHT}+${X}+${Y}" +repage "$OUT_DIR/$NAME-$slug.png"
  echo "$OUT_DIR/$NAME-$slug.png  ($title ${WIDTH}x${HEIGHT}+${X}+${Y})"
done

if [[ $KEEP -eq 1 ]]; then
  echo "--keep：应用仍在 $DISPLAY_NUM 上运行（数据目录 $DATA_DIR），按回车结束"
  read -r _
fi
