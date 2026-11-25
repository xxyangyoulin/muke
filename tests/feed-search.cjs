const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {openPage} = require('./cdp.cjs');

(async () => {
 const [base, root] = process.argv.slice(2);
 const sql = query => execFileSync('sqlite3', ['-cmd', '.timeout 5000', root + '/var/blog.sqlite', query], {encoding:'utf8'}).trim();
 const pause = () => new Promise(resolve => setTimeout(resolve, 550));
 sql(`WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<40)
 INSERT INTO posts(content,created_at,updated_at) SELECT 'search-fixture-'||x,'2021-01-01','2021-01-01' FROM n;
 WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<40)
 INSERT INTO articles(title,content,tags,draft,created_at,updated_at)
 SELECT 'search-fixture-'||x,'正文 search-fixture-'||x,'search-fixture-tag',0,'2018-01-01','2018-01-01' FROM n;
 INSERT INTO posts(content,created_at,updated_at) VALUES('search-fixture-safe [a.*] <img src=x onerror=alert(1)>','2021-01-01','2021-01-01');`);
 let page, server;
 try {
  execFileSync('php', [root + '/bin/export.php'], {cwd:root, stdio:'pipe'});
  server = http.createServer((req, res) => {
   const relative = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
   const file = path.join(root, 'dist', relative.endsWith('/') ? relative + 'index.html' : relative);
   if (!file.startsWith(root + '/dist/') || !fs.existsSync(file) || !fs.statSync(file).isFile()) { res.writeHead(404); res.end(); return; }
   res.setHeader('Content-Type', ({'.js':'text/javascript', '.css':'text/css', '.html':'text/html', '.json':'application/json'})[path.extname(file)] || 'application/octet-stream');
   res.end(fs.readFileSync(file));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const staticBase = 'http://127.0.0.1:' + server.address().port;
  page = await openPage(base);
  const search = async text => {
   await page.evalJs(`document.querySelector('#feed-search').value=${JSON.stringify(text)};document.querySelector('#feed-search').dispatchEvent(new Event('input'));`);
   await pause();
  };
  const select = async type => {
   await page.evalJs(`document.querySelector('[data-search-type="${type}"]').click()`);
   await pause();
  };
  const loadAll = async () => {
   for (let i=0; i<20 && await page.evalJs(`!!document.querySelector('#feed-more')`); i++) {
    await page.evalJs(`window.scrollTo(0,document.body.scrollHeight)`);
    await new Promise(resolve => setTimeout(resolve,150));
   }
  };
  for (const site of [base, staticBase]) {
   await page.goto(site + '/');
   await search('SEARCH-fixture-');
   await loadAll();
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list > article').length`), 81, '混排搜索分页遗漏结果');
   assert.ok(await page.evalJs(`[...document.querySelectorAll('#post-list > article')].every(el=>el.querySelector('mark.search-highlight'))`));
   await select('article');
   await loadAll();
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list > article').length`), 40);
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list > [id^="post-"]').length`), 0);
   assert.equal(await page.evalJs(`document.querySelector('[data-search-type="article"]').getAttribute('aria-pressed')`), 'true');
   await select('post');
   await loadAll();
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list > article').length`), 41);
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list .article-teaser').length`), 0);
   await search('[a.*]');
   assert.equal(await page.evalJs(`document.querySelector('.search-highlight').textContent`), '[a.*]');
   await search('<img src=x onerror=alert(1)>');
   assert.equal(await page.evalJs(`document.querySelector('.search-highlight').textContent`), '<img src=x onerror=alert(1)>');
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list .post-content img').length`), 0, '关键词被当作 HTML 插入');
   await search('no-matching-search-fixture');
   assert.ok(await page.evalJs(`document.querySelector('#post-list').textContent.includes('没有找到')`));
   await search('');
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list mark').length`), 0);
   await loadAll();
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list .article-teaser').length`), 0, '清空关键词丢失类型筛选');
   await select('all');
   await loadAll();
   assert.ok(await page.evalJs(`document.querySelectorAll('#post-list > article').length > 81`), '清空未恢复时间线');
   await page.evalJs(`document.querySelector('#feed-search').value='search-fixture-1';document.querySelector('#feed-search').dispatchEvent(new Event('input'));document.querySelector('[data-search-type="article"]').click();document.querySelector('#feed-search').value='search-fixture-40';document.querySelector('#feed-search').dispatchEvent(new Event('input'));`);
   await pause();
   assert.equal(await page.evalJs(`document.querySelectorAll('#post-list > article').length`), 1);
   assert.equal(await page.evalJs(`document.querySelector('#post-list .article-teaser h3').textContent`), 'search-fixture-40');
   await page.evalJs(`document.querySelector('a[href="/articles/"]').click()`);
   await pause();
   await page.evalJs(`document.querySelector('a[href="/"]').click()`);
   await pause();
   assert.equal(await page.evalJs(`document.querySelector('#feed-search').value`), '');
   assert.equal(await page.evalJs(`document.querySelector('[data-search-type="all"]').getAttribute('aria-pressed')`), 'true');
   await loadAll();
   assert.ok(await page.evalJs(`document.querySelectorAll('#post-list > article').length > 81`));
   if (site === staticBase) assert.equal(await page.evalJs(`performance.getEntriesByType('resource').filter(e=>e.name.includes('/api?')).length`), 0);
  }
  await page.send('Emulation.setDeviceMetricsOverride', {width:390, height:844, deviceScaleFactor:1, mobile:true});
  await page.goto(base + '/');
  assert.ok(await page.evalJs(`document.querySelector('#feed-search').getBoundingClientRect().width > 0`), '手机端搜索不可见');
  assert.ok(await page.evalJs(`document.documentElement.scrollWidth <= window.innerWidth`), '手机端横向溢出');
  await search('search-fixture-40');
  await select('article');
  assert.equal(await page.evalJs(`document.querySelectorAll('#post-list > article').length`), 1);
  assert.deepEqual(page.errors, []);
  console.log('feed-search: 动态/静态站类型筛选、分页高亮、安全转义、快速切换、恢复时间线与手机端通过');
 } finally {
  page?.close();
  server?.close();
  sql("DELETE FROM posts WHERE content LIKE 'search-fixture-%'; DELETE FROM articles WHERE title LIKE 'search-fixture-%';");
 }
})().catch(error => { console.error(error); process.exitCode=1; });
