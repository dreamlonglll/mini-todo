// 介绍视频的时间轴：每个场景在 [start, start + dur] 内显示，元素的位移 / 缩放 / 透明度由 Motion 按时间插值，
// 录屏片段按帧号换图、光标按采集时记下的轨迹重画。window.seek(t) 把整个舞台摆到 t 秒（返回 Promise，图片解码完才 resolve）。
"use strict";

const W = 1920;
const H = 1080;
const params = new URLSearchParams(location.search);
const ASSETS = (window.INTRO_ASSETS || params.get("assets") || "../../target/intro-video/assets").replace(/\/$/, "");
const CLIPS = window.INTRO_CLIPS || {}; // render.cjs 注入：片段名 → capture.py 写的 meta
const XFADE = 0.6; // 场景交叉淡入时长

// ---------------------------------------------------------------------------
// 基础
// ---------------------------------------------------------------------------

const EASE = {
  linear: (p) => p,
  out: (p) => 1 - Math.pow(1 - p, 3),
  in: (p) => p * p * p,
  inout: (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2),
  back: (p) => 1 + 2.2 * Math.pow(p - 1, 3) + 1.2 * Math.pow(p - 1, 2),
};

function el(tag, cls, parent, html) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (html !== undefined) node.innerHTML = html;
  if (parent) parent.appendChild(node);
  return node;
}

function css(node, styles) {
  Object.assign(node.style, styles);
  return node;
}

const motions = [];

/** 一个元素的分段补间：x / y / s（缩放）/ o（透明度）/ b（模糊），段与段首尾相接、按开始时间排列 */
class Motion {
  constructor(node, init = {}) {
    this.node = node;
    this.init = Object.assign({ x: 0, y: 0, s: 1, o: 1 }, init);
    this.segs = [];
    motions.push(this);
  }
  to(t0, dur, props, ease = "out") {
    this.segs.push({ t0, dur, props, ease });
    return this;
  }
  value(t) {
    let cur = Object.assign({}, this.init);
    for (const g of this.segs) {
      if (t < g.t0) break;
      const p = g.dur > 0 ? Math.min(1, (t - g.t0) / g.dur) : 1;
      const e = EASE[g.ease](p);
      const next = Object.assign({}, cur);
      for (const k in g.props) next[k] = cur[k] + (g.props[k] - cur[k]) * e;
      cur = next;
    }
    return cur;
  }
  apply(t) {
    const v = this.value(t);
    const st = this.node.style;
    st.opacity = v.o;
    st.transform = `translate(${v.x}px, ${v.y}px) scale(${v.s})`;
    if ("b" in this.init) st.filter = v.b > 0.05 ? `blur(${v.b}px)` : "none";
  }
}

const sizes = {}; // 图片名 → [w, h]

function loadImage(src) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error(`加载失败：${src}`));
    img.src = src;
  });
}

function asset(name) {
  return `${ASSETS}/${name}`;
}

// ---------------------------------------------------------------------------
// 窗口、录屏片段、光标
// ---------------------------------------------------------------------------

/** 一张窗口截图：left/top 是舞台坐标，scale 是相对素材（2 倍像素）的缩放 */
function windowShot(parent, name, { x, y, scale, kind = "alpha", desk = false }) {
  const [w, h] = sizes[name];
  const box = el("div", `win ${kind}${desk ? " on-desk" : ""}`, parent);
  const img = el("img", "", box);
  img.src = asset(name);
  css(img, { width: `${w}px`, height: `${h}px` });
  css(box, { width: `${w}px`, height: `${h}px` });
  const m = new Motion(box, { x, y, s: scale });
  return { box, img, motion: m, w, h, x, y, scale };
}

const CURSOR_SVG = `<svg viewBox="0 0 24 36" xmlns="http://www.w3.org/2000/svg">
  <path d="M2 2 L2 28 L8.5 21.5 L13 32 L17.5 30 L13 19.8 L22 19.8 Z" fill="#fff" stroke="#111827" stroke-width="2" stroke-linejoin="round"/></svg>`;

const players = [];

/**
 * 录屏片段（capture.py 的 clip）：frames 模式按帧号换 JPEG、用开始录屏时的透明窗口图做遮罩；
 * keyframes 模式（半透明窗口，如收纳箱）在各个状态的透明截图之间切换。光标都按记录的轨迹重画。
 */
