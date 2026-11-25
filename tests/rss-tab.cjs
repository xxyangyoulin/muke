// RSS 订阅插件 · 浏览器冒烟：首页标签、独立订阅页、时间线并入开关、退订清史
const assert = require('node:assert/strict');
const http = require('node:http');
const {openPage} = require('./cdp.cjs');

(async () => {
 const [base] = process.argv.slice(2);
 // 本地 Atom 夹具（标题里带标记词，避免误配其他测试数据；正文图片指向公网占位避免 SSRF 拦截噪音）
 const fixture = http.createServer((req, res) => {
  if (req.url === '/atom.xml') {
   const entry = n => `<entry><title>RSS夹具第${n}条</title><link rel="alternate" type="text/html" href="http://example.test/${n}"/><id>tag:rss-tab.test,2026:${n}</id><published>2026-11-0${n}T08:00:00Z</published><updated>2026-11-0${n}T08:00:00Z</updated><content type="html"><![CDATA[<p>夹具正文 ${n}</p>]]></content></entry>`;
   res.setHeader('Content-Type', 'application/atom+xml');
   res.end(`<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>夹具订阅源</title>${entry(1)}${entry(2)}</feed>`);
  } else { res.writeHead(404); res.end(); }
 });
 await new Promise(resolve => fixture.listen(0, '127.0.0.1', resolve));
 const feedUrl = 'http://127.0.0.1:' + fixture.address().port + '/atom.xml';
 const response = await fetch(base + '/api?action=state');
 let cookie = response.headers.get('set-cookie').split(';')[0];
 const state = await response.json();
 const call = async (action, data) => {
  const res = await fetch(base + '/api?action=' + action, {method: data ? 'POST' : 'GET', headers: {Cookie: cookie, 'X-CSRF-Token': state.csrf, 'Content-Type': 'application/json'}, body: data ? JSON.stringify(data) : undefined});
  if (res.headers.get('set-cookie')) cookie = res.headers.get('set-cookie').split(';')[0];
  return {status: res.status, data: await res.json()};
 };
 let page;
 try {
  await call(state.setup ? 'setup' : 'login', {username: 'tester', password: 'Password12345'});
  assert.equal((await call('save-plugin', {id: 'rss', enabled: 1, values: {}})).status, 200);
  const added = await call('plugin-admin', {id: 'rss', op: 'add', url: feedUrl});
  assert.equal(added.status, 200);
  assert.equal(added.data.added, 2, '添加订阅未导入夹具条目');

  page = await openPage(base);
  // 访客视角：标签页可见、订阅动态默认不混入首页
  await page.goto(base + '/');
  assert.ok(await page.evalJs(`!!document.querySelector('.top-tabs a[href="/rss/"]')`), '首页缺少 RSS 订阅标签');
  assert.equal(await page.evalJs(`document.querySelectorAll('#post-list .post-rss-card').length`), 0, '默认关闭时订阅动态混入首页');
  assert.equal(await page.evalJs(`!!document.querySelector('.rss-admin-panel')`), false, '访客也能看到管理面板');

  await page.goto(base + '/rss/');
  assert.ok(await page.evalJs(`!!document.querySelector('#rss-list .post-rss-card')`), '订阅页未渲染订阅卡片');
  assert.ok(await page.evalJs(`!!document.querySelector('.feed-layout .right-column .rss-feeds-card')`), '访客右栏缺少订阅源卡片');
  assert.equal(await page.evalJs(`document.querySelectorAll('#rss-list > article').length`), 2, '订阅页条目数不符');
  assert.ok(await page.evalJs(`document.querySelector('.rss-chip.active')?.textContent.includes('全部')`), '缺少筛选 chip');

  // 管理员视角：注入会话 Cookie（HttpOnly，走 CDP），右栏管理面板与删除入口出现
  await page.send('Network.enable');
  await page.send('Network.setCookie', {name: cookie.split('=')[0], value: cookie.split('=').slice(1).join('='), url: base});
  await page.goto(base + '/rss/');
  assert.ok(await page.evalJs(`!!document.querySelector('.right-column .rss-admin-panel')`), '管理员订阅面板缺失');
  assert.equal(await page.evalJs(`document.querySelectorAll('#rss-list [data-rss-delete]').length`), 2, '订阅页缺少管理删除入口');

  // 并入首页动态列表：开启后时间线出现订阅卡片
  await call('save-plugin', {id: 'rss', values: {in_feed: '1'}});
  await page.goto(base + '/');
  assert.ok(await page.evalJs(`!!document.querySelector('#post-list .post-rss-card')`), '开启并入后首页仍无订阅动态');
  await call('save-plugin', {id: 'rss', values: {in_feed: '0'}});
  await page.goto(base + '/');
  assert.equal(await page.evalJs(`document.querySelectorAll('#post-list .post-rss-card').length`), 0, '关闭并入后首页仍混有订阅动态');

  // 退订（清史）：订阅页清空
  const feedId = (await page.evalJs(`(async () => (await (await fetch('/api?action=plugin-data&id=rss')).json()).data.feeds[0].id)()`));
  const removed = await call('plugin-admin', {id: 'rss', op: 'remove', feed_id: feedId, purge: 1});
  assert.equal(removed.status, 200);
  assert.equal(removed.data.deleted, 2, '清史退订未删除动态');
  await page.goto(base + '/rss/');
  await new Promise(resolve => setTimeout(resolve, 600));
  assert.equal(await page.evalJs(`document.querySelectorAll('#rss-list > article').length`), 0, '退订后订阅页仍有动态');
  await page.goto(base + '/articles/');
  assert.ok(await page.evalJs(`!!document.querySelector('.top-tabs a[href="/rss/"]')`), '文章页缺少 RSS 订阅标签');

  assert.deepEqual(page.errors, []);
  console.log('rss-tab: 首页标签、订阅页、并入开关与退订清史通过');
 } finally {
  page?.close();
  fixture.close();
  try { await call('save-plugin', {id: 'rss', enabled: 0, values: {}}); } catch {}
 }
})().catch(error => { console.error(error); process.exitCode = 1; });
