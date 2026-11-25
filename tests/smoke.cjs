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
  for (const tab of ['overview', 'posts', 'articles', 'plugins', 'comments', 'options', 'settings', 'publish']) {
   await p.evalJs(`[...document.querySelectorAll('[data-admin-tab]')].find(b=>b.dataset.adminTab==='${tab}').click()`);
   await new Promise(r => setTimeout(r, 500));
   ok(`页签渲染 ${tab}`, await p.evalJs(`!!document.querySelector('#admin-body')`));
   if (tab === 'comments') {
    ok('评论默认显示管理列表', await p.evalJs(`!document.querySelector('[data-comments-panel="list"]').hidden && document.querySelector('[data-comments-panel="settings"]').hidden`));
    await p.evalJs(`document.querySelector('[data-comments-tab="settings"]').click()`);
    ok('可切换到评论设置', await p.evalJs(`document.querySelector('[data-comments-panel="list"]').hidden && !document.querySelector('[data-comments-panel="settings"]').hidden && typeof document.querySelector('#comment-provider-form').onsubmit === 'function'`));
    await p.evalJs(`document.querySelector('#comment-provider-form input[name=smtp_host]').value = 'unsaved.example.com'; document.querySelector('[data-comments-tab="list"]').click(); document.querySelector('[data-comments-tab="settings"]').click()`);
    ok('切换评论页签保留未保存输入', await p.evalJs(`document.querySelector('#comment-provider-form input[name=smtp_host]').value === 'unsaved.example.com'`));
    await p.evalJs(`document.querySelector('[data-comments-tab="list"]').click()`);
    ok('可切回评论管理', await p.evalJs(`!document.querySelector('[data-comments-panel="list"]').hidden && document.querySelector('[data-comments-tab="list"]').getAttribute('aria-pressed') === 'true'`));
   }
   if (tab === 'plugins') {
    ok('插件设置位于启用左侧', await p.evalJs(`Array.from(document.querySelectorAll('.plugin-card')).every(card => {
     const button = card.querySelector('.plugin-head .plugin-settings');
     const toggle = card.querySelector('.plugin-enabled');
     return button && button.getBoundingClientRect().right <= toggle.getBoundingClientRect().left && !card.querySelector('.plugin-actions .plugin-settings');
    })`));
    await p.evalJs(`document.querySelector('.plugin-settings').click()`);
    ok('插件设置弹窗可打开', await p.evalJs(`document.querySelector('#plugin-settings-dialog').open`));
    await p.evalJs(`document.querySelector('#plugin-settings-dialog').close()`);
    ok('插件同步显示完整时间', await p.evalJs(`(async () => {
     const {pluginCardHTML} = await import('/assets/views/admin.js');
     const plugin = {id:'test', name:'test', description:'', post_count:1};
     const html = pluginCardHTML({...plugin, last_sync:new Date(2026,10,25,0,5,9).getTime()/1000});
     return html.includes('上次 2026/11/25 00:05:09') && !pluginCardHTML({...plugin, last_sync:0}).includes(' · 上次 ');
    })()`));
   }
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
