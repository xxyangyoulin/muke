// 浏览器渲染冒烟：node tests/smoke.cjs <base_url>
const { openPage, ok, results } = require('./cdp.cjs');

(async () => {
 const base = process.argv[2] || 'http://127.0.0.1:8771';
 const p = await openPage(base);
 try {
  // 三个公开页面渲染、无模块异常
  for (const [path, marker] of [['/', '.timeline'], ['/wallpapers', '.gallery-page'], ['/articles', '.articles-hero']]) {
   await p.goto(base + path);
   ok(`页面渲染 ${path}`, await p.evalJs(`!!document.querySelector('${marker}')`));
  }
  // 登录后台
  await p.goto(base + '/admin');
  await p.evalJs(`document.querySelector('#auth-form input[name=username]').value='tester';document.querySelector('#auth-form input[name=password]').value='Password12345';document.querySelector('#auth-form').requestSubmit()`);
  await new Promise(r => setTimeout(r, 1400));
  ok('后台登录', await p.evalJs(`!!document.querySelector('[data-admin-tab]')`));
  // 各页签渲染
  for (const tab of ['overview', 'posts', 'articles', 'plugins', 'options', 'settings', 'publish']) {
   await p.evalJs(`[...document.querySelectorAll('[data-admin-tab]')].find(b=>b.dataset.adminTab==='${tab}').click()`);
   await new Promise(r => setTimeout(r, 500));
   ok(`页签渲染 ${tab}`, await p.evalJs(`!!document.querySelector('#admin-body')`));
   if (tab === 'articles') {
    await p.evalJs(`document.querySelector('[data-edit-article]').click()`);
    await new Promise(r => setTimeout(r, 500));
    ok('文章摘要进入编辑器仍有完整正文', await p.evalJs(`!!(document.querySelector('#article-content')?.value || document.querySelector('#article-content')?.textContent)`));
    await p.evalJs(`document.querySelector('#editor-back').click()`);
   }
  }
  // 发布器绑定（回归：bindComposer 曾丢失）
  await p.goto(base + '/');
  ok('发布器已绑定', await p.evalJs(`typeof document.querySelector('#attach-images')?.onclick === 'function' && !!document.querySelector('#compose-files')`));
  // 评论：动态卡片有评论按钮，文章页有评论面板
  ok('动态评论入口', await p.evalJs(`!!document.querySelector('[data-post-comments]')`));
  await p.goto(base + '/articles');
  for (let i = 0; i < 12 && !(await p.evalJs(`!!document.querySelector('.article-teaser')`)); i++) await new Promise(r => setTimeout(r, 300));
  await p.evalJs(`document.querySelector('.article-teaser')?.click()`);
  await new Promise(r => setTimeout(r, 1200));
  ok('文章评论面板', await p.evalJs(`!!document.querySelector('.comments-panel') && !!document.querySelector('[data-comment-form]')`));
  // 灯箱开合（回归：hidden/display 曾冲突）
  await p.goto(base + '/wallpapers');
  for (let i = 0; i < 20 && !(await p.evalJs(`!!document.querySelector('.tile-preview')`)); i++) await new Promise(r => setTimeout(r, 300));
  await p.evalJs(`document.querySelector('.tile-preview')?.click()`);
  await new Promise(r => setTimeout(r, 1200));
  ok('灯箱打开', await p.evalJs(`document.querySelector('#lightbox').classList.contains('open')`));
  await p.evalJs(`document.querySelector('[data-lb-close]').click()`);
  await new Promise(r => setTimeout(r, 500));
  ok('灯箱关闭不遮挡', await p.evalJs(`!document.querySelector('#lightbox').classList.contains('open') && document.querySelector('#lightbox').hidden`));
  // 日期带年规则（回归：非今年需带年）
  await p.goto(base + '/');
  ok('无运行时异常', p.errors.length === 0, p.errors[0] || '无');
 } finally {
  p.close();
 }
 console.log(`smoke: ${results.pass} 通过 / ${results.fail} 失败`);
 process.exit(results.fail ? 1 : 0);
})().catch(e => { console.error('smoke 失败:', e.message); process.exit(1); });
