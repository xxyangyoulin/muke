const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const {openPage} = require('./cdp.cjs');

(async () => {
 const [base, root] = process.argv.slice(2);
 const response = await fetch(base + '/api?action=state');
 let cookie = response.headers.get('set-cookie').split(';')[0];
 const state = await response.json();
 const call = async (action, data) => {
  const res = await fetch(base + '/api?action=' + action, {method:data ? 'POST' : 'GET', headers:{Cookie:cookie, 'X-CSRF-Token':state.csrf, 'Content-Type':'application/json'}, body:data ? JSON.stringify(data) : undefined});
  if (res.headers.get('set-cookie')) cookie = res.headers.get('set-cookie').split(';')[0];
  return {status:res.status, data:await res.json()};
 };
 assert.equal((await call('login', {username:'tester', password:'Password12345'})).status, 200);
 const original = (await call('state')).data.options;
 const keys = ['comment_provider', 'giscus_repo', 'giscus_repo_id', 'giscus_category_id'];
 const config = {comment_provider:'giscus', giscus_repo:'example/comments', giscus_repo_id:'R_test', giscus_category_id:'DIC_test'};
 let page;
 let failStats = false;
 const stats = http.createServer((req, res) => {
  const query = new URL(req.url, 'http://localhost').searchParams;
  res.setHeader('Content-Type', 'application/json');
  if (failStats) { res.writeHead(503); res.end(JSON.stringify({error:'Unavailable'})); return; }
  const term = query.get('term');
  if (!['post:1', 'article:1'].includes(term)) { res.writeHead(404); res.end(JSON.stringify({error:'Discussion not found'})); return; }
  if (query.get('strict') !== 'true') { res.writeHead(400); res.end('{}'); return; }
  const post = term === 'post:1';
  const more = post && !query.get('after');
  res.end(JSON.stringify({discussion:{totalCommentCount:post ? 51 : 2, totalReplyCount:post ? (more ? 3 : 4) : 1, reactions:{HEART:{count:post ? 9 : 4, viewerHasReacted:false}}, pageInfo:{hasNextPage:more, hasPreviousPage:!!query.get('after'), endCursor:more ? 'next' : null}}}));
 });
 await new Promise(resolve => stats.listen(0, '127.0.0.1', resolve));
 const commentsPath = root + '/src/Comments.php';
 const commentsSource = fs.readFileSync(commentsPath, 'utf8');
 fs.writeFileSync(commentsPath, commentsSource.replace('https://giscus.app/api/discussions?', 'http://127.0.0.1:' + stats.address().port + '/api/discussions?'));
 try {
  assert.equal((await call('save-settings', {...config, giscus_repo:'https://example.com'})).status, 422);
  assert.equal((await call('state')).data.options.comment_provider, original.comment_provider);
  assert.equal((await call('save-settings', {...config, giscus_category_id:''})).status, 422);
  assert.equal((await call('save-settings', config)).status, 200);
  const html = await fetch(base);
  assert.ok(html.headers.get('content-security-policy').includes("frame-src 'self' https://giscus.app"));
  page = await openPage(base);
  page.ws.addEventListener('message', event => {
   const message = JSON.parse(event.data);
   if (message.method === 'Fetch.requestPaused') page.send('Fetch.fulfillRequest', {requestId:message.params.requestId, responseCode:200, responseHeaders:[{name:'Content-Type', value:'text/html'}], body:Buffer.from('<!doctype html><title>giscus test</title>').toString('base64')}).catch(error => { if (!error.message.includes('Invalid InterceptionId')) throw error; });
  });
  await page.send('Fetch.enable', {patterns:[{urlPattern:'https://giscus.app/*'}]});
  await page.goto(base + '/article/1/?giscus=test-session');
  assert.equal(await page.evalJs(`new URL(location.href).searchParams.has('giscus')`), false);
  assert.equal(await page.evalJs(`JSON.parse(localStorage.getItem('giscus-session'))`), 'test-session');
  const frame = await page.evalJs(`document.querySelector('iframe[data-giscus]').src`);
  assert.equal(new URL(frame).searchParams.get('term'), 'article:1');
  assert.equal(new URL(frame).searchParams.get('session'), 'test-session');
  const result = await page.evalJs(`(async () => {
   const {store} = await import('/assets/lib/store.js');
   const {mountComments} = await import('/assets/components/comments.js');
   const host = document.createElement('div'); document.body.append(host);
   store.isStatic = true;
   mountComments('post', 1, 99, host);
   const frames = [...document.querySelectorAll('iframe[data-giscus]')];
   const last = frames.at(-1);
   const loading = last.style.display === 'none' && host.querySelector('[role=status]')?.textContent === '评论加载中…';
   window.dispatchEvent(new MessageEvent('message', {origin:'https://evil.example', source:last.contentWindow, data:{giscus:{resizeHeight:700}}}));
   const ignored = last.style.height === '';
   window.dispatchEvent(new MessageEvent('message', {origin:'https://giscus.app', source:last.contentWindow, data:{giscus:{resizeHeight:700}}}));
   const isolated = frames[0].style.height === '' && last.style.height === '700px';
   const term = new URL(last.src).searchParams.get('term');
   window.dispatchEvent(new MessageEvent('message', {origin:'https://giscus.app', source:last.contentWindow, data:{giscus:{signOut:true}}}));
   const signedOut = localStorage.getItem('giscus-session') === null && frames.every(f => !new URL(f.src).searchParams.has('session'));
   return {ignored,isolated,term,signedOut,loading};
  })()`);
  assert.deepEqual(result, {ignored:true, isolated:true, term:'post:1', signedOut:true, loading:true});
  await new Promise(resolve => setTimeout(resolve, 500));
  assert.equal(await page.evalJs(`document.querySelector('iframe[data-giscus="post:1"]').style.display`), '');
  await page.goto(base + '/');
  assert.equal(await page.evalJs(`!!document.querySelector('[data-post-comments]')`), true);
  await page.evalJs(`document.querySelector('[data-post-comments]').click()`);
  assert.ok(await page.evalJs(`!!document.querySelector('iframe[data-giscus^="post:"]')`));
  const reactions = await page.evalJs(`(async () => {
   const {store} = await import('/assets/lib/store.js');
   const {postHTML} = await import('/assets/lib/ui.js');
   store.isStatic = true;
   const button = document.querySelector('[data-giscus-like]');
   const id = button.dataset.giscusLike;
   button.click();
   const frame = document.querySelector('iframe[data-giscus="post:' + id + '"]');
   const send = (count, viewerHasReacted) => window.dispatchEvent(new MessageEvent('message', {origin:'https://giscus.app', source:frame.contentWindow, data:{giscus:{discussion:{reactions:{HEART:{count, viewerHasReacted}}}}}}));
   send(7, true);
   const liked = button.getAttribute('aria-pressed') === 'true' && button.querySelector('span').textContent === '7';
   const rerendered = document.createElement('div');
   rerendered.innerHTML = postHTML(store.posts.find(post => String(post.id) === id));
   const retained = rerendered.querySelector('[data-giscus-like]').getAttribute('aria-pressed') === 'true' && rerendered.querySelector('[data-giscus-like] span').textContent === '7';
   send(6, false);
   return {liked,retained,unliked:button.getAttribute('aria-pressed') === 'false' && button.querySelector('span').textContent === '6', nativeButtons:document.querySelectorAll('[data-like]').length, metadata:new URL(frame.src).searchParams.get('emitMetadata')};
  })()`);
  assert.deepEqual(reactions, {liked:true,retained:true,unliked:true,nativeButtons:0,metadata:'1'});
  assert.equal((await call('export', {})).status, 200);
  const content = JSON.parse(fs.readFileSync(root + '/dist/assets/content.json', 'utf8'));
  const articles = JSON.parse(fs.readFileSync(root + '/dist/assets/articles.json', 'utf8'));
  assert.deepEqual(content.posts.find(post => Number(post.id) === 1).giscus_counts, {comments:58,likes:9});
  assert.deepEqual(articles.articles.find(article => Number(article.id) === 1).giscus_counts, {comments:3,likes:4});
  assert.ok(content.posts.filter(post => Number(post.id) !== 1).every(post => post.giscus_counts.comments === 0 && post.giscus_counts.likes === 0));
  assert.deepEqual(await page.evalJs(`(async () => {
   const {postHTML} = await import('/assets/lib/ui.js');
   const {store} = await import('/assets/lib/store.js');
   const {giscusReactions,giscusComments} = await import('/assets/components/giscus.js');
   giscusReactions.clear(); giscusComments.clear();
   const post = {...store.posts[0], id:987654, like_count:999, comment_count:999, giscus_counts:{comments:58,likes:9}};
   const host = document.createElement('div');host.innerHTML = postHTML(post);
   return [host.querySelector('[data-giscus-like] span').textContent,host.querySelector('[data-giscus-comments]').textContent,host.querySelector('[data-giscus-like]').getAttribute('aria-pressed')];
  })()`), ['9','58 评论','false']);
  const previous = fs.readFileSync(root + '/dist/assets/content.json', 'utf8');
  failStats = true;
  assert.ok((await call('export', {})).status >= 400);
  assert.equal(fs.readFileSync(root + '/dist/assets/content.json', 'utf8'), previous, '统计失败覆盖了旧导出');
  const exported = fs.readFileSync(root + '/dist/article/1/index.html', 'utf8');
  assert.ok(exported.includes('giscus.app/zh-CN/widget?') && exported.includes('term=article%3A1'));
  assert.ok(fs.readFileSync(root + '/dist/_headers', 'utf8').includes("frame-src 'self' https://giscus.app"));
  assert.ok(fs.existsSync(root + '/dist/assets/components/giscus.js'));
  console.log('giscus: 配置校验、授权回跳、文章/动态隔离、多 iframe 消息校验、退出登录、加载提示、爱心同步、导出统计分页、无讨论归零与失败保留旧版通过');
 } finally {
  page?.close();
  fs.writeFileSync(commentsPath, commentsSource);
  await new Promise(resolve => stats.close(resolve));
  await call('save-settings', Object.fromEntries(keys.map(key => [key, original[key] || ''])));
 }
})().catch(error => { console.error(error); process.exitCode = 1; });
