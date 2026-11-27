// RSS 订阅插件 · 前端（/rss 标签页 + 订阅卡片 + 管理面板）
import { store } from '/assets/lib/store.js';
import { api, apiGet } from '/assets/lib/api.js';
import { icon, esc, toast, empty, postHTML, postItems, dateLabel } from '/assets/lib/ui.js';
import { openLightbox } from '/assets/components/lightbox.js';
import { mountInlineComments } from '/assets/components/comments.js';
import { beginView, onViewLeave } from '/assets/lib/lifecycle.js';
import { topTabsHTML } from '/assets/views/feed.js';

const ID = 'rss';
const PAGE_KEY = 'plugin:rss'; // app.js 路由器为插件页设定的 page 值
const rstate = { loaded: false, feeds: [], posts: [], total: 0, nextCursor: null, filter: '', shown: 15 };

// ---- 数据加载（动态站走 plugin-data 端点，静态站读导出的 plugins 数据）----
async function fetchRss(params = {}) {
 return (await apiGet('plugin-data', { id: ID, limit: 30, ...params })).data;
}
async function loadRss() {
 if (store.isStatic) {
  const data = store.pluginsData?.[ID] || {};
  rstate.feeds = data.feeds || [];
  rstate.posts = data.posts || [];
  rstate.total = store.pluginsMeta?.[ID]?.total ?? rstate.posts.length;
 } else {
  const data = await fetchRss(rstate.filter ? { feed: rstate.filter } : {});
  rstate.feeds = data.feeds;
  rstate.posts = data.posts;
  rstate.total = data.total;
  rstate.nextCursor = data.next_cursor;
 }
 rstate.loaded = true;
}
async function reloadRssPage() {
 try { await loadRss(); } catch (e) { toast(e.message); return; }
 if (store.page === PAGE_KEY) renderRssPage();
}

