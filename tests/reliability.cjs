const assert = require('node:assert/strict');
const fs = require('node:fs');
const { execFileSync } = require('node:child_process');
const { openPage } = require('./cdp.cjs');

(async () => {
 const [base, root] = process.argv.slice(2);
 const stateResponse = await fetch(base + '/api?action=state');
 let cookie = stateResponse.headers.get('set-cookie').split(';')[0];
 const state = await stateResponse.json();
 const call = async (action, data) => {
  const response = await fetch(base + '/api?action=' + action, {method: data ? 'POST' : 'GET', headers: {Cookie: cookie, 'X-CSRF-Token': state.csrf, ...(data instanceof FormData ? {} : {'Content-Type':'application/json'})}, body: data ? data instanceof FormData ? data : JSON.stringify(data) : undefined});
  if (response.headers.get('set-cookie')) cookie = response.headers.get('set-cookie').split(';')[0];
  return {status: response.status, data: await response.json()};
 };
 const sql = query => execFileSync('sqlite3', [root + '/var/blog.sqlite', query], {encoding:'utf8'}).trim();
 assert.equal((await call('login', {username:'tester',password:'Password12345'})).status, 200);
 const bytes = fs.readFileSync(root + '/walls/w1.jpg');
 const form = new FormData();
 form.set('content','reliability-original');
 form.append('images[]',new File([bytes],'old.jpg',{type:'image/jpeg'}));
 assert.equal((await call('save-post', form)).status, 200);
 const post = (await call('posts')).data.posts.find(p=>p.content==='reliability-original');
 const uploads = root + '/var/uploads';
 const beforeFiles = fs.readdirSync(uploads).sort();
 const edit = new FormData();
 edit.set('id',post.id); edit.set('content','must not save'); edit.set('keep','[]');
 edit.append('images[]',new File([bytes],'new.jpg',{type:'image/jpeg'}));
 edit.append('images[]',new File(['invalid'],'invalid.jpg',{type:'image/jpeg'}));
 assert.equal((await call('save-post', edit)).status, 422);
 assert.deepEqual(fs.readdirSync(uploads).sort(),beforeFiles,'失败上传留下孤立文件或删除旧图');
 assert.equal(sql('SELECT content FROM posts WHERE id='+post.id),'reliability-original');
 const backup = (await call('backup-export')).data;
 const originalName = post.images[0].ref.slice(3);
 const originalBytes = fs.readFileSync(uploads+'/'+originalName);
 const blockedName = 'ffffffffffffffffffffffffffffffff.jpg';
 fs.mkdirSync(uploads+'/'+blockedName);
 try {
  backup.uploads = {[originalName]: Buffer.from('replacement').toString('base64'), [blockedName]: bytes.toString('base64')};
  assert.equal((await call('backup-import', backup)).status, 500);
  assert.deepEqual(fs.readFileSync(uploads+'/'+originalName), originalBytes, '失败导入未恢复原图片');
  assert.equal(sql('SELECT content FROM posts WHERE id='+post.id),'reliability-original');
 } finally { fs.rmdirSync(uploads+'/'+blockedName); }
 assert.deepEqual(fs.readdirSync(uploads).sort(), beforeFiles, '失败导入留下临时文件');
 const prior = sql("SELECT key || '=' || value FROM settings WHERE key LIKE 'plugin:douban:%' ORDER BY key");
 assert.equal((await call('save-plugin',{id:'douban',values:{user_id:'123456',category:'invalid'}})).status,422);
 assert.equal(sql("SELECT key || '=' || value FROM settings WHERE key LIKE 'plugin:douban:%' ORDER BY key"),prior,'无效配置被部分写入');
 const wallsBefore = sql('SELECT COUNT(*) FROM wallpapers');
 fs.renameSync(root+'/walls',root+'/walls-offline');
 try {
  assert.equal((await call('sync-plugin',{id:'wallpapers'})).status,500);
  assert.equal(sql('SELECT COUNT(*) FROM wallpapers'),wallsBefore,'目录离线清空了登记');
  assert.equal((await call('plugin-data&id=wallpapers')).status,200);
 } finally { fs.renameSync(root+'/walls-offline',root+'/walls'); }
 sql("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<65) INSERT INTO posts(content,created_at,updated_at) SELECT 'page-test-'||x,'2020-01-01','2020-01-01' FROM n;");
 try {
  let cursor = '', ids = [];
  do {
   const page = (await call('posts&limit=17&q=page-test-&cursor='+encodeURIComponent(cursor))).data;
   assert.ok(page.posts.length<=17); ids.push(...page.posts.map(p=>p.id)); cursor=page.next_cursor;
  } while(cursor);
  assert.equal(ids.length,65); assert.equal(new Set(ids).size,65);
  const page = await openPage(base);
  try {
   await page.goto(base + '/');
   for (let i = 0; i < 16; i++) {
    await page.evalJs(`window.scrollTo(0, document.body.scrollHeight)`);
    await new Promise(resolve => setTimeout(resolve, 250));
   }
   assert.equal(await page.evalJs(`(async () => { const {store} = await import('/assets/lib/store.js'); return store.posts.filter(p => p.content.startsWith('page-test-')).length; })()`), 65, '首页滚动未加载全部历史');
   sql('UPDATE wallpapers SET bytes=bytes+1');
   try {
    const refreshed = await page.evalJs(`(async () => {
     const {store} = await import('/assets/lib/store.js');
     const url = document.querySelector('script[src*="/plugin-asset/wallpapers/plugin.js"]').src;
     const {refreshWallpapers} = await import(url);
     const posts = store.posts, cursor = store.postsNext, y = window.scrollY;
     const list = document.querySelector('#post-list');
     const changed = await refreshWallpapers();
     return {changed, samePosts: store.posts === posts, sameCursor: store.postsNext === cursor, sameList: list === document.querySelector('#post-list'), sameScroll: Math.abs(window.scrollY-y)<2};
    })()`);
    assert.deepEqual(refreshed,{changed:true,samePosts:true,sameCursor:true,sameList:true,sameScroll:true},'壁纸刷新破坏阅读位置或历史分页');
   } finally { sql('UPDATE wallpapers SET bytes=bytes-1'); }
   await page.evalJs(`const search = document.querySelector('#feed-search'); search.value = 'page-test-65'; search.dispatchEvent(new Event('input'));`);
   await new Promise(resolve => setTimeout(resolve, 650));
   assert.deepEqual(await page.evalJs(`(async () => { const {store} = await import('/assets/lib/store.js'); return store.posts.map(p => p.content); })()`), ['page-test-65']);
   await page.evalJs(`document.querySelector('a[href="/articles/"]').click()`);
   await new Promise(resolve => setTimeout(resolve, 250));
   assert.equal(await page.evalJs(`(async () => { const {store} = await import('/assets/lib/store.js'); return store.posts.filter(p => p.content.startsWith('page-test-')).length; })()`), 65, '离开搜索未恢复时间线');
   assert.deepEqual(page.errors, []);
  } finally { page.close(); }
 } finally { sql("DELETE FROM posts WHERE content LIKE 'page-test-%'"); }
 const summaries = (await call('articles')).data.articles;
 assert.ok(summaries.length && !('content' in summaries[0]) && !('html' in summaries[0]));
 assert.ok((await call('articles&id='+summaries[0].id)).data.article.html);
 for (const action of ['purge-plugin-posts', 'remove-plugin']) {
  const upload = new FormData();
  upload.set('content','plugin-cleanup-test');
  upload.append('images[]',new File([bytes],'cleanup.jpg',{type:'image/jpeg'}));
  assert.equal((await call('save-post',upload)).status,200);
  const target = (await call('posts')).data.posts.find(p=>p.content==='plugin-cleanup-test');
  const image = uploads+'/'+target.images[0].ref.slice(3);
  sql(`UPDATE posts SET source='douban', source_key='cleanup' WHERE id=${target.id};
   INSERT INTO comments(kind,target_id,author,content,created_at) VALUES('post',${target.id},'test','cleanup','2020'),('article',${target.id},'test','keep','2020');
   INSERT INTO post_likes VALUES(${target.id},'cleanup','2020');
   INSERT INTO plugin_tombstones VALUES('cleanup','douban','2020');
   INSERT OR REPLACE INTO settings VALUES('plugin:douban:enabled','1');
   CREATE TRIGGER fail_cleanup BEFORE DELETE ON plugin_tombstones WHEN OLD.plugin='douban' BEGIN SELECT RAISE(ABORT,'injected cleanup failure'); END;`);
  try {
   const failed = await fetch(base+'/api?action='+action,{method:'POST',headers:{Cookie:cookie,'X-CSRF-Token':state.csrf,'Content-Type':'application/json'},body:JSON.stringify({id:'douban',purge:true})});
   assert.equal(failed.status,500);
   assert.equal(sql(`SELECT COUNT(*) FROM posts WHERE id=${target.id}`),'1');
   assert.equal(sql(`SELECT COUNT(*) FROM comments WHERE kind='post' AND target_id=${target.id}`),'1');
   assert.equal(sql(`SELECT COUNT(*) FROM post_likes WHERE post_id=${target.id}`),'1');
   assert.ok(fs.existsSync(image),'失败清理删除了图片');
  } finally { sql('DROP TRIGGER fail_cleanup'); }
  assert.equal((await call(action,{id:'douban',purge:true})).status,200);
  assert.equal(sql(`SELECT COUNT(*) FROM posts WHERE id=${target.id}`),'0');
  assert.equal(sql(`SELECT COUNT(*) FROM comments WHERE kind='post' AND target_id=${target.id}`),'0');
  assert.equal(sql(`SELECT COUNT(*) FROM post_likes WHERE post_id=${target.id}`),'0');
  assert.equal(sql(`SELECT COUNT(*) FROM comments WHERE kind='article' AND target_id=${target.id}`),'1');
  assert.ok(!fs.existsSync(image),'成功清理遗留图片');
  assert.equal(sql("SELECT COUNT(*) FROM plugin_tombstones WHERE plugin='douban'"),'0');
  assert.equal(sql("SELECT value FROM settings WHERE key='plugin:douban:enabled'"), action==='remove-plugin'?'0':'1');
  sql(`DELETE FROM comments WHERE kind='article' AND target_id=${target.id}`);
 }
 await call('delete-post',{id:post.id});
 console.log('reliability: 失败回滚、插件清理、配置原子性、离线目录、分页搜索、壁纸刷新阅读位置和文章详情通过');
})().catch(error=>{console.error(error);process.exit(1);});
