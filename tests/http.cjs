const assert = require('node:assert/strict');
const http = require('node:http');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const run = promisify(execFile);

(async () => {
 const server = http.createServer((req, res) => {
  if (req.url === '/relative') { res.writeHead(302, {Location: '/ok'}); res.end(); }
  else if (req.url === '/loop') { res.writeHead(302, {Location: '/loop'}); res.end(); }
  else if (req.url === '/scheme') { res.writeHead(302, {Location: 'file:///etc/passwd'}); res.end(); }
  else if (req.url === '/origin') { res.writeHead(302, {Location: `http://localhost:${server.address().port}/headers`}); res.end(); }
  else if (req.url === '/headers') res.end(req.headers.authorization || 'none');
  else res.end('ok');
 });
 await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
 const base = `http://127.0.0.1:${server.address().port}`;
 const fetch = async (url, allowPrivate = false) => {
  const result = await run('php', ['-r', `require $argv[1]; function opt($key) { return ''; } try { echo http_fetch($argv[2], 2, ['Authorization'=>'secret'], $argv[3] === '1'); } catch (Throwable $e) { fwrite(STDERR, $e->getMessage()); exit(1); }`, __dirname + '/../src/Http.php', url, allowPrivate ? '1' : '0']);
  return result.stdout;
 };
 try {
  for (const url of ['file:///etc/passwd', 'gopher://example.com', base, 'http://[::1]/', 'http://[::ffff:127.0.0.1]/', 'http://user:pass@example.com/']) await assert.rejects(fetch(url));
  assert.equal(await fetch(base + '/relative', true), 'ok');
  assert.equal(await fetch(base + '/headers', true), 'secret');
  assert.equal(await fetch(base + '/origin', true), 'none');
  await assert.rejects(fetch(base + '/scheme', true));
  await assert.rejects(fetch(base + '/loop', true));
  console.log('http: 协议、凭据、内网地址、相对跳转、跨源凭据隔离和跳转上限通过');
 } finally { server.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