class ClipPlayer {
  constructor(parent, name, { x, y, scale, kind = "alpha", desk = false, cursorPx = 34 }) {
    const meta = CLIPS[name];
    if (!meta) throw new Error(`缺少片段 ${name}（先跑 capture.py）`);
    this.meta = meta;
    this.name = name;
    this.box = el("div", `win ${kind}${desk ? " on-desk" : ""}`, parent);
    this.clip = el("div", "clip", this.box);
    css(this.box, { width: `${meta.width}px`, height: `${meta.height}px` });
    css(this.clip, { width: `${meta.width}px`, height: `${meta.height}px` });
    if (meta.mode !== "keyframes") {
      const mask = `url("${asset(`clips/${name}-mask.png`)}")`;
      css(this.clip, { webkitMaskImage: mask, maskImage: mask, webkitMaskSize: "100% 100%", maskSize: "100% 100%" });
    }
    this.img = el("img", "", this.clip);
    css(this.img, { width: `${meta.width}px`, height: `${meta.height}px` });
    this.ripples = [el("div", "ripple", this.clip), el("div", "ripple", this.clip)];
    this.cursor = el("div", "cursor", this.clip, CURSOR_SVG);
    this.cursorScale = (cursorPx / 50) / scale;
    this.motion = new Motion(this.box, { x, y, s: scale });
    this.segs = [];
    this.cursorOn = true;
    this.cursorLinger = Infinity; // 播完后光标再停留多久（秒）
    this.src = "";
    this.x = x;
    this.y = y;
    this.scale = scale;
    players.push(this);
  }
  /** 场景时间 [t0, t1) 内播放片段时间 c0 起、rate 倍速 */
  play(t0, t1, c0 = 0, rate = 1) {
    this.segs.push({ t0, t1, c0, rate });
    return this;
  }
  /** 从场景时间 t0 起依次播放 [[c0, c1, rate], …]，返回播完的场景时间（之后停在最后一帧） */
  playSegments(t0, list) {
    let t = t0;
    for (const [c0, c1, rate] of list) {
      const t1 = t + (c1 - c0) / rate;
      this.play(t, t1, c0, rate);
      t = t1;
    }
    return t;
  }
  /** 片段时长（秒） */
  get length() {
    return this.meta.mode === "keyframes" ? this.meta.duration : this.meta.frames / this.meta.fps;
  }
  clipTime(t) {
    let ct = this.segs.length ? this.segs[0].c0 : 0;
    for (const s of this.segs) {
      if (t < s.t0) break;
      ct = s.c0 + (Math.min(t, s.t1) - s.t0) * s.rate;
    }
    return Math.max(0, Math.min(ct, this.length - 1e-3));
  }
  /** 第 n 个指定类型事件的片段时间（找交互的时间点用） */
  eventTime(kind, n = 0) {
    const list = this.meta.events.filter((e) => e.kind === kind);
    if (!list[n]) throw new Error(`${this.name} 没有第 ${n} 个 ${kind} 事件`);
    return list[n].t;
  }
  /** 片段时间 → 场景时间（按第一段 play 换算） */
  sceneTime(ct) {
    for (const s of this.segs) {
      const t = s.t0 + (ct - s.c0) / s.rate;
      if (t >= s.t0 - 1e-6 && t <= s.t1 + 1e-6) return t;
    }
    throw new Error(`${this.name}：片段时间 ${ct} 不在任何播放段内`);
  }
  pointer(ct) {
    const ev = this.meta.events;
    let prev = ev[0];
    for (let i = 0; i < ev.length; i++) {
      const e = ev[i];
      if (e.t > ct) {
        if (prev && prev !== e && e.kind === "move" && prev.t < ct) {
          const p = (ct - prev.t) / (e.t - prev.t);
          return [prev.x + (e.x - prev.x) * p, prev.y + (e.y - prev.y) * p];
        }
        break;
      }
      prev = e;
    }
    return prev ? [prev.x, prev.y] : [-100, -100];
  }
  frameSrc(ct) {
    const m = this.meta;
    if (m.mode === "keyframes") {
      let shot = m.shots[0];
      for (const s of m.shots) if (s.t <= ct) shot = s;
      return asset(`clips/${this.name}/${shot.file}`);
    }
    const n = Math.min(m.frames, Math.max(1, Math.floor(ct * m.fps) + 1));
    return asset(`clips/${this.name}/${String(n).padStart(4, "0")}.jpg`);
  }
  seek(t) {
    const ct = this.clipTime(t);
    const src = this.frameSrc(ct);
    const [px, py] = this.pointer(ct);
    const lastEnd = this.segs.length ? this.segs[this.segs.length - 1].t1 : Infinity;
    css(this.cursor, {
      transform: `translate(${px - 2}px, ${py - 2}px) scale(${this.cursorScale})`,
      // 片段播完后光标不再留在画面上（之后的演示可能另画光标）
      display: this.cursorOn && t <= lastEnd + this.cursorLinger ? "block" : "none",
    });
    // 点击涟漪：最近两次按下
    const downs = this.meta.events.filter((e) => (e.kind === "down" || e.kind === "rdown") && e.t <= ct).slice(-2);
    this.ripples.forEach((r, i) => {
      const e = downs[downs.length - 1 - i];
      const age = e ? ct - e.t : 9;
      if (age >= 0 && age < 0.55) {
        const p = age / 0.55;
        css(r, { left: `${e.x}px`, top: `${e.y}px`, opacity: (1 - p) * 0.9, transform: `scale(${(0.35 + p * 0.9) * this.cursorScale * 1.4})` });
      } else {
        css(r, { opacity: 0 });
      }
    });
    if (src !== this.src) {
      this.src = src;
      this.img.src = src;
      return this.img.decode().catch(() => {});
    }
    return null;
  }
}

