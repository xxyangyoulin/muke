// 轻量 CDP 助手（无第三方依赖，使用 Node 原生 WebSocket）
const { spawn } = require('child_process');

async function openPage(url, width = 1440, height = 900) {
 const port = 9460 + Math.floor(Math.random() * 300);
 const chrome = spawn(process.env.CHROME_BIN || '/opt/google/chrome/chrome', [
  '--headless=new', '--no-sandbox', '--disable-gpu', '--no-proxy-server',
  `--window-size=${width},${height}`, `--remote-debugging-port=${port}`,
  '--user-data-dir=/tmp/mkblog-smoke-' + Date.now(), 'about:blank',
 ], { stdio: ['ignore', 'ignore', 'pipe'] });
 let startupError = null, diagnostics = '', target;
 chrome.on('error', error => { startupError = error; });
 chrome.stderr.on('data', chunk => { diagnostics = (diagnostics + chunk.toString()).slice(-4000); });
 const deadline = Date.now() + 15000;
 try {
  while (Date.now() < deadline) {
   if (startupError) throw startupError;
   if (chrome.exitCode !== null) throw new Error('Chrome 启动失败：' + diagnostics);
   try {
    const response = await fetch(`http://127.0.0.1:${port}/json/list`, {signal: AbortSignal.timeout(1000)});
    if (response.ok) target = (await response.json()).find(t => t.type === 'page' && t.webSocketDebuggerUrl);
   } catch {}
   if (target) break;
   await new Promise(r => setTimeout(r, 250));
  }
  if (!target) throw new Error('等待 Chrome 调试接口超时：' + diagnostics);
 } catch (error) { chrome.kill(); throw error; }
 const ws = new WebSocket(target.webSocketDebuggerUrl);
 await new Promise(r => ws.addEventListener('open', r));
 let seq = 0;
 const pending = new Map();
 const errors = [];
 ws.addEventListener('message', d => {
  const m = JSON.parse(d.data ?? d);
  if (m.id && pending.has(m.id)) {
   const p = pending.get(m.id);
   pending.delete(m.id);
   m.error ? p.rej(new Error(m.error.message)) : p.res(m.result);
  }
  if (m.method === 'Runtime.exceptionThrown') errors.push((m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text || '').slice(0, 250));
 });
 const send = (method, params = {}) => new Promise((res, rej) => {
  const id = ++seq;
  pending.set(id, { res, rej });
  ws.send(JSON.stringify({ id, method, params }));
 });
 const evalJs = async expression => {
  const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error((r.exceptionDetails.exception?.description || r.exceptionDetails.text || '').slice(0, 250));
  return r.result.value;
 };
 await send('Page.enable');
 await send('Runtime.enable');
 const goto = async u => { await send('Page.navigate', { url: u }); await new Promise(r => setTimeout(r, 1800)); };
 const close = () => { try { ws.close(); } catch {} chrome.kill(); };
 return { chrome, ws, send, evalJs, goto, errors, close, target: url };
}

const results = { pass: 0, fail: 0 };
function ok(name, cond, detail = '') {
 console.log((cond ? '✓ ' : '✗ ') + name + (detail ? '：' + detail : ''));
  cond ? results.pass++ : results.fail++;
}

module.exports = { openPage, ok, results };
