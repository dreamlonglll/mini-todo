# 制作功能介绍视频并重写 README

## 背景

README 里的截图来自较早的版本（标题栏等细节与 3.0 不同），正文偏开发者视角、很长；项目没有能直观展示功能的视频。

## 目标

1. 一段介绍 Mini Todo 3.0 功能的视频（中文字幕，无配音），素材全部来自**当前版本的真实应用**（Linux Xvfb 下运行 debug 构建截图 / 录制），
   不用旧截图、不画假界面。
2. 重写根目录 `README.md`：面向用户先讲清楚"是什么、长什么样、怎么装"，功能按模块精简列出，开发 / 云端 API 部分保留要点、细节链到子文档。
3. 视频与截图可以重新生成：生成脚本提交到仓库（`pc-gpui/scripts/intro-video/`），界面改版后一条命令刷新。

## 范围与决定（默认值，用户未另行指定）

- 视频：1920×1080、30fps、H.264（yuv420p，浏览器 / 播放器通用），约 1.5~2.5 分钟，体积尽量 ≤ 15MB；无音轨。
  （实际 2 分 17 秒、12.9MB：为压住渐变色带加了细颗粒与 aq-mode=3，比不加时大约 30%）
  放 `docs/video/mini-todo-intro.mp4`，另出一张封面 `docs/video/poster.png`。
- 视频结构：片头 → 待办列表 / 四象限 / 日历 → 详情与 Markdown、子任务 → 提醒（应用内通知、重复提醒）→ 外观（深浅色）与固定模式
  → 桌面收纳箱 → WebDAV 同步与设置 → 云端 API / Skill → 片尾（下载、平台）。
- 合成方式：HTML/CSS 场景 + 自写补间（`Motion`），Playwright（本机 chromium）逐帧截图，ffmpeg 编码；确定性渲染，不依赖实时播放。
- README 截图换成当前版本：`docs/images/` 下重新生成（主窗口列表 / 四象限 / 日历 / 详情 / 收纳箱 / 设置等），旧的 3 张替换掉。
- README 用 GitHub 能直接显示的方式引用视频：封面图链接到 mp4（GitHub 的 README 不能内嵌仓库里的 mp4 播放器）。
- Linux 上无法展示的平台能力（嵌入桌面、托盘菜单、系统 Toast、macOS 外观）在视频里只用文字说明，不伪造画面。

## 不做

- 不改应用代码与行为。
- 不配音、不加背景音乐（没有可用的授权素材）。
- 不上传 GitHub Release 附件 / user-attachments（工具无此能力），在交付说明里告诉用户如何换成内嵌播放器。

## 验收

- [x] `docs/video/mini-todo-intro.mp4` 可播放，ffprobe 校验 1080p / H.264 / yuv420p，时长与体积在上述范围内
- [x] 抽帧检查：每个场景字幕不溢出、截图清晰、没有黑块 / 错位
- [x] README 中每条功能、平台差异、安装步骤、数据位置与 CLAUDE.md / 代码一致（由 trellis-check 子代理核对）
- [x] README 中的相对链接与图片路径全部存在
- [x] 生成脚本有说明（`pc-gpui/scripts/intro-video/README.md`），并登记到 `pc-gpui/scripts/README.md`
