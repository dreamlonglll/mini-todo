// 通过 WebView2 远程调试端口（CDP）在 mini-todo 的某个 WebView 里执行 JS。
// 用法：node cdp.mjs [--target <url 子串>] [--list] "<js expression>"
// 需要启动 mini-todo 时设置 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222
const PORT = process.env.CDP_PORT || '9222';
const args = process.argv.slice(2);
let targetHint = 'localhost:1420/';
let list = false;
const rest = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--target') { targetHint = args[++i]; continue; }
  if (args[i] === '--list') { list = true; continue; }
  rest.push(args[i]);
}
const expr = rest.join(' ');

const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const pages = targets.filter(t => t.type === 'page');
if (list) {
  for (const t of pages) console.log(`${t.id}\t${t.title}\t${t.url}`);
  process.exit(0);
}
// 主窗口 URL 形如 http://localhost:1420/ 或 http://localhost:1420/#/ ；编辑器为 #/editor?...
let page = pages.find(t => t.url.includes(targetHint) && (targetHint !== 'localhost:1420/' || !/#\/(editor|settings|completed|subtask-editor|notification)/.test(t.url)));
if (!page) { console.error('no page target matching', targetHint, '\navailable:', pages.map(p => p.url)); process.exit(2); }

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let id = 0;
function send(method, params) {
  return new Promise((res, rej) => {
    const myId = ++id;
    const onmsg = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id !== myId) return;
      ws.removeEventListener('message', onmsg);
      msg.error ? rej(new Error(JSON.stringify(msg.error))) : res(msg.result);
    };
    ws.addEventListener('message', onmsg);
    ws.send(JSON.stringify({ id: myId, method, params }));
  });
}
const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true });
if (r.exceptionDetails) { console.error('EXCEPTION:', r.exceptionDetails.text, r.exceptionDetails.exception?.description ?? ''); ws.close(); process.exit(3); }
console.log(typeof r.result.value === 'string' ? r.result.value : JSON.stringify(r.result.value));
ws.close();