/** 窗口里的逻辑坐标（应用里的 1 倍像素；素材是 2 倍）→ 舞台坐标，窗口摆在 (x, y)、缩放 scale */
function at(win, lx, ly) {
  return [win.x + lx * 2 * win.scale, win.y + ly * 2 * win.scale];
}

// ---------------------------------------------------------------------------
// 文字
// ---------------------------------------------------------------------------

/** 左侧文案：kicker + 标题 + 要点，依次滑入；返回所有 Motion 以便整体淡出 */
function copyBlock(parent, t0, { num, kicker, title, bullets = [], note, dark = false, left = 120, width = 700, gap = 0.32 }) {
  const box = el("div", `copy${dark ? " on-dark" : ""}`, parent);
  css(box, { left: `${left}px`, width: `${width}px` });
  const items = [];
  const k = el("div", "kicker", box, `${num ? `<span class="num">${num}</span>` : ""}<span>${kicker}</span>`);
  items.push(k);
  items.push(el("div", "title", box, title));
  const ul = el("ul", "bullets", box);
  for (const b of bullets) items.push(el("li", "", ul, b));
  if (note) items.push(el("div", "note", box, note));
  const ms = items.map((node, i) => {
    const start = i < 2 ? t0 + i * 0.15 : t0 + 0.45 + (i - 2) * gap;
    return new Motion(node, { x: -36, o: 0 }).to(start, 0.7, { x: 0, o: 1 });
  });
  return { box, motions: ms };
}

/** 底部字幕（镜头推近、文案让开时说明正在发生什么） */
function caption(parent, text, t0, t1, dark = false) {
  const wrap = el("div", "caption-wrap", parent);
  const pill = el("div", `caption${dark ? " dark" : ""}`, wrap, text);
  const m = new Motion(pill, { y: 20, o: 0 }).to(t0, 0.45, { y: 0, o: 1 });
  if (t1) m.to(t1, 0.35, { o: 0, y: 10 }, "in");
  return pill;
}

function fadeOut(list, t0, dur = 0.4) {
  for (const m of list) m.to(t0, dur, { o: 0, x: -24 }, "in");
}

function tag(parent, text, x, y, t0, t1, cls = "") {
  const node = el("div", `tag ${cls}`, parent, text);
  const m = new Motion(node, { x, y: y + 14, o: 0 }).to(t0, 0.45, { y, o: 1 });
  if (t1) m.to(t1, 0.35, { o: 0 }, "in");
  return node;
}

function ring(parent, x, y, w, h, t0, t1) {
  const node = el("div", "ring", parent);
  css(node, { width: `${w}px`, height: `${h}px`, transformOrigin: "50% 50%" });
  const m = new Motion(node, { x, y, o: 0, s: 1.12 }).to(t0, 0.45, { o: 1, s: 1 });
  if (t1) m.to(t1, 0.35, { o: 0 }, "in");
  return node;
}

// ---------------------------------------------------------------------------
// 场景
// ---------------------------------------------------------------------------

const scenes = [];

function scene(dur, bg, build) {
  scenes.push({ dur, bg, build });
}

/** 窗口从右侧滑入淡入 */
function enter(m, t, x, y, s, dx = 60) {
  m.init = { x: x + dx, y, s, o: 0 };
  m.to(t, 0.8, { x, o: 1 });
  return m;
}

// 00 片头
scene(5.6, "bg-hero", (root, s) => {
  const hero = el("div", "hero", root);
  const icon = el("img", "icon", hero);
  icon.src = asset("icon.png");
  icon.style.transformOrigin = "50% 50%";
  const h1 = el("h1", "", hero, "Mini Todo");
  const tl = el("div", "tagline", hero, "简洁、聚焦的桌面待办");
  const chips = el("div", "chips", hero);
  const chipList = ["Windows 10 / 11", "macOS 11+（预览）", "原生 GPUI 渲染", "无需 WebView2", "MIT 开源"].map((c) => el("div", "chip", chips, c));
  new Motion(icon, { s: 0.6, o: 0, y: 30 }).to(s + 0.2, 0.9, { s: 1, o: 1, y: 0 }, "back");
  new Motion(h1, { o: 0, y: 30 }).to(s + 0.6, 0.8, { o: 1, y: 0 });
  new Motion(tl, { o: 0, y: 24 }).to(s + 0.9, 0.8, { o: 1, y: 0 });
  chipList.forEach((c, i) => new Motion(c, { o: 0, y: 20 }).to(s + 1.4 + i * 0.12, 0.6, { o: 1, y: 0 }));
});

