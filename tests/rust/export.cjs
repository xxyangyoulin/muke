const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const {openPage} = require('../cdp.cjs');

(async () => {
 const root = path.resolve(process.argv[2]);
 const requests = [];
 const server = http.createServer(async (req, res) => {
  const pathname = new URL(req.url, 'http://localhost').pathname;
  requests.push(pathname);
  const file = path.resolve(root, '.' + pathname, pathname.endsWith('/') ? 'index.html' : '');
  if (!file.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try {
   const content = await fs.readFile(file);
   const mime = {'.html':'text/html', '.js':'text/javascript', '.css':'text/css', '.svg':'image/svg+xml'};
   res.writeHead(200, {'Content-Type':(mime[path.extname(file)] || 'application/octet-stream') + '; charset=utf-8'}).end(content);
  } catch { res.writeHead(404).end(); }
 });
 await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
 let page;
 try {
  const base = `http://127.0.0.1:${server.address().port}`;
  page = await openPage(base);
  await page.goto(base);
  async function until(expression) {
   for (let i=0;i<150;i++) {
    if (await page.evalJs(expression)) return;
    await new Promise(resolve => setTimeout(resolve, 50));
   }
   throw new Error('页面等待超时: ' + expression + '\n' + await page.evalJs(`document.querySelector('#main')?.textContent`) + '\n' + JSON.stringify(page.errors));
  }
  await until(`document.querySelector('#post-list')?.textContent.includes('静态动态正文')`);
  await page.evalJs(`document.querySelector('nav [data-nav="articles"]').click()`);
  await until(`document.querySelector('#article-rows')?.textContent.includes('静态文章')`);
  await page.evalJs(`document.querySelector('#article-rows a[href="/article/7/"]').click()`);
  await until(`document.querySelector('.article-content')?.textContent.includes('静态文章正文')`);
  // Direct requests must work on a plain file server without route rewrites.
  for (const route of ['/article/7/', '/articles/']) {
   await page.goto(base + route);
   await until(`!document.querySelector('#baked-body') && document.querySelector('#main').textContent.includes('静态文章')`);
   assert.equal(await page.evalJs('document.title'), route === '/article/7/' ? '静态文章 · 静态测试站' : '文章随笔 · 静态测试站');
  }
  await page.evalJs(`document.querySelector('nav [data-nav="feed"]').click()`);
  await until(`document.querySelector('#post-list')?.textContent.includes('静态动态正文')`);
  assert.equal(await page.evalJs('document.title'), '静态测试站 · 记录');
  assert.ok(await page.evalJs(`document.querySelector('.post-movie-card').textContent.includes('电影 <img') && document.querySelector('.post-weibo-card').textContent.includes('微博正文')`));
  // RSS 是首页顶栏标签（page.tab），不走左侧导航（与 PHP 一致）
  await page.evalJs(`document.querySelector('.top-tabs a[href="/rss/"]').click()`);
  await until(`document.querySelector('#rss-list')?.textContent.includes('订阅标题')`);
  await page.evalJs(`document.querySelector('nav [data-nav="wallpapers"]').click()`);
  await until(`!!document.querySelector('.tile-preview') && !document.querySelector('#baked-body')`);
  for (const [route, selector] of [['/rss/', '#rss-list'], ['/wallpapers/', '.tile-preview']]) {
   await page.goto(base + route);
   await until(`!!document.querySelector(${JSON.stringify(selector)}) && !document.querySelector('#baked-body')`);
  }
  assert.ok(!requests.some(url => url.startsWith('/api')), '静态浏览不能请求后台 API');
  assert.deepEqual(page.errors, []);
  await page.send('Emulation.setScriptExecutionDisabled', {value:true});
  await page.goto(base + '/article/7/');
  await until(`document.querySelector('#baked-body')?.textContent.includes('静态文章正文')`);
  assert.equal(await page.evalJs(`getComputedStyle(document.querySelector('#baked-body')).display === 'none'`), false);
  for (const [route, selector] of [['/rss/', '.post-rss-card'], ['/wallpapers/', '.tile-preview'], ['/', '.post-movie-card']]) {
   await page.goto(base + route);
   assert.ok(await page.evalJs(`!!document.querySelector('#baked-body ${selector}') && !document.querySelector('#injected')`));
   assert.equal(await page.evalJs(`getComputedStyle(document.querySelector('#baked-body')).display === 'none'`), false);
  }
  console.log('PASS: 静态首页、文章、插件页面和卡片、直接访问及禁用 JavaScript 正文');
 } finally {
  page?.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
 }
})().catch(error => {console.error(error); process.exitCode=1;});
