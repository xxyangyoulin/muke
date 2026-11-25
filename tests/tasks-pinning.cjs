const assert = require('node:assert/strict');
const fs = require('node:fs');
const {execFileSync} = require('node:child_process');
const {openPage} = require('./cdp.cjs');

(async () => {
 const [base, root] = process.argv.slice(2);
 const sql = query => execFileSync('sqlite3', ['-cmd', '.timeout 5000', root + '/var/blog.sqlite', query], {encoding:'utf8'}).trim();
 const response = await fetch(base + '/api?action=state');
 let cookie = response.headers.get('set-cookie').split(';')[0];
 const state = await response.json();
 const call = async (action, data) => {
  const res = await fetch(base + '/api?action=' + action, {method:data ? 'POST' : 'GET', headers:{Cookie:cookie, 'X-CSRF-Token':state.csrf, 'Content-Type':'application/json'}, body:data ? JSON.stringify(data) : undefined});
  if (res.headers.get('set-cookie')) cookie = res.headers.get('set-cookie').split(';')[0];
  return {status:res.status, data:await res.json()};
 };
 assert.equal((await fetch(base + '/api?action=tasks')).status, 401);
 assert.equal((await call('pin-post', {id:1, pinned:1})).status, 401);
 assert.equal((await call('save-backup-settings', {enabled:1, interval_hours:24, keep:7})).status, 401);
 assert.equal((await call('login', {username:'tester', password:'Password12345'})).status, 200);
 const before = sql("SELECT key||'='||value FROM settings WHERE key LIKE 'backup_%' ORDER BY key");
 assert.equal((await call('save-backup-settings', {enabled:1, interval_hours:0, keep:2})).status, 422);
 assert.equal(sql("SELECT key||'='||value FROM settings WHERE key LIKE 'backup_%' ORDER BY key"), before);
 assert.equal((await call('save-backup-settings', {enabled:0, interval_hours:12, keep:3})).status, 200);
 const tasks = (await call('tasks')).data;
 assert.equal(tasks.backup.enabled, false);
 assert.equal(tasks.backup.interval_hours, 12);
 assert.equal(tasks.backup.keep, 3);
 assert.equal(tasks.backup.next_run, null);
 assert.ok(Array.isArray(tasks.plugins) && Array.isArray(tasks.backup.files));
 assert.equal(typeof tasks.worker.running, 'boolean');
 sql("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<35) INSERT INTO posts(content,created_at,updated_at) SELECT 'pin-test-'||x,'2001-01-01','2001-01-01' FROM n;");
 const first = Number(sql("SELECT id FROM posts WHERE content='pin-test-1'"));
 const second = Number(sql("SELECT id FROM posts WHERE content='pin-test-2'"));
 let page;
 try {
  assert.equal((await call('pin-post', {id:first, pinned:2})).status, 422);
  assert.equal((await call('pin-post', {id:999999999, pinned:1})).status, 404);
  assert.equal((await call('pin-post', {id:first, pinned:1})).status, 200);
  assert.equal((await call('pin-post', {id:second, pinned:1})).status, 200);
  assert.equal((await call('posts')).data.posts[0].id, second);
  let cursor = '', ids = [];
  do {
   const result = (await call('posts&q=pin-test-&limit=1&cursor=' + encodeURIComponent(cursor))).data;
   ids.push(...result.posts.map(post => post.id));
   cursor = result.next_cursor;
  } while (cursor);
  assert.equal(ids.length, 35);
  assert.equal(new Set(ids).size, 35);
  assert.deepEqual(ids.slice(0, 2), [second, first]);
  const backup = (await call('backup-export')).data;
  assert.equal(backup.posts.find(post => post.id === first).pinned, 1);
  assert.equal((await call('backup-import', backup)).status, 200);
  assert.equal(sql('SELECT pinned FROM posts WHERE id=' + first), '1');
  page = await openPage(base);
  await page.goto(base + '/');
  assert.equal(await page.evalJs(`document.querySelector('#post-list > article').id`), 'post-' + second);
  assert.equal(await page.evalJs(`!!document.querySelector('[data-pin]')`), false);
  assert.deepEqual(await page.evalJs(`(async () => {
   const {store} = await import('/assets/lib/store.js');
   const {feedItems} = await import('/assets/views/feed.js');
   const old = {posts:store.posts, postsNext:store.postsNext, articles:store.articles, articlesNext:store.articlesNext};
   try {
    store.posts = [{id:1, pinned:1, created_at:'2001-01-01'}]; store.postsNext='more';
    store.articles = [{id:9, created_at:'2026-01-01'}]; store.articlesNext=null;
    const pinnedPage = feedItems().map(item => item.type + item.id);
    store.posts.push({id:2, pinned:0, created_at:'2025-01-01'});
    return [pinnedPage, feedItems().map(item => item.type + item.id)];
   } finally { Object.assign(store, old); }
  })()`), [['post1'], ['post1', 'article9', 'post2']], '仅置顶的分页边界破坏文章混排');
  await page.goto(base + '/admin?tab=tasks');
  await page.evalJs(`document.querySelector('#auth-form input[name=username]').value='tester';document.querySelector('#auth-form input[name=password]').value='Password12345';document.querySelector('#auth-form').requestSubmit()`);
  for (let i=0; i<20 && !(await page.evalJs(`!!document.querySelector('#scheduled-backup-form')`)); i++) await new Promise(r => setTimeout(r,200));
  assert.ok(await page.evalJs(`!!document.querySelector('#scheduled-backup-form')`));
  assert.ok(await page.evalJs(`document.querySelector('#task-status').textContent.includes('插件同步')`));
  await page.evalJs(`document.querySelector('#scheduled-backup-form input[name=keep]').value='4';document.querySelector('#scheduled-backup-form').requestSubmit()`);
  for (let i=0; i<20 && sql("SELECT value FROM settings WHERE key='backup_keep'") !== '4'; i++) await new Promise(r => setTimeout(r,100));
  assert.equal(sql("SELECT value FROM settings WHERE key='backup_keep'"), '4');
  await page.goto(base + '/');
  await page.evalJs(`document.querySelector('[data-pin="${second}"]').click()`);
  for (let i=0; i<20 && sql('SELECT pinned FROM posts WHERE id=' + second) !== '0'; i++) await new Promise(r => setTimeout(r,100));
  assert.equal(sql('SELECT pinned FROM posts WHERE id=' + second), '0');
  await page.goto(base + '/');
  assert.equal(await page.evalJs(`document.querySelector('#post-list > article').id`), 'post-' + first);
  assert.equal((await call('export', {})).status, 200);
  const html = fs.readFileSync(root + '/dist/index.html', 'utf8');
  assert.match(html, /pinned-label/);
  assert.ok(html.indexOf('id="post-' + first + '"') < html.indexOf('id="post-' + second + '"'));
  assert.equal(JSON.parse(fs.readFileSync(root + '/dist/assets/content.json')).posts[0].id, first);
  assert.deepEqual(page.errors, []);
 } finally {
  page?.close();
  sql("DELETE FROM posts WHERE content LIKE 'pin-test-%'; DELETE FROM settings WHERE key LIKE 'backup_%';");
 }
 console.log('tasks-pinning: 权限、备份配置、置顶/取消、分页、备份导入、任务页与静态导出通过');
})().catch(error => { console.error(error); process.exitCode=1; });