// 01 列表 + 02 四象限（同一段录屏：悬停 → 展开子任务 → 切到四象限 → 按住圆点跨象限拖动 → 切回列表）
scene(21, "bg-studio", (root, s) => {
  const win = { x: 1090, y: 60, scale: 0.8 };
  const clip = new ClipPlayer(root, "clip-main", win);
  enter(clip.motion, s, win.x, win.y, win.scale);
  const end = clip.playSegments(s + 1.0, [[0, clip.length, 1]]);
  const tView = clip.sceneTime(clip.eventTime("down", 1)); // 「切换到四象限视图」
  const tDrag = clip.sceneTime(clip.eventTime("down", 2));
  const tDrop = clip.sceneTime(clip.eventTime("up", 2));
  const switchAt = tView - 0.7;
  const a = copyBlock(root, s + 0.3, {
    num: "01",
    kicker: "待办列表",
    title: "一眼看清<br>今天要做什么",
    bullets: [
      "优先级颜色区分<b>轻重缓急</b>",
      "开始 / 截止时间、提醒、重复一目了然",
      "子任务就地展开，<b>进度</b>实时计数",
      "拖拽排序，重要的事一键<b>置顶</b>",
    ],
  });
  fadeOut(a.motions, switchAt);
  copyBlock(root, switchAt + 0.4, {
    num: "02",
    kicker: "四象限",
    title: "按重要与紧急<br>分配精力",
    bullets: [
      "标题栏一键切换 <b>列表 / 四象限</b>",
      "按住圆点拖到另一个象限，<b>分类随之改变</b>",
      "按「<b>重要 × 紧急</b>」分成四格，先做最要紧的",
    ],
  });
  ring(root, ...at(win, 186, 8), 38 * 2 * win.scale, 38 * 2 * win.scale, tView - 0.9, tView + 0.5);
  const [, dy] = at(win, 0, 300);
  tag(root, "拖到「重要且紧急」", win.x - 300, dy, tDrag - 0.1, tDrop + 1.6, "blue");
  return { minDur: end - s + 0.6 };
});

// 03 日历（宽窗口：放大后从右边出血）
scene(10.5, "bg-studio", (root, s) => {
  const x = 700;
  const y = 168;
  const scale = 0.62;
  const w = windowShot(root, "main-calendar.png", { x, y, scale });
  enter(w.motion, s, x, y, scale);
  w.motion.to(s + 1.2, 9, { s: 0.665, x: 660, y: 140 }, "inout");
  copyBlock(root, s + 0.3, {
    num: "03",
    kicker: "日历",
    title: "把计划<br>摊在月历上",
    bullets: ["跨天待办显示为<b>横条</b>，点击看详情", "<b>农历</b>、节气、传统节日", "<b>法定节假日</b>与调休上班日"],
    width: 520,
  });
});

// 04 详情与 Markdown（录屏：只读详情 → [编辑] → 单击描述改源码 → 放大编辑的分栏预览；镜头推近、字幕说明）
scene(20, "bg-studio", (root, s) => {
  const home = { x: 800, y: 250, scale: 0.5 };
  const clip = new ClipPlayer(root, "clip-editor", { ...home, kind: "opaque", cursorPx: 30 });
  enter(clip.motion, s, home.x, home.y, home.scale);
  const t0 = s + 0.8;
  const end = clip.playSegments(t0, [
    [0, 8.0, 1],
    [8.0, 12.9, 1.25],
    [12.9, 16.0, 1.8],
    [16.0, 19.0, 1],
  ]);
  const ct = (c) => clip.sceneTime(c);
  const tEdit = ct(clip.eventTime("down", 0));
  const tDesc = ct(clip.eventTime("down", 1));
  const tBig = ct(clip.eventTime("down", 2));
  const intro = copyBlock(root, s + 0.3, { num: "04", kicker: "详情与 Markdown", title: "写得下细节，<br>也读得舒服", width: 620 });
  // 推近到左侧表单（逻辑坐标 (330, 300) 落在舞台中央偏下）
  const zoom = 0.86;
  const fx = 960 - 330 * 2 * zoom;
  const fy = 560 - 300 * 2 * zoom;
  fadeOut(intro.motions, tEdit - 1.0);
  clip.motion.to(tEdit - 1.0, 1.0, { x: fx, y: fy, s: zoom }, "inout");
  caption(root, "点「编辑」，详情原地切换为编辑表单", tEdit - 0.3, tDesc - 0.3);
  caption(root, "单击描述，直接改 Markdown 源码", tDesc, tBig - 0.9);
  // 打开放大编辑前拉远到整窗
  const full = 0.82;
  clip.motion.to(tBig - 0.8, 0.8, { x: (W - 2160 * full) / 2, y: (H - 1200 * full) / 2 - 20, s: full }, "inout");
  caption(root, "放大编辑：左边源码，右边实时预览", tBig + 0.2, end - 0.1);
  // 换成内容更丰富的只读详情，镜头回到原位，文案带要点重新出现
  const rich = windowShot(root, "editor-view.png", { ...home, kind: "opaque" });
  rich.motion.init = { x: (W - 2160 * full) / 2, y: (H - 1200 * full) / 2 - 20, s: full, o: 0 };
  rich.motion.to(end, 0.5, { o: 1 }, "inout").to(end + 0.5, 0.9, { x: home.x, y: home.y, s: home.scale }, "inout");
  clip.motion.to(end + 0.5, 0.01, { o: 0 });
  copyBlock(root, end + 1.0, {
    num: "04",
    kicker: "详情与 Markdown",
    title: "写得下细节，<br>也读得舒服",
    bullets: [
      "点击待办先看<b>只读详情</b>，一键切到编辑",
      "<b>Markdown</b>：表格、任务清单、代码、链接",
      "粘贴或拖入<b>图片</b>，保存在本机",
      "放大编辑：<b>源码与预览</b>分栏",
    ],
    width: 620,
    gap: 0.4,
  });
  return { minDur: end + 5.6 - s };
});

