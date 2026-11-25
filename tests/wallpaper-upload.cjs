const assert = require('node:assert/strict');
const { openPage } = require('./cdp.cjs');
const { execFileSync, spawn } = require('node:child_process');
const fs = require('node:fs');

(async () => {
 const base = process.argv[2] || 'http://127.0.0.1:8771';
 const p = await openPage(base);
 try {
  await p.goto(base + '/wallpapers');
  assert.equal(await p.evalJs(`!!document.querySelector('#upload-wallpaper')`), false);
  assert.equal(await p.evalJs(`(async () => {
   const state = await (await fetch('/api?action=state')).json();
   return (await fetch('/api?action=plugin-admin', {method:'POST', headers:{'Content-Type':'application/json','X-CSRF-Token':state.csrf}, body:JSON.stringify({id:'wallpapers',op:'upload'})})).status;
  })()`), 401);
  await p.evalJs(`(async () => {
   const state = await (await fetch('/api?action=state')).json();
   const response = await fetch('/api?action=login', {method:'POST', headers:{'Content-Type':'application/json','X-CSRF-Token':state.csrf}, body:JSON.stringify({username:'tester',password:'Password12345'})});
   if (!response.ok) throw new Error('登录失败');
  })()`);
  await p.goto(base + '/wallpapers');
  assert.equal(await p.evalJs(`!!document.querySelector('#upload-wallpaper')`), true);
  assert.equal(await p.evalJs(`document.querySelector('#wallpaper-file').multiple`), true);
  for (const width of [1440, 1024, 390]) {
   await p.send('Emulation.setDeviceMetricsOverride', {width, height:900, deviceScaleFactor:1, mobile:false});
   assert.equal(await p.evalJs(`(() => {
    const upload = document.querySelector('#upload-wallpaper');
    const a = upload.getBoundingClientRect(), b = document.querySelector('#refresh-gallery').getBoundingClientRect();
    const search = document.querySelector('.gallery-search').getBoundingClientRect();
    return upload.textContent === '上传' && a.left >= b.right && b.left >= search.right && Math.abs(a.top - b.top) < 1 && Math.abs((b.top + b.bottom - search.top - search.bottom) / 2) < 1 && a.right <= innerWidth;
   })()`), true, '同步和上传应位于搜索框右侧且同排、不溢出');
  }
  await p.send('Emulation.clearDeviceMetricsOverride');
  for (const [name, width, height, expected] of [
   ['landscape.png', 80, 40, 'landscape.png'],
   ['portrait.png', 40, 80, 'Portrait/portrait.png'],
   ['square.png', 40, 40, 'Portrait/square.png'],
   ['landscape.png', 100, 50, null],
  ]) {
   await p.evalJs(`(async () => {
    const canvas = document.createElement('canvas');
    canvas.width = ${width}; canvas.height = ${height};
    const blob = await new Promise(resolve => canvas.toBlob(resolve));
    const transfer = new DataTransfer();
    transfer.items.add(new File([blob], ${JSON.stringify(name)}, {type:'image/png'}));
    const picker = document.querySelector('#wallpaper-file');
    picker.files = transfer.files;
    await picker.onchange();
   })()`);
   const items = await p.evalJs(`fetch('/api?action=plugin-data&id=wallpapers').then(r=>r.json()).then(r=>r.data.items)`);
   assert.equal(await p.evalJs(`document.querySelector('#upload-wallpaper').textContent`), '上传');
   const uploaded = items.find(w => w.width === width && w.height === height);
   assert.ok(uploaded, `上传后未登记 ${name}`);
   if (expected) assert.equal(uploaded.path, expected);
   else {
    assert.match(uploaded.path, /^landscape-[a-f0-9]+\.png$/);
    assert.ok(items.some(w => w.path === 'landscape.png' && w.width === 80));
   }
   assert.equal(await p.evalJs(`!!document.querySelector('[data-wall-id="${uploaded.id}"]')`), true);
   assert.equal((await fetch(base + '/image?file=' + encodeURIComponent(uploaded.path))).status, 200);
  }
  for (const [csrf, expected] of [[false, 403], [true, 422]]) {
   assert.equal(await p.evalJs(`(async () => {
    const state = await (await fetch('/api?action=state')).json();
    const data = new FormData();
    data.set('id','wallpapers'); data.set('op','upload');
    data.set('count','1');
    data.set('images[]',new File(['not an image'],'invalid.png',{type:'image/png'}));
    return (await fetch('/api?action=plugin-admin',{method:'POST',headers:${csrf ? "{'X-CSRF-Token':state.csrf}" : '{}'},body:data})).status;
   })()`), expected);
  }
  const before = await p.evalJs(`fetch('/api?action=posts').then(r=>r.json()).then(r=>r.posts.length)`);
  await p.evalJs(`(async () => {
   const transfer = new DataTransfer();
   for (let i = 0; i < 6; i++) {
    const canvas = document.createElement('canvas');
    canvas.width = i % 2 ? 600 : 1200; canvas.height = i % 2 ? 1200 : 600;
    const blob = await new Promise(resolve => canvas.toBlob(resolve));
    transfer.items.add(new File([blob], 'batch-' + i + '.png', {type:'image/png'}));
   }
   const picker = document.querySelector('#wallpaper-file');
   picker.files = transfer.files;
   await picker.onchange();
  })()`);
  let posts = await p.evalJs(`fetch('/api?action=posts').then(r=>r.json()).then(r=>r.posts)`);
  assert.equal(posts.length, before + 1, '批量上传必须仅生成一条动态');
  const batch = posts.find(post => post.images.length === 6);
  assert.ok(batch, '超过四张图片也必须全部返回');
  const walls = await p.evalJs(`fetch('/api?action=plugin-data&id=wallpapers').then(r=>r.json()).then(r=>r.data.items)`);
  for (let i = 0; i < 6; i++) assert.ok(walls.some(w => w.path === (i % 2 ? 'Portrait/' : '') + 'batch-' + i + '.png'));
  await p.goto(base + '/');
  assert.equal(await p.evalJs(`document.querySelectorAll('#post-${batch.id} .post-image').length`), 6);
  for (const width of [1440, 390]) {
   await p.send('Emulation.setDeviceMetricsOverride', {width, height:900, deviceScaleFactor:1, mobile:false});
   const layout = await p.evalJs(`(async () => {
    const grid = document.querySelector('#post-${batch.id} .post-images');
    const images = [...grid.querySelectorAll('img')];
    await Promise.all(images.map(img => img.decode()));
    return {
     left: Math.abs(grid.firstElementChild.getBoundingClientRect().left - grid.getBoundingClientRect().left),
     tiles: [...grid.children].map(tile => { const rect = tile.getBoundingClientRect(); return {top:rect.top, width:rect.width, height:rect.height, radius:parseFloat(getComputedStyle(tile).borderRadius)}; }),
     images: images.map(img => {
      const rect = img.getBoundingClientRect();
      return {width:rect.width, height:rect.height, ratio:img.naturalWidth / img.naturalHeight, fit:getComputedStyle(img).objectFit, within:rect.right <= grid.getBoundingClientRect().right + 1};
     })
    };
   })()`);
   assert.ok(layout.left < 1, '壁纸图片须左对齐');
   assert.equal(new Set(layout.tiles.slice(0, 3).map(tile => tile.top)).size, 1, '前三张图片同排');
   assert.ok(layout.tiles[3].top > layout.tiles[0].top, '第四张图片须换行');
   for (const tile of layout.tiles) {
    assert.ok(tile.radius > 0, '多图使用圆角矩形');
    assert.ok(Math.abs(tile.width - tile.height) < 1, '多图卡片宽高相等');
   }
   for (const img of layout.images) {
    assert.ok(img.height > 0 && img.height <= 360, '壁纸图片最大高度为 360px');
    assert.ok(Math.abs(img.width - img.height) < 1, '多图须填满正方形卡片');
    assert.equal(img.fit, 'cover');
    assert.ok(img.within, '窄屏图片不得溢出动态');
   }
  }
  await p.send('Emulation.clearDeviceMetricsOverride');
  await p.evalJs(`document.querySelectorAll('#post-${batch.id} .post-image')[5].click()`);
  assert.equal(await p.evalJs(`document.querySelector('#lb-counter').textContent`), '6 / 6');
  await p.evalJs(`document.querySelector('[data-lb-close]').click()`);
  await p.evalJs(`(async () => {
   const { api } = await import('/assets/lib/api.js');
   await api('sync-plugin', {id:'wallpapers'});
  })()`);
  posts = await p.evalJs(`fetch('/api?action=posts').then(r=>r.json()).then(r=>r.posts)`);
  assert.equal(posts.length, before + 1, '同步不得重复发布批量图片');
  const deletedWall = walls.find(w => w.name === 'batch-0.png');
  await p.evalJs(`(async () => {
   const { api } = await import('/assets/lib/api.js');
   await api('plugin-admin', {id:'wallpapers',op:'delete',wall_id:'${deletedWall.id}'});
  })()`);
  posts = await p.evalJs(`fetch('/api?action=posts').then(r=>r.json()).then(r=>r.posts)`);
  assert.equal(posts.find(post => post.id === batch.id).images.length, 5, '删除单张壁纸须保留同批其他图片');
  const root = process.argv[3];
  const sql = query => execFileSync('sqlite3', [root+'/var/blog.sqlite',query],{encoding:'utf8'}).trim();
  sql(`INSERT INTO comments(kind,target_id,author,content,created_at) VALUES('post',${batch.id},'test','batch comment','2020'); INSERT INTO post_likes VALUES(${batch.id},'batch-like','2020');`);
  await p.goto(base + '/wallpapers');
  const remaining = walls.filter(w => /^batch-[1-5]\.png$/.test(w.name));
  const failedWall = remaining[0];
  sql(`CREATE TRIGGER fail_wall_delete BEFORE DELETE ON wallpapers WHEN OLD.id='${failedWall.id}' BEGIN SELECT RAISE(ABORT,'injected wall failure'); END;`);
  try {
   assert.equal(await p.evalJs(`(async () => {
    const state = await (await fetch('/api?action=state')).json();
    return (await fetch('/api?action=plugin-admin',{method:'POST',headers:{'Content-Type':'application/json','X-CSRF-Token':state.csrf},body:JSON.stringify({id:'wallpapers',op:'delete',wall_id:'${failedWall.id}'})})).status;
   })()`),500);
   assert.ok(fs.existsSync(root+'/walls/'+failedWall.path),'失败删除未恢复源文件');
   assert.equal(JSON.parse(sql(`SELECT images FROM posts WHERE id=${batch.id}`)).length,5);
  } finally { sql('DROP TRIGGER fail_wall_delete'); }
  for (let i=0; i<remaining.length; i++) {
   await p.evalJs(`(async () => {
    document.querySelector('[data-wall-delete="${remaining[i].id}"]').click();
    await document.querySelector('[data-wall-confirm]').onclick();
   })()`);
   const left = remaining.length-i-1;
   assert.equal(sql(`SELECT COUNT(*) FROM posts WHERE id=${batch.id}`),left?'1':'0');
   assert.equal(sql(`SELECT COUNT(*) FROM comments WHERE kind='post' AND target_id=${batch.id}`),left?'1':'0');
   assert.equal(sql(`SELECT COUNT(*) FROM post_likes WHERE post_id=${batch.id}`),left?'1':'0');
   assert.equal(await p.evalJs(`import('/assets/lib/store.js').then(({store})=>store.posts.find(post=>post.id===${batch.id})?.images.length || 0)`),left,'删除后前端动态缓存未同步');
   assert.ok(!fs.existsSync(root+'/walls/'+remaining[i].path));
  }
  await p.evalJs(`document.querySelector('nav a[href="/"]').click()`);
  assert.equal(await p.evalJs(`!!document.querySelector('#post-${batch.id}')`),false,'返回首页仍显示已删动态');
  // 模拟存活的 watcher，仅验证排队，不启动真实部署或访问 Netlify。
  const watcher = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)', 'auto-deploy-test']);
  const pendingPath = root + '/var/deploy.pending.json';
  const watcherPath = root + '/var/deploy.watcher.pid';
  fs.writeFileSync(watcherPath, String(watcher.pid));
  try {
   sql(`INSERT OR REPLACE INTO settings(key,value) VALUES ('auto_deploy','1'),('auto_deploy_debounce','10'),('netlify_token','test-token'),('netlify_site','test-site');`);
   for (const scenario of ['single', 'batch', 'no-post', 'invalid', 'rollback', 'disabled', 'no-token']) {
    fs.rmSync(pendingPath, {force:true});
    if (scenario === 'no-post') sql(`INSERT OR REPLACE INTO settings(key,value) VALUES ('wallpaper_post_template','');`);
    if (scenario === 'disabled') sql(`UPDATE settings SET value='0' WHERE key='auto_deploy';`);
    if (scenario === 'no-token') sql(`UPDATE settings SET value='1' WHERE key='auto_deploy'; DELETE FROM settings WHERE key='netlify_token';`);
    if (scenario === 'rollback') sql(`CREATE TRIGGER fail_wall_upload BEFORE INSERT ON wallpapers BEGIN SELECT RAISE(ABORT,'injected upload failure'); END;`);
    const countBefore = Number(sql('SELECT COUNT(*) FROM posts'));
    const started = Math.floor(Date.now() / 1000);
    const status = await p.evalJs(`(async () => {
     const state = await (await fetch('/api?action=state')).json();
     const data = new FormData();
     data.set('id','wallpapers'); data.set('op','upload');
     data.set('count', '${scenario === 'batch' ? 2 : 1}');
     const canvas = document.createElement('canvas'); canvas.width=60; canvas.height=30;
     const blob = ${scenario === 'invalid' ? "new Blob(['invalid'])" : 'await new Promise(resolve => canvas.toBlob(resolve))'};
     for (let i=0; i<Number(data.get('count')); i++) data.append('images[]',new File([blob],'deploy-${scenario}-'+i+'.png',{type:'image/png'}));
     return (await fetch('/api?action=plugin-admin',{method:'POST',headers:{'X-CSRF-Token':state.csrf},body:data})).status;
    })()`);
    assert.equal(status, scenario === 'invalid' ? 422 : scenario === 'rollback' ? 500 : 200, scenario);
    if (['single','batch','no-post'].includes(scenario)) {
     const pending = JSON.parse(fs.readFileSync(pendingPath,'utf8'));
     assert.ok(pending.touched_at >= started);
     assert.equal(pending.due_at - pending.touched_at, 600, '遵循自动部署防抖设置');
     assert.equal(Number(sql('SELECT COUNT(*) FROM posts')) - countBefore, scenario === 'no-post' ? 0 : 1);
    } else assert.equal(fs.existsSync(pendingPath), false, scenario + ' 不应触发部署');
    if (scenario === 'rollback') sql('DROP TRIGGER fail_wall_upload');
   }
   sql(`INSERT OR REPLACE INTO settings(key,value) VALUES ('auto_deploy','1'),('netlify_token','test-token');`);
   fs.copyFileSync(root + '/walls/landscape.png', root + '/walls/sync-deploy.png');
   await p.evalJs(`import('/assets/lib/api.js').then(({api}) => api('sync-plugin',{id:'wallpapers'}))`);
   assert.ok(fs.existsSync(pendingPath), '目录新增图片且不生成动态时也须部署');
   fs.rmSync(pendingPath);
   await p.evalJs(`import('/assets/lib/api.js').then(({api}) => api('sync-plugin',{id:'wallpapers'}))`);
   assert.equal(fs.existsSync(pendingPath), false, '无变化同步不应触发部署');
   const syncId = sql(`SELECT id FROM wallpapers WHERE name='sync-deploy.png'`);
   await p.evalJs(`import('/assets/lib/api.js').then(({api}) => api('plugin-admin',{id:'wallpapers',op:'delete',wall_id:'${syncId}'}))`);
   assert.ok(fs.existsSync(pendingPath), '删除壁纸须触发部署');
   fs.rmSync(pendingPath);
   fs.copyFileSync(root + '/walls/landscape.png', root + '/walls/sync-deploy.png');
   await p.evalJs(`import('/assets/lib/api.js').then(({api}) => api('sync-plugin',{id:'wallpapers'}))`);
   fs.rmSync(pendingPath);
   fs.unlinkSync(root + '/walls/sync-deploy.png');
   await p.evalJs(`import('/assets/lib/api.js').then(({api}) => api('sync-plugin',{id:'wallpapers'}))`);
   assert.ok(fs.existsSync(pendingPath), '目录文件消失须触发部署');
  } finally {
   watcher.kill();
   fs.rmSync(pendingPath, {force:true});
   fs.rmSync(watcherPath, {force:true});
   sql(`DROP TRIGGER IF EXISTS fail_wall_upload; DELETE FROM settings WHERE key IN ('auto_deploy','auto_deploy_debounce','netlify_token','netlify_site','wallpaper_post_template');`);
  }
  assert.deepEqual(p.errors, []);
  console.log('wallpaper-upload: 单张/批量上传、分目录、合并动态、六图展示与预览、同步去重、删除及权限校验通过');
 } finally { p.close(); }
})().catch(error => { console.error(error); process.exit(1); });
