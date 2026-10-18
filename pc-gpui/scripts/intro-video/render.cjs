#!/usr/bin/env node
// 介绍视频的逐帧渲染：Playwright（Chromium）打开 video.html，逐帧 window.seek(t) 截图，经管道交给 ffmpeg 编码 H.264；
// 按时间切成几段并行渲染，最后无损拼接。
// 用法见同目录 README.md。依赖：Node 18+、playwright（全局或本地安装均可，浏览器用 Playwright 自带的 Chromium）、ffmpeg。
"use strict";

const fs = require("fs");
const path = require("path");
const { spawn, execSync } = require("child_process");

function loadPlaywright() {
  try {
    return require("playwright");
  } catch {
    const root = execSync("npm root -g").toString().trim();
    return require(path.join(root, "playwright"));
  }
}

const HERE = __dirname;
const WORKSPACE = path.resolve(HERE, "..", "..");

function parseArgs(argv) {
  const args = {
    assets: path.join(WORKSPACE, "target", "intro-video", "assets"),
    out: path.join(WORKSPACE, "target", "intro-video", "mini-todo-intro.mp4"),
    fps: 30,
    crf: 26, // 2 分 17 秒约 9 MB：GitHub 免费账户的视频附件上限是 10 MB（crf 23 约 13 MB，上传会失败）
    jobs: 3, // 并行渲染的浏览器数（按时间切段，各自编码后无损拼接）
    from: 0,
    to: null,
    frames: null, // 逗号分隔的时间点：只截这些帧（检查用）
    framesOut: path.join(WORKSPACE, "target", "intro-video", "frames"),
    stills: [], // README 截图：名字（video.js 的 STILLS）
    stillsOut: path.join(WORKSPACE, "target", "intro-video", "stills"),
  };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    const next = () => argv[++i];
    if (a === "--assets") args.assets = path.resolve(next());
    else if (a === "--out") args.out = path.resolve(next());
    else if (a === "--fps") args.fps = Number(next());
    else if (a === "--crf") args.crf = Number(next());
    else if (a === "--jobs") args.jobs = Math.max(1, Number(next()));
    else if (a === "--from") args.from = Number(next());
    else if (a === "--to") args.to = Number(next());
    else if (a === "--frames") args.frames = next().split(",").map(Number);
    else if (a === "--frames-out") args.framesOut = path.resolve(next());
    else if (a === "--still") args.stills.push(next());
    else if (a === "--stills-out") args.stillsOut = path.resolve(next());
    else if (a === "-h" || a === "--help") {
      console.log(fs.readFileSync(path.join(HERE, "README.md"), "utf8"));
      process.exit(0);
    } else {
      console.error(`未知参数：${a}`);
      process.exit(2);
    }
  }
  return args;
}

function readClips(assets) {
  const dir = path.join(assets, "clips");
  const clips = {};
  if (!fs.existsSync(dir)) return clips;
  for (const f of fs.readdirSync(dir)) {
    if (f.endsWith(".json")) clips[f.slice(0, -5)] = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
  }
  return clips;
}

async function openPage(browser, args, viewport = { width: 1920, height: 1080 }) {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  page.on("pageerror", (e) => console.error("[页面错误]", e.message));
  const clips = readClips(args.assets);
  await page.addInitScript(
    ({ assets, clips }) => {
      window.INTRO_ASSETS = assets;
      window.INTRO_CLIPS = clips;
    },
    { assets: "file://" + args.assets, clips },
  );
  return page;
}

function run(cmd, argv, opts = {}) {
  return new Promise((resolve, reject) => {
    const p = spawn(cmd, argv, { stdio: ["ignore", "inherit", "inherit"], ...opts });
    p.on("close", (code) => (code === 0 ? resolve() : reject(new Error(`${cmd} 退出码 ${code}`))));
  });
}

const LAUNCH = { args: ["--allow-file-access-from-files", "--font-render-hinting=none"] };