// 05 子任务
scene(9.5, "bg-studio", (root, s) => {
  const x = 900;
  const y = 90;
  const scale = 0.6;
  const w = windowShot(root, "subtask-view.png", { x, y, scale, kind: "opaque" });
  enter(w.motion, s, x, y, scale);
  copyBlock(root, s + 0.3, {
    num: "05",
    kicker: "子任务",
    title: "大事拆小，<br>一步步完成",
    bullets: [
      "每个子任务都有自己的 <b>Markdown</b> 内容与图片",
      "可从 .md / .txt 文件或整个<b>文件夹</b>批量导入",
      "拖动排序、双击改名，进度自动统计",
    ],
    width: 680,
  });
});

// 06 提醒（编辑窗口的提醒下拉 + 右下角的应用内通知）
scene(11.5, "bg-studio", (root, s) => {
  const x = 720;
  const y = 110;
  const scale = 0.7;
  const ed = windowShot(root, "editor-notify-select.png", { x, y, scale, kind: "opaque" });
  enter(ed.motion, s, x, y, scale);
  const nscale = 0.8;
  const nx = W - 640 * nscale - 60;
  const ny = H - 240 * nscale - 60;
  const n = windowShot(root, "notification.png", { x: nx, y: ny, scale: nscale });
  n.motion.init = { x: W + 40, y: ny, s: nscale, o: 1 };
  n.motion.to(s + 4.4, 0.9, { x: nx }, "out");
  tag(root, "应用内通知：右下角弹出，不抢焦点", nx, ny - 74, s + 5.1, null, "blue");
  copyBlock(root, s + 0.3, {
    num: "06",
    kicker: "提醒",
    title: "到点提醒，<br>不怕忘",
    bullets: [
      "<b>系统通知</b>，或右下角的应用内通知",
      "准时或提前 5 / 15 / 30 分钟、1 小时，也可自定义",
      "按天 / 周 / 月<b>重复</b>，自动推进到下一次",
      "启动时补发错过的重复提醒",
    ],
    width: 540,
    gap: 0.5,
  });
});

// 07 外观与窗口模式（桌面壁纸上：浅色 → 深色半透明 → 固定模式）
scene(13, "bg-desk", (root, s) => {
  const win = { x: 1090, y: 60, scale: 0.8 };
  const light = windowShot(root, "main-list.png", { ...win, desk: true });
  enter(light.motion, s, win.x, win.y, win.scale);
  light.motion.to(s + 3.4, 0.8, { o: 0 }, "inout");
  const dark = windowShot(root, "main-dark.png", { ...win, desk: true });
  dark.motion.init = { x: win.x, y: win.y, s: win.scale, o: 0 };
  dark.motion.to(s + 3.4, 0.8, { o: 1 }, "inout").to(s + 7.8, 0.8, { o: 0 }, "inout");
  const fixed = windowShot(root, "main-fixed-dark.png", { ...win, desk: true });
  fixed.motion.init = { x: win.x, y: win.y, s: win.scale, o: 0 };
  fixed.motion.to(s + 7.8, 0.8, { o: 1 }, "inout");
  ring(root, ...at(win, 222, 8), 38 * 2 * win.scale, 38 * 2 * win.scale, s + 2.6, s + 4.4);
  ring(root, ...at(win, 258, 8), 38 * 2 * win.scale, 38 * 2 * win.scale, s + 7.2, s + 10.6);
  const [tx, ty] = at(win, 20, 520);
  tag(root, "深色主题：底色与透明度可调，透出桌面", tx, ty, s + 4.2, s + 7.6);
  tag(root, "固定模式：标题栏锁形按钮一键切换", tx, ty, s + 8.6, null);
  copyBlock(root, s + 0.3, {
    num: "07",
    kicker: "外观与窗口",
    title: "融入你的桌面",
    bullets: [
      "浅色 / <b>深色</b>主题，深色可调底色与透明度",
      "<b>固定模式</b>：钉在原处，不可拖动、不占任务栏",
      "可选贴边自动隐藏、唤起时置顶",
      "Windows 11 24H2+ 可<b>嵌入桌面</b>，Win+D 也不消失",
      "多显示器分别记住窗口位置",
    ],
    dark: true,
    width: 760,
    gap: 0.45,
  });
});

