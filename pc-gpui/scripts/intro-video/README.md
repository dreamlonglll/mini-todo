# 介绍视频与 README 截图

`docs/video/mini-todo-intro.mp4`（功能介绍视频）与 `docs/images/*.png`（README 截图）都由这里的脚本从**当前版本的真实应用**生成：
Linux 的 Xvfb 下以 2 倍缩放运行 debug 构建，截带透明通道的窗口图、录几段真实交互，再用 HTML 时间轴合成，Playwright 逐帧截图交给 ffmpeg 编码。
界面改版后按下面三步重新生成即可，不需要手工截图。

| 文件 | 作用 |
|---|---|
| `capture.py` | 采集素材：复用 `../regress.py` 的场景（样例数据、打开方式、交互坐标都一样，坐标仍按 1 倍逻辑像素写），输出到 `target/intro-video/assets/` |
| `video.html` / `video.js` | 合成页面：1920×1080 舞台、场景时间轴（`scene(...)`）、补间（`Motion`）、录屏回放与光标重画（`ClipPlayer`）、README 截图的静态合成（`STILLS`） |
| `render.cjs` | 渲染：逐帧 `window.seek(t)` 截图 → ffmpeg（H.264 / yuv420p / faststart）；也能只截几帧检查，或输出 README 截图 |

## 依赖

Ubuntu：`xvfb xdotool x11-apps imagemagick ffmpeg mesa-vulkan-drivers fonts-noto-cjk fonts-liberation fonts-inter pngquant`（在 `regress.py` 的依赖之外另加
`x11-apps` 里的 `xwd`、`ffmpeg`（含 `ffprobe`）与合成页面用的 Inter 字体）；Node 18+ 与 `playwright`（全局或本地安装均可，用 Playwright 自带的 Chromium）。

## 用法

```bash
cd pc-gpui
cargo build -p mini-todo && cargo build -p mini-todo-core --example seed

# 1. 采集素材（约 5 分钟）→ target/intro-video/assets/
python3 scripts/intro-video/capture.py --no-build            # --only 'clip-*' 只重录片段，--only 'main-*' 只重截主窗口

# 2. 渲染视频（4 核约 5 分钟）→ target/intro-video/mini-todo-intro.mp4
node scripts/intro-video/render.cjs                            # --jobs N：按时间切成 N 段并行渲染（默认 3），最后无损拼接
node scripts/intro-video/render.cjs --frames 8,42,100          # 只截这几秒的画面到 target/intro-video/frames/（改时间轴时先看这个）
node scripts/intro-video/render.cjs --from 36 --to 60 --out /tmp/part.mp4   # 只渲染一段

# 3. README 截图 → target/intro-video/stills/（提交前用 pngquant 压一下，体积约降到 1/4，肉眼看不出差别）
node scripts/intro-video/render.cjs --still views --still detail --still desktop --still settings
(cd target/intro-video/stills && for f in *.png; do pngquant --quality 80-95 --speed 1 --force --ext .png "$f"; done)

# 发布到仓库
mkdir -p ../docs/video
cp target/intro-video/mini-todo-intro.mp4 ../docs/video/
cp target/intro-video/stills/{views,detail,desktop,settings}.png ../docs/images/
cp target/intro-video/assets/icon.png ../docs/images/icon.png
```

README 顶部的播放器用的是 GitHub 附件链接（`https://github.com/user-attachments/assets/…`），不会随仓库里的 mp4 自动更新：
重新生成视频后，在 GitHub 网页上把新的 mp4 拖进任意 Issue / PR 编辑框（不用提交），复制生成的链接替换 README 里的旧链接（顶部导航与正文各一处）。

直接在浏览器里看 README 截图：`chromium --allow-file-access-from-files 'file://…/video.html?still=views'`。
时间轴（`?t=42` 看第 42 秒）要用到 `render.cjs` 注入的录屏片段元数据，从浏览器直接打开会因为"缺少片段"整页报错，看视频画面用 `--frames`。

## 编码

逐帧截 JPEG（质量 95）经管道交给 ffmpeg：libx264 `-preset slow -crf 26 -tune animation`、`yuv420p`、`+faststart`，另加 `aq-mode=3`。
大面积蓝色渐变在 8 位 H.264 里会出明显色带，合成页面在每个场景的背景上叠了一层极淡的固定噪点（`.grain`），配合 `aq-mode=3` 压住色带；
去掉其中任何一个，片头片尾和壁纸场景的色带都会回来。整段 2 分 17 秒约 9 MB：GitHub 免费账户往 Issue / README 编辑框里拖视频的上限是 10 MB，
`--crf 23` 约 13 MB 会上传失败，加长视频后注意别超过这个上限。

## 素材采集的几个要点

- **2 倍缩放**（`GPUI_X11_SCALE_FACTOR=2`，Xvfb 3840×2160）：素材在 1080p 视频里缩小显示，文字清晰。
- **透明通道**：Xvfb 没有合成器，`import` 截出来透明处是黑的；`capture.py` 用 `xwd` 读 32 位窗口的原始像素（BGRA，GPUI 输出的是预乘 alpha），
  还原成直通 alpha 再写 PNG，所以深色半透明主题、收纳箱能叠在视频里的壁纸上。
- **录屏片段**（`("rec", 名字, 窗口标题)` … `("stop",)`）：ffmpeg x11grab 录窗口区域，不画系统光标；`-copyts` 让帧时间戳是墙钟时间，
  与 xdotool 操作时记下的光标轨迹对齐，视频里的光标与点击涟漪按轨迹重画。录屏的黑角用开始录制时的透明窗口图做遮罩去掉。
- **关键帧片段**（`("krec", …)` / `("snap",)` / `("kstop",)`）：半透明窗口（收纳箱）录屏拿不到透明度，改为在每个状态拍一张透明截图，
  拍照耗时不计入片段时间。
- Xvfb 必须带 `-noreset`：最后一个客户端断开时 X 服务器会重置并把光标放回屏幕中央，下一个场景的窗口就带着悬停样式出现。
- xdotool 的限制：`type` 只输入 ASCII（中文会丢字），以 `-` 开头的文字会被当成选项（`capture.py` 先单独按一次减号）。
- 待办要按住颜色圆点（拖动手柄，应用里唯一的拖动入口）才能拖动；Linux 上 GPUI 移动不了窗口，收纳箱的吸附对齐是用两张截图做的动画演示，不是录屏。
- 视频里的时间点尽量从片段的事件（第几次点击）推出来（`clip.eventTime("down", n)`），重录片段后不用手改时间轴。