/** 渲染帧 [a, b) 到 file：每段一个独立的浏览器进程；JPEG（质量 95）经管道交给 ffmpeg，编码参数各段一致以便无损拼接 */
async function renderRange(chromium, args, a, b, file, onProgress) {
  const browser = await chromium.launch(LAUNCH);
  const page = await openPage(browser, args);
  await page.goto("file://" + path.join(HERE, "video.html"));
  if (!(await page.evaluate(() => window.READY))) throw new Error(await page.evaluate(() => window.READY_ERROR));
  const ff = spawn(
    "ffmpeg",
    ["-hide_banner", "-loglevel", "error", "-y", "-f", "image2pipe", "-framerate", String(args.fps), "-c:v", "mjpeg", "-i", "-",
     "-c:v", "libx264", "-preset", "slow", "-crf", String(args.crf), "-tune", "animation", "-pix_fmt", "yuv420p",
     // aq-mode=3 偏向暗部与平坦区域，配合页面上的细颗粒压住渐变色带
     "-x264-params", "aq-mode=3", "-g", String(args.fps * 4), "-movflags", "+faststart", file],
    { stdio: ["pipe", "inherit", "inherit"] },
  );
  const done = new Promise((resolve, reject) => ff.on("close", (code) => (code === 0 ? resolve() : reject(new Error(`ffmpeg 退出码 ${code}`)))));
  for (let i = a; i < b; i++) {
    await page.evaluate((t) => window.seek(t), i / args.fps);
    const jpg = await page.screenshot({ type: "jpeg", quality: 95 });
    if (!ff.stdin.write(jpg)) await new Promise((r) => ff.stdin.once("drain", r));
    onProgress(i - a + 1);
  }
  ff.stdin.end();
  await done;
  await browser.close();
}

async function main() {
  const args = parseArgs(process.argv);
  if (!fs.existsSync(path.join(args.assets, "main-list.png"))) {
    console.error(`素材目录里没有截图：${args.assets}（先跑 capture.py）`);
    process.exit(1);
  }
  const { chromium } = loadPlaywright();
  const browser = await chromium.launch(LAUNCH);
  try {
    if (args.stills.length) {
      fs.mkdirSync(args.stillsOut, { recursive: true });
      for (const name of args.stills) {
        const page = await openPage(browser, args, { width: 2400, height: 1400 });
        await page.goto("file://" + path.join(HERE, "video.html") + `?still=${encodeURIComponent(name)}`);
        const ok = await page.evaluate(() => window.READY);
        if (!ok) throw new Error(await page.evaluate(() => window.READY_ERROR));
        const size = await page.evaluate(() => window.STILL_SIZE);
        await page.setViewportSize(size);
        const file = path.join(args.stillsOut, `${name}.png`);
        await page.screenshot({ path: file, clip: { x: 0, y: 0, ...size } });
        console.log(`  ${file} ${size.width}x${size.height}`);
        await page.close();
      }
      return;
    }

    const page = await openPage(browser, args);
    await page.goto("file://" + path.join(HERE, "video.html"));
    const ok = await page.evaluate(() => window.READY);
    if (!ok) throw new Error(await page.evaluate(() => window.READY_ERROR));
    const duration = await page.evaluate(() => window.DURATION);
    const scenes = await page.evaluate(() => window.SCENES);
    console.log(`时长 ${duration.toFixed(2)} 秒，${scenes.length} 个场景：` + scenes.map((s) => s.start.toFixed(1)).join(" / "));

    if (args.frames) {
      fs.mkdirSync(args.framesOut, { recursive: true });
      for (const t of args.frames) {
        await page.evaluate((t) => window.seek(t), t);
        const file = path.join(args.framesOut, `t${t.toFixed(2).padStart(7, "0")}.png`);
        await page.screenshot({ path: file });
        console.log(`  ${file}`);
      }
      return;
    }

    await page.close();
    const to = args.to == null ? duration : Math.min(args.to, duration);
    const first = Math.round(args.from * args.fps);
    const last = Math.round(to * args.fps); // 不含
    fs.mkdirSync(path.dirname(args.out), { recursive: true });
    const jobs = Math.min(args.jobs, last - first);
    const per = Math.ceil((last - first) / jobs);
    const parts = [];
    const progress = new Array(jobs).fill(0);
    const started = Date.now();
    const tick = setInterval(() => {
      const done = progress.reduce((a, b) => a + b, 0);
      process.stdout.write(`\r  ${done}/${last - first} 帧，已用 ${((Date.now() - started) / 1000).toFixed(0)}s   `);
    }, 5000);
    await Promise.all(
      Array.from({ length: jobs }, async (_, j) => {
        const a = first + j * per;
        const b = Math.min(last, a + per);
        const file = jobs === 1 ? args.out : `${args.out}.part${j}.mp4`;
        parts[j] = file;
        await renderRange(chromium, args, a, b, file, (n) => (progress[j] = n));
      }),
    );
    clearInterval(tick);
    if (jobs > 1) {
      // 各段编码参数相同，concat 分离器直接拼接（不重新编码）
      const list = `${args.out}.parts.txt`;
      fs.writeFileSync(list, parts.map((p) => `file '${p.replace(/'/g, "'\\''")}'`).join("\n") + "\n");
      await run("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", list, "-c", "copy",
        "-movflags", "+faststart", "-metadata", "title=Mini Todo 功能介绍", args.out]);
      for (const p of parts) fs.unlinkSync(p);
      fs.unlinkSync(list);
    }
    console.log(`\n输出：${args.out}（${((Date.now() - started) / 1000).toFixed(0)} 秒）`);
  } finally {
    await browser.close();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