// 08 桌面收纳箱（关键帧片段：Ctrl+滚轮、右键菜单；之后演示吸附对齐）
scene(19, "bg-desk", (root, s) => {
  const scale = 0.76;
  const fy = 110;
  const x1 = 1850 - 736 * scale; // 收纳箱（右，有项目）
  const x2 = x1 - 736 * scale - 30; // 收纳箱 2（左，空）
  const clip = new ClipPlayer(root, "clip-fence", { x: x1, y: fy, scale, desk: true, cursorPx: 30 });
  clip.motion.init = { x: x1, y: fy + 40, s: scale, o: 0 };
  clip.motion.to(s + 0.2, 0.8, { y: fy, o: 1 });
  const clipEnd = clip.playSegments(s + 1.4, [[0, clip.length, 1]]);
  clip.cursorLinger = 0.3;
  const tWheel = clip.sceneTime(clip.meta.shots[2].t);
  const tMenu = clip.sceneTime(clip.eventTime("rdown", 0));
  tag(root, "Ctrl + 滚轮：调图标大小", x1, fy + 672 * scale + 30, tWheel - 0.2, tMenu - 0.6, "blue");
  tag(root, "右键：打开、移回桌面、移动到其它收纳箱", x1 - 60, fy + 672 * scale + 30, tMenu, clipEnd - 0.4, "blue");

  // 吸附对齐：收纳箱 2 从下方拖上来，接近时上边对齐，目标收纳箱的上边亮一条高亮线
  const f2 = windowShot(root, "fence-pair-1.png", { x: x2, y: fy, scale, desk: true });
  const startY = fy + 330;
  f2.motion.init = { x: x2 - 30, y: startY, s: scale, o: 0 };
  f2.motion.to(s + 0.6, 0.8, { x: x2, o: 1 });
  const tDrag = clipEnd + 0.2;
  f2.motion.to(tDrag, 1.5, { y: fy + 22 }, "inout").to(tDrag + 1.5, 0.12, { y: fy }, "out");
  const line = el("div", "snapline", root);
  css(line, { width: `${736 * scale}px`, height: "4px" });
  new Motion(line, { x: x1, y: fy - 2, o: 0 }).to(tDrag + 1.5, 0.12, { o: 1 }).to(tDrag + 3.0, 0.3, { o: 0 }, "in");
  const cur = el("div", "cursor", root, CURSOR_SVG);
  const grab = [x2 + 140, startY + 22];
  new Motion(cur, { x: grab[0] + 260, y: grab[1] + 260, o: 0, s: 0.62 })
    .to(tDrag - 0.9, 0.7, { x: grab[0], y: grab[1], o: 1 }, "inout")
    .to(tDrag, 1.5, { y: fy + 22 + 22 }, "inout")
    .to(tDrag + 1.5, 0.12, { y: fy + 22 }, "out")
    .to(tDrag + 2.7, 0.6, { x: grab[0] + 180, y: fy + 520, o: 0 }, "inout");
  tag(root, "拖动时自动吸附对齐，按住 Alt 临时关闭", x2, fy + 672 * scale + 30, tDrag + 1.6, null, "blue");

  copyBlock(root, s + 0.3, {
    num: "08",
    kicker: "桌面收纳箱",
    title: "桌面再乱，<br>也能收拾干净",
    bullets: [
      "桌面文件拖进来，<b>真实移动</b>到收纳文件夹",
      "重名自动编号<b>绝不覆盖</b>，删除时原样移回",
      "双击打开、右键菜单、折叠与锁定",
      "Windows 嵌在桌面层；macOS 版为<b>预览</b>",
    ],
    dark: true,
    width: 600,
    left: 80,
    gap: 0.6,
  });
  return { minDur: tDrag + 4.2 - s };
});

// 09 同步与备份
scene(11, "bg-studio", (root, s) => {
  const x = 930;
  const y = (H - 1120 * 0.66) / 2;
  const scale = 0.66;
  const w = windowShot(root, "settings-data.png", { x, y, scale, kind: "opaque" });
  enter(w.motion, s, x, y, scale);
  copyBlock(root, s + 0.3, {
    num: "09",
    kicker: "同步与备份",
    title: "多台电脑，<br>一份数据",
    bullets: [
      "<b>WebDAV</b> 同步：坚果云、Nextcloud、自建都能用",
      "逐条合并：同时改不同的待办<b>也不丢</b>，删除同样同步",
      "定时自动同步，在后台进行",
      "一键导出 / 导入 <b>ZIP 备份</b>",
    ],
    width: 720,
    gap: 0.5,
  });
});