// ---- 列表渲染 ----
// 本页的动态不在 store.posts 里：改写交互属性，避免与 app.js 的全局委托（它只认 store.posts）抢事件
function rssPostHTML(post) {
 return postHTML(post)
  .replace(/data-media-post="/g, 'data-rss-media="')
  .replace(/data-post-comments="/g, 'data-rss-comments="')
  .replace(/data-delete="/g, 'data-rss-delete="')
  .replace(/data-pin="/g, 'data-rss-pin="');
}
function rssFilter() {
 return rstate.filter ? rstate.posts.filter(p => (p.meta?.feed || '') === rstate.filter) : rstate.posts;
}
function rssListHTML() {
 const list = store.isStatic ? rssFilter().slice(0, rstate.shown) : rstate.posts;
 return list.length ? list.map(rssPostHTML).join('') : '';
}
function rssHasMore() {
 return store.isStatic ? rssFilter().length > rstate.shown : !!rstate.nextCursor;
}

// ---- 右栏卡片：管理员管理订阅（添加 / 单源同步 / 退订），访客订阅源列表 ----
function feedRowsHTML() {
 return rstate.feeds.map(f => {
  const last = f.last_sync ? dateLabel(new Date(f.last_sync * 1000).toISOString()) : '';
  return `<div class="rss-feed-row" data-feed-row="${esc(f.id)}"><div class="rss-feed-main"><strong>${esc(f.title)}</strong><span class="small-label">${esc(f.url || '')}${f.count ? ` · ${f.count} 条` : ''}${last ? ` · 同步于 ${esc(last)}` : ''}</span>${f.last_error ? `<span class="rss-feed-error">上次失败：${esc(f.last_error)}</span>` : ''}</div><div class="rss-feed-actions"><button class="text-link" type="button" data-rss-sync-feed="${esc(f.id)}">${icon('refresh')}同步</button><button class="text-link" type="button" data-rss-remove-feed="${esc(f.id)}">${icon('trash')}退订</button></div></div>`;
 }).join('') || '<p class="rss-side-note">还没有订阅源，添加第一个吧。</p>';
}
function adminPanelHTML() {
 return `<section class="side-card rss-admin-panel"><div class="section-heading"><h2>管理订阅</h2><span class="small-label">${rstate.feeds.length} 个源</span></div>
<form id="rss-add-form" class="rss-add-row"><input name="url" type="text" placeholder="订阅源地址 https://…（RSS / Atom 均可）" maxlength="500" required><input name="alias" type="text" placeholder="显示名称（可选）" maxlength="40"><button class="primary" type="submit">添加订阅</button></form>
<div class="rss-feed-rows">${feedRowsHTML()}</div></section>`;
}
function feedsCardHTML() {
 return `<section class="side-card rss-feeds-card"><div class="section-heading"><h3>订阅源</h3><span>${rstate.feeds.length}</span></div>${rstate.feeds.map(f => `<button class="rss-source-row${rstate.filter === f.id ? ' active' : ''}" type="button" data-rss-filter="${esc(f.id)}"><strong>${esc(f.title)}</strong><span>${f.count} 条</span></button>`).join('') || '<p class="rss-side-note">暂无订阅源，稍后再来看看。</p>'}</section>`;
}

// ---- 页面：与首页同款双栏布局（feed-layout），切换标签不发生列宽跳变 ----
function renderRssPage() {
 beginView();
 if (!rstate.loaded) { loadRss().then(() => { if (store.page === PAGE_KEY) renderRssPage(); }).catch(e => { store.main.innerHTML = empty('订阅数据加载失败', e.message, 'search'); }); }
 document.title = 'RSS 订阅 · ' + store.baseTitle.split(' · ')[0];
 const admin = store.state.admin && !store.isStatic;
 const chips = [['', '全部', rstate.total], ...rstate.feeds.map(f => [f.id, f.title, f.count])]
  .map(([id, label, n]) => `<button type="button" class="rss-chip${rstate.filter === id ? ' active' : ''}" data-rss-filter="${esc(id)}">${esc(label)}${n ? ` <small>${n}</small>` : ''}</button>`).join('');
 store.main.innerHTML = `<div class="feed-layout"><section class="timeline">${topTabsHTML(ID)}<header class="rss-hero"><h1>RSS 订阅<span>。</span></h1><p>我订阅的站点更新，聚合在此；点击标题前往原文。</p><div class="rss-facts"><span><strong>${rstate.feeds.length}</strong> 个源</span><span><strong>${rstate.total}</strong> 条动态</span></div></header>${rstate.feeds.length ? `<div class="rss-filters" role="group" aria-label="按订阅源筛选">${chips}</div>` : ''}<div id="rss-list" class="rss-list">${rssListHTML() || (rstate.loaded ? empty('还没有订阅内容', '订阅源同步后会出现在这里。', 'clock') : empty('正在载入订阅…', '', 'clock'))}</div>${rssHasMore() ? '<div class="rss-more" id="rss-more">下滑自动加载</div>' : ''}</section><aside class="right-column${admin ? ' rss-aside' : ''}">${admin ? adminPanelHTML() : feedsCardHTML()}</aside></div>`;
 bindRssAdmin();
 bindRssFilters();
 bindRssMore();
 if (admin) {
  const events = new EventSource('/api/tasks/events');
  let previous = null;
  events.addEventListener('tasks', event => {
   const jobs = JSON.parse(event.data).jobs;
   const finished = jobs.filter(job => job.kind === 'plugin_sync' && ['succeeded','failed','cancelled'].includes(job.state)).map(job => job.id).join(',');
   if (previous !== null && previous !== finished) reloadRssPage();
   previous = finished;
  });
  onViewLeave(() => events.close());
 }
}

function bindRssFilters() {
 document.querySelectorAll('[data-rss-filter]').forEach(button => button.onclick = () => {
  rstate.filter = button.dataset.rssFilter;
  rstate.shown = 15;
  if (store.isStatic) { renderRssPage(); return; }
  rstate.posts = [];
  rstate.nextCursor = null;
  loadRss().then(() => { if (store.page === PAGE_KEY) renderRssPage(); }).catch(e => toast(e.message));
 });
}

function bindRssMore() {
 const sentinel = document.querySelector('#rss-more');
 if (!sentinel) return;
 let busy = false;
 const load = async () => {
  if (busy) return;
  busy = true;
  try {
   if (store.isStatic) { rstate.shown += 15; }
   else {
    if (!rstate.nextCursor) return;
    const data = await fetchRss({ ...(rstate.filter ? { feed: rstate.filter } : {}), cursor: rstate.nextCursor });
    const known = new Set(rstate.posts.map(p => p.id));
    rstate.posts.push(...data.posts.filter(p => !known.has(p.id)));
    rstate.nextCursor = data.next_cursor;
   }
   if (store.page !== PAGE_KEY) return;
   const list = document.querySelector('#rss-list');
   if (list) list.innerHTML = rssListHTML();
   sentinel.remove();
   if (rssHasMore()) { document.querySelector('#rss-list')?.insertAdjacentHTML('afterend', '<div class="rss-more" id="rss-more">下滑自动加载</div>'); bindRssMore(); }
  } catch (e) { toast(e.message); } finally { busy = false; }
 };
 const check = () => { if (!sentinel.isConnected || busy) return; if (sentinel.getBoundingClientRect().top < window.innerHeight + 300) load(); };
 const io = new IntersectionObserver(entries => { if (entries.some(en => en.isIntersecting)) load(); });
 window.addEventListener('scroll', check, { passive: true });
 onViewLeave(() => { io.disconnect(); window.removeEventListener('scroll', check); });
 io.observe(sentinel);
 check();
}

// ---- 管理操作绑定 ----
function bindRssAdmin() {
 const form = document.querySelector('#rss-add-form');
 if (form) form.onsubmit = async e => {
  e.preventDefault();
  const button = form.querySelector('button[type=submit]');
  button.disabled = true;
  try {
   const result = await api('plugin-admin', { id: ID, op: 'add', url: form.elements.url.value.trim(), alias: form.elements.alias.value.trim() });
   toast(`已订阅「${result.title}」，后台任务 #${result.task_id} 正在同步`);
   await reloadRssPage();
  } catch (error) { toast(error.message); } finally { button.disabled = false; }
 };
 document.querySelectorAll('[data-rss-sync-feed]').forEach(button => button.onclick = async () => {
  button.disabled = true;
  try { const r = await api('plugin-admin', { id: ID, op: 'sync', feed_id: button.dataset.rssSyncFeed }); toast(`已加入后台任务 #${r.task_id}`); await reloadRssPage(); }
  catch (error) { toast(error.message); } finally { button.disabled = false; }
 });
 document.querySelectorAll('[data-rss-remove-feed]').forEach(button => button.onclick = () => {
  const feedId = button.dataset.rssRemoveFeed;
  const feed = rstate.feeds.find(f => f.id === feedId);
  const dialog = rssDialog('rss-remove-dialog', `<h2>退订「${esc(feed?.title || feedId)}」？</h2><p>将移除该订阅源；已同步的动态可选择保留或一并删除。</p><label class="switch-line"><input type="checkbox" id="rss-remove-purge" ${feed?.count ? 'checked' : ''}> 同时删除它已同步的动态（不可恢复）</label><div class="dialog-actions"><button class="secondary" type="button" data-rss-cancel>取消</button><button class="danger" type="button" data-rss-confirm>确认退订</button></div>`);
  dialog.querySelector('[data-rss-cancel]').onclick = () => dialog.close();
  dialog.querySelector('[data-rss-confirm]').onclick = async () => {
   dialog.querySelector('[data-rss-confirm]').disabled = true;
   try {
    const r = await api('plugin-admin', { id: ID, op: 'remove', feed_id: feedId, purge: dialog.querySelector('#rss-remove-purge').checked ? 1 : 0 });
    toast(r.deleted ? `已退订，删除 ${r.deleted} 条动态` : '已退订，动态保留');
    dialog.close();
    await reloadRssPage();
   } catch (error) { toast(error.message); dialog.close(); } finally { dialog.querySelector('[data-rss-confirm]').disabled = false; }
  };
  dialog.showModal();
 });
}

// 自建确认弹窗（不与核心 #delete-dialog 的处理器冲突）
function rssDialog(id, innerHTML) {
 let dialog = document.querySelector('#' + id);
 if (!dialog) {
  dialog = document.createElement('dialog');
  dialog.id = id;
  document.body.appendChild(dialog);
  dialog.addEventListener('click', e => { if (e.target === dialog) dialog.close(); });
 }
 dialog.innerHTML = innerHTML;
 return dialog;
}

// ---- 本页动态交互（灯箱 / 评论 / 删除 / 置顶；点赞与分享走 app.js 全局委托，不依赖 store.posts）----
document.addEventListener('click', async e => {
 const media = e.target.closest('[data-rss-media]');
 if (media) {
  const post = rstate.posts.find(p => p.id === Number(media.dataset.rssMedia));
  if (post?.images?.length) { e.preventDefault(); openLightbox(postItems(post.images), Number(media.dataset.mediaIndex), media.querySelector('img')); }
  return;
 }
 const comments = e.target.closest('[data-rss-comments]');
 if (comments) {
  const slot = document.querySelector(`[data-comments-slot="${comments.dataset.rssComments}"]`);
  if (!slot) return;
  if (!slot.hidden) { slot.hidden = true; return; }
  slot.hidden = false;
  if (!slot.firstChild) {
   const post = rstate.posts.find(p => p.id === Number(comments.dataset.rssComments));
   if (post) mountInlineComments(post, slot);
  }
  slot.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  return;
 }
 const del = e.target.closest('[data-rss-delete]');
 if (del) {
  const id = Number(del.dataset.rssDelete);
  const dialog = rssDialog('rss-delete-dialog', '<h2>删除这条订阅动态？</h2><p>删除后该条目立墓碑，之后同步不会再出现。</p><div class="dialog-actions"><button class="secondary" type="button" data-rss-cancel>取消</button><button class="danger" type="button" data-rss-confirm>确认删除</button></div>');
  dialog.querySelector('[data-rss-cancel]').onclick = () => dialog.close();
  dialog.querySelector('[data-rss-confirm]').onclick = async () => {
   dialog.querySelector('[data-rss-confirm]').disabled = true;
   try { await api('delete-post', { id }); toast('动态已删除'); dialog.close(); await reloadRssPage(); }
   catch (error) { toast(error.message); dialog.close(); } finally { dialog.querySelector('[data-rss-confirm]').disabled = false; }
  };
  dialog.showModal();
  return;
 }
 const pin = e.target.closest('[data-rss-pin]');
 if (pin) {
  const id = Number(pin.dataset.rssPin);
  const post = rstate.posts.find(p => p.id === id);
  pin.disabled = true;
  try {
   await api('pin-post', { id, pinned: Number(post?.pinned) ? 0 : 1 });
   toast(Number(post?.pinned) ? '已取消置顶' : '已置顶');
   await reloadRssPage();
  } catch (error) { toast(error.message); pin.disabled = false; }
 }
});

// ---- 订阅卡片（首页时间线并入时由 postHTML 经 meta.card 分发渲染）----
// 头部身份（favicon + 站名 + @域名）由 postHTML 按 meta.avatar 渲染，卡片内不再重复源名
MKBLOG.registerCard('rss', post => {
 const m = post.meta || {};
 const e = MKBLOG.esc;
 const covers = (post.images || []).slice(0, 3);
 const href = /^https?:\/\//i.test(post.link || '') ? post.link : '';
 return `<div class="post-rss-card">${m.title && href ? `<p class="rss-line"><a class="rss-title" href="${e(href)}" target="_blank" rel="noopener">${e(m.title)}</a></p>` : (m.title ? `<p class="rss-line"><strong class="rss-title">${e(m.title)}</strong></p>` : '')}${m.summary ? `<p class="rss-summary">${e(m.summary)}</p>` : ''}${covers.length ? `<div class="rss-grid count${covers.length}">${covers.map((img, i) => `<button class="post-image" type="button" data-media-post="${post.id}" data-media-index="${i}" aria-label="查看图片"><img src="${e(img.thumb || img.url)}" alt="" loading="lazy"></button>`).join('')}</div>` : ''}</div>`;
});

MKBLOG.registerPage('/rss', { render: renderRssPage });