// 10 云端 API 与 AI
scene(11, "bg-ink", (root, s) => {
  const term = el("div", "term", root);
  term.innerHTML = `<div class="bar"><i class="dot" style="background:#F87171"></i><i class="dot" style="background:#FBBF24"></i><i class="dot" style="background:#34D399"></i><span>Claude Code Skill · minitodo.py</span></div>
<div class="body"><span class="c"># 今天有什么待办？</span>
<span class="p">$</span> python minitodo.py <span class="k">today</span>
<span class="c"># 帮我加一个"买菜"，后天截止</span>
<span class="p">$</span> python minitodo.py <span class="k">add</span> <span class="s">"买菜"</span> --due 2026-10-20
<span class="c"># 把 C3 标记完成</span>
<span class="p">$</span> python minitodo.py <span class="k">done</span> C3</div>`;
  new Motion(term, { x: 900, y: 150, o: 0 }).to(s + 0.4, 0.8, { x: 860, o: 1 });
  const api = el("div", "api", root);
  const rows = [
    ["GET", "#2563EB", "/todos", "列表与筛选"],
    ["POST", "#059669", "/todos", "新建"],
    ["PATCH", "#D97706", "/todos/:id", "修改"],
    ["DELETE", "#DC2626", "/todos/:id", "删除（同步墓碑）"],
  ];
  for (const [m, c, u, d] of rows) {
    el("div", "m", api, m).style.background = c;
    el("div", "u", api, `${u}<em>${d}</em>`);
  }
  new Motion(api, { x: 900, y: 640, o: 0 }).to(s + 1.6, 0.8, { x: 860, o: 1 });
  copyBlock(root, s + 0.3, {
    num: "10",
    kicker: "云端 API · 可选",
    title: "让 AI<br>帮你管待办",
    bullets: [
      "在 VPS 上部署 <b>cloud/</b>，经同一个 WebDAV 与电脑同步",
      "REST API + <b>Claude Code Skill</b>：用自然语言查看、添加、完成待办",
      "配合 openclaw 定时推送<b>临期提醒</b>",
    ],
    dark: true,
    width: 640,
    gap: 0.5,
  });
});

// 11 片尾
scene(6.5, "bg-hero", (root, s) => {
  const hero = el("div", "hero", root);
  const icon = el("img", "icon", hero);
  icon.src = asset("icon.png");
  icon.style.transformOrigin = "50% 50%";
  const h1 = el("h1", "", hero, "Mini Todo");
  const url = el("div", "url", hero, "github.com/dreamlonglll/mini-todo");
  const small = el("div", "small", hero, "Windows 10 / 11 x64 · macOS 11+（预览）· 免费开源（MIT）");
  new Motion(icon, { s: 0.7, o: 0 }).to(s + 0.2, 0.8, { s: 1, o: 1 }, "back");
  new Motion(h1, { o: 0, y: 24 }).to(s + 0.45, 0.7, { o: 1, y: 0 });
  new Motion(url, { o: 0, y: 24 }).to(s + 0.8, 0.7, { o: 1, y: 0 });
  new Motion(small, { o: 0, y: 24 }).to(s + 1.1, 0.7, { o: 1, y: 0 });
});

// ---------------------------------------------------------------------------
// README 截图（render.cjs --still 名字）：同一批素材的静态合成
// ---------------------------------------------------------------------------

const STILLS = {
  // 三种视图并排
  views: { size: [2410, 940], bg: "bg-studio", build(root) {
    const s = 0.62;
    windowShot(root, "main-list.png", { x: 70, y: 98, scale: s }).motion.apply(0);
    windowShot(root, "main-quadrant.png", { x: 70 + 760 * s + 50, y: 98, scale: s }).motion.apply(0);
    windowShot(root, "main-calendar.png", { x: 70 + 2 * (760 * s + 50), y: 98, scale: s }).motion.apply(0);
  } },
  // 只读详情 + 子任务
  detail: { size: [1960, 1100], bg: "bg-studio", build(root) {
    windowShot(root, "editor-view.png", { x: 80, y: 70, scale: 0.83, kind: "opaque" }).motion.apply(0);
  } },
  // 桌面：深色半透明主窗口、两个收纳箱（一个折叠）、应用内通知
  desktop: { size: [2400, 1350], bg: "bg-desk", build(root) {
    const fs = 0.9;
    windowShot(root, "main-dark.png", { x: 120, y: 110, scale: 0.92, desk: true }).motion.apply(0);
    windowShot(root, "fence.png", { x: 960, y: 110, scale: fs, desk: true }).motion.apply(0);
    windowShot(root, "fence-pair-1.png", { x: 960 + 736 * fs + 40, y: 110, scale: fs, desk: true }).motion.apply(0);
    windowShot(root, "fence-collapsed.png", { x: 960, y: 110 + 672 * fs + 40, scale: fs, desk: true }).motion.apply(0);
    windowShot(root, "notification.png", { x: 2400 - 640 * 0.9 - 100, y: 1350 - 240 * 0.9 - 100, scale: 0.9, desk: true }).motion.apply(0);
  } },
  // 设置：常规 + 数据与同步
  settings: { size: [2040, 900], bg: "bg-studio", build(root) {
    const s = 0.66;
    windowShot(root, "settings-general.png", { x: 70, y: 80, scale: s, kind: "opaque" }).motion.apply(0);
    windowShot(root, "settings-data.png", { x: 70 + 1360 * s + 60, y: 80, scale: s, kind: "opaque" }).motion.apply(0);
  } },
};

// ---------------------------------------------------------------------------
// 组装与 seek
// ---------------------------------------------------------------------------

const stage = document.getElementById("stage");
const progress = document.getElementById("progress");
let DURATION = 0;

const IMAGES = ["icon.png", "main-list.png", "main-quadrant.png", "main-dark.png", "main-fixed-dark.png",
  "main-calendar.png", "editor-view.png", "editor-notify-select.png", "subtask-view.png", "notification.png", "fence.png",
  "fence-pair-1.png", "fence-collapsed.png", "settings-general.png", "settings-data.png"];

async function loadSizes() {
  await Promise.all(
    IMAGES.map(async (n) => {
      const img = await loadImage(asset(n));
      sizes[n] = [img.naturalWidth, img.naturalHeight];
    }),
  );
}

async function initStill(name) {
  const still = STILLS[name];
  if (!still) throw new Error(`没有这张截图：${name}（可选：${Object.keys(STILLS).join(" / ")}）`);
  const [w, h] = still.size;
  for (const node of [document.documentElement, document.body, stage]) css(node, { width: `${w}px`, height: `${h}px` });
  progress.style.display = "none";
  const root = el("div", "scene", stage);
  root.style.display = "block";
  el("div", `bg ${still.bg}`, root);
  still.build(root);
  window.STILL_SIZE = { width: w, height: h };
  await Promise.all([...document.images].map((i) => i.decode().catch(() => {})));
}

/** 静态细颗粒：大面积渐变在 8 位 H.264 里会出色带，叠一层极淡的固定噪点让编码器保住过渡（伪随机，结果确定） */
function grainUrl() {
  const c = document.createElement("canvas");
  c.width = c.height = 256;
  const ctx = c.getContext("2d");
  const img = ctx.createImageData(256, 256);
  let seed = 20261018;
  for (let i = 0; i < img.data.length; i += 4) {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    const v = seed >>> 24;
    img.data[i] = img.data[i + 1] = img.data[i + 2] = v;
    img.data[i + 3] = 255;
  }
  ctx.putImageData(img, 0, 0);
  return c.toDataURL("image/png");
}

let GRAIN = "";

function background(root, cls) {
  el("div", `bg ${cls}`, root);
  css(el("div", "grain", root), { backgroundImage: `url(${GRAIN})` });
}

async function init() {
  await loadSizes();
  GRAIN = grainUrl();
  if (params.get("still")) return initStill(params.get("still"));
  let t = 0;
  scenes.forEach((sc, i) => {
    const root = el("div", "scene", stage);
    background(root, sc.bg);
    sc.root = root;
    sc.start = t;
    const extra = sc.build(root, t) || {};
    if (extra.minDur && extra.minDur > sc.dur) sc.dur = extra.minDur;
    sc.fade = new Motion(root, { o: i === 0 ? 1 : 0 }).to(t, i === 0 ? 0 : XFADE, { o: 1 }, "inout");
    t += sc.dur - XFADE;
  });
  // 结尾淡出到黑
  const last = scenes[scenes.length - 1];
  last.fade.to(last.start + last.dur - 0.8, 0.8, { o: 0 }, "in");
  DURATION = last.start + last.dur;
  window.DURATION = DURATION;
  window.SCENES = scenes.map((sc) => ({ start: sc.start, dur: sc.dur }));
  await window.seek(Number(params.get("t") || 0));
}

window.seek = async function seek(t) {
  for (const sc of scenes) {
    const on = t >= sc.start - 1e-6 && t <= sc.start + sc.dur + 1e-6;
    sc.root.style.display = on ? "block" : "none";
  }
  for (const m of motions) m.apply(t);
  const waits = [];
  for (const p of players) {
    const sc = scenes.find((x) => x.root.contains(p.box));
    if (sc && sc.root.style.display === "block") {
      const w = p.seek(t);
      if (w) waits.push(w);
    }
  }
  progress.style.transform = `scaleX(${Math.min(1, t / DURATION)})`;
  await Promise.all(waits);
  await Promise.all([...document.images].filter((i) => !i.complete).map((i) => i.decode().catch(() => {})));
};

window.READY = init()
  .then(() => true)
  .catch((e) => {
    document.body.innerHTML = `<pre style="color:#fff;font-size:28px;padding:40px;white-space:pre-wrap">${e.stack || e}</pre>`;
    window.READY_ERROR = String(e.stack || e);
    return false;
  });
