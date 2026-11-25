// 壁纸收藏插件 · 前端（X 风格图库页 + 首页侧栏小组件 + 自动同步）
import { store } from '/assets/lib/store.js';
import { api, apiGet } from '/assets/lib/api.js';
import { icon, esc, toast, o, empty } from '/assets/lib/ui.js';
import { openLightbox } from '/assets/components/lightbox.js';
import { feed } from '/assets/views/feed.js';
import { beginView, onViewLeave } from '/assets/lib/lifecycle.js';
let stopGalleryMore = () => {};
let postsPending = false;

const ID = 'wallpapers';
const PAGE_KEY = 'plugin:wallpapers'; // app.js 路由器为插件页设定的 page 值
const wstate = { items: [], available: true, batch: 24, interval: 30, loaded: false, filter: 'all', query: '', shown: 24, galleryList: [] };
const SEARCH_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.8"/><path d="m16 16 5 5"/></svg>';
const isUltra = w => Math.max(w.width, w.height) >= 3840 && Math.min(w.width, w.height) >= 2160;
const batchSize = () => o('gallery_batch', store.pluginsMeta?.[ID]?.batch ?? wstate.batch) || 24;

// ---- 图片地址（动态走 /image 端点，静态用导出的原/缩略地址）----
export function imageUrl(w, thumb = false) {
 return store.isStatic ? (thumb ? w.thumbnail : w.original) : `/image?file=${encodeURIComponent(w.path)}${thumb ? '&thumb=1' : ''}&v=${w.mtime}`;
}
export function downloadUrl(w) { return store.isStatic ? w.original : imageUrl(w) + '&download=1'; }
function wallpaperItems(list) {
 return list.map(w => ({ src: imageUrl(w), width: w.width, height: w.height, downloadUrl: downloadUrl(w), downloadName: w.name, caption: `<strong>${esc(w.name.replace(/\.[^.]+$/, ''))}</strong><small>${w.width} × ${w.height} · ${(w.bytes / 1024 / 1024).toFixed(1)} MB</small>` }));
}

// ---- 数据加载（静态站从 content.json，动态站从 plugin-data 端点）----
export async function refreshWallpapers() {
 const data = store.isStatic
  ? { items: store.pluginsData?.[ID] || [], available: true, batch: store.pluginsMeta?.[ID]?.batch ?? wstate.batch, interval: 0 }
  : (await apiGet('plugin-data', { id: ID })).data;
 const changed = data.available !== wstate.available || JSON.stringify(data.items) !== JSON.stringify(wstate.items);
 if (changed && wstate.loaded && !store.isStatic) postsPending = true;
 wstate.items = data.items; wstate.available = data.available;
 if (data.batch) wstate.batch = data.batch;
 if (data.interval !== undefined) wstate.interval = data.interval;
 wstate.loaded = true;
 updateNavCount();
 if (changed) {
  renderMiniGallery();
  if (store.page === PAGE_KEY) updateGalleryData();
 }
 // 已加载历史或正在阅读时延后更新；请求返回时也检查，避免覆盖新分页或搜索结果。
 const canRefreshPosts = () => store.page === 'feed' && window.scrollY === 0 && store.feedShown <= 15 && store.posts.length <= 30 && !store.postsQuery && !document.querySelector('#feed-search')?.value && !document.querySelector('#compose-content')?.value;
 if (postsPending && canRefreshPosts()) {
  const previous = store.posts;
  const result = await apiGet('posts');
  if (canRefreshPosts() && store.posts === previous) {
   postsPending = false;
   const postsChanged = JSON.stringify(previous) !== JSON.stringify(result.posts);
   store.posts = result.posts; store.postsNext = result.next_cursor; store.postsTotal = result.total;
   if (postsChanged) feed();
  }
 }
 return changed;
}
function updateNavCount() { const nc = document.querySelector(`[data-plugin-count="${ID}"]`); if (nc) nc.textContent = wstate.items.length; }

// 数据刷新（自动同步/手动同步）时只重建网格与统计，不重建页面骨架：
// 整页 innerHTML 替换会让页面高度瞬间塌回首屏，浏览器随之把滚动钳制回顶部
function updateGalleryData() {
 if (store.page !== PAGE_KEY || !document.querySelector('.gallery-page')) return;
 renderGallery();
 const facts = document.querySelector('.gallery-facts');
 if (facts) {
  const ultra = wstate.items.filter(isUltra).length;
  facts.innerHTML = `<span><strong id="wall-total">${wstate.items.length}</strong> 张壁纸</span>${ultra ? `<span><strong>${ultra}</strong> 张 4K 超清</span>` : ''}`;
 }
}

// ---- 图库页（占满宽度的 X 式瀑布流：变高瓷砖 + 悬停删除，下载在图注行）----
function wallCardHTML(w, index) {
 return `<article class="media-tile" data-wall-id="${esc(w.id)}" data-wall-name="${esc(w.name)}"><button class="tile-preview" data-preview-index="${index}" type="button" aria-label="预览 ${esc(w.name)}"><img src="${esc(imageUrl(w, true))}" alt="${esc(w.name)}" width="${w.width}" height="${w.height}" loading="lazy">${isUltra(w) ? '<span class="resolution-badge">4K</span>' : ''}</button>${store.state.admin && !store.isStatic ? `<button class="wall-delete" type="button" data-wall-delete="${esc(w.id)}" aria-label="删除 ${esc(w.name)}">${icon('trash')}</button>` : ''}<div class="wall-caption"><div class="wall-caption-text"><strong>${esc(w.name.replace(/\.[^.]+$/, ''))}</strong><small>${w.width} × ${w.height}</small></div><a class="tile-download" href="${esc(downloadUrl(w))}" download aria-label="下载 ${esc(w.name)}">${icon('download')}</a></div></article>`;
}
function layoutMasonry() {
 const grid = document.querySelector('.masonry');
 if (!grid) return;
 grid.querySelectorAll('.media-tile').forEach(card => {
  card.style.gridRowEnd = `span ${Math.ceil(card.getBoundingClientRect().height + 16)}`;
 });
}
// 列宽变化（窗口缩放/断点）时整列重排；图片逐张 load 由网格捕获阶段监听补算
const masonryResize = new ResizeObserver(entries => {
 const width = entries[0].contentRect.width;
 if (width !== masonryResize.width) { masonryResize.width = width; layoutMasonry(); }
});
function filteredWallpapers() {
 return wstate.items.filter(w => (!wstate.query || w.name.toLowerCase().includes(wstate.query)) && (wstate.filter === 'all' || wstate.filter === 'landscape' && w.width >= w.height || wstate.filter === 'portrait' && w.height > w.width || wstate.filter === '4k' && isUltra(w)));
}
function renderGallery(append = false) {
 const list = filteredWallpapers();
 wstate.galleryList = list;
 const results = document.querySelector('#gallery-results');
 if (!results) return;
 const totalEl = document.querySelector('#wall-total');
 if (totalEl) totalEl.textContent = wstate.items.length;
 if (!list.length) { results.innerHTML = empty(wstate.available ? '没有找到匹配的壁纸' : '还没有连接到壁纸目录', wstate.available ? '换个关键词或筛选条件试试。' : '请检查本地壁纸目录是否存在并可读取。', 'image'); return; }
 let grid = results.querySelector('.masonry');
 if (!append || !grid) {
  results.innerHTML = '<div class="masonry"></div>';
  grid = results.querySelector('.masonry');
  masonryResize.disconnect();
  grid.addEventListener('load', layoutMasonry, true); // 捕获阶段接住 img load：加载完逐张补算行高
  masonryResize.observe(grid);
 }
 const start = grid.children.length;
 grid.insertAdjacentHTML('beforeend', list.slice(start, wstate.shown).map((w, i) => wallCardHTML(w, i)).join(''));
 layoutMasonry();
 results.querySelector('.gallery-more')?.remove();
 results.insertAdjacentHTML('beforeend', list.length > wstate.shown ? '<div class="gallery-more" id="gallery-more">下滑自动加载</div>' : `<div class="gallery-more">已展示全部 ${list.length} 张</div>`);
 bindGalleryMore();
}
function bindGalleryMore() {
 stopGalleryMore();
 const sentinel = document.querySelector('#gallery-more');
 if (!sentinel) return;
 let busy = false;
 const load = () => {
  if (busy) return;
  busy = true;
  io.disconnect();
  window.removeEventListener('scroll', onScroll, { passive: true });
  wstate.shown += batchSize();
  renderGallery(true); // 追加模式：只插入新瓷砖。整网格重建会让浏览器丢失滚动锚点，页面瞬间回顶
 };
 // IO + scroll 双保险：与首页加载更多同款，避免 IntersectionObserver 偶发不回调
 const check = () => { if (!sentinel.isConnected || busy) return; const r = sentinel.getBoundingClientRect(); if (r.top < window.innerHeight + 400) load(); };
 const onScroll = () => check();
 const io = new IntersectionObserver(entries => { if (entries.some(en => en.isIntersecting)) load(); });
 window.addEventListener('scroll', onScroll, { passive: true });
 stopGalleryMore = onViewLeave(() => { io.disconnect(); window.removeEventListener('scroll', onScroll); });
 io.observe(sentinel);
 check();
}
function renderGalleryPage() {
 beginView();
 onViewLeave(() => masonryResize.disconnect());
 if (!wstate.loaded) refreshWallpapers().then(() => { if (store.page === PAGE_KEY) renderGalleryPage(); }).catch(() => {});
 document.title = '壁纸收藏 · ' + store.baseTitle.split(' · ')[0];
 wstate.shown = batchSize();
 const total = wstate.items.length;
 const ultra = wstate.items.filter(isUltra).length;
 store.main.innerHTML = `<div class="gallery-page"><div class="gallery-hero"><h1>壁纸收藏<span>。</span></h1><p>${store.isStatic ? '精选风景已随站点打包，可在线预览、下载原图。' : '本地壁纸目录自动同步，可在线预览、下载原图。'}</p><div class="gallery-facts"><span><strong id="wall-total">${total}</strong> 张壁纸</span>${ultra ? `<span><strong>${ultra}</strong> 张 4K 超清</span>` : ''}</div></div><div class="gallery-toolbar"><div class="filter-pills"><button type="button" data-filter="all" class="active">全部</button><button type="button" data-filter="landscape">横屏</button><button type="button" data-filter="portrait">竖屏</button><button type="button" data-filter="4k">4K 超清</button></div><div class="gallery-tools"><label class="gallery-search">${SEARCH_SVG}<input id="gallery-search" type="text" placeholder="搜索壁纸" aria-label="搜索壁纸"></label>${!store.isStatic && store.state.admin ? `<div class="gallery-actions"><button class="text-link" id="refresh-gallery" type="button">${icon('refresh')}同步</button></div>` : ''}</div></div><div id="gallery-results"></div></div>`;
 document.querySelectorAll('[data-filter]').forEach(b => b.onclick = () => { wstate.filter = b.dataset.filter; wstate.shown = batchSize(); document.querySelectorAll('[data-filter]').forEach(x => x.classList.toggle('active', x === b)); renderGallery(); });
 document.querySelector('#gallery-search').oninput = e => { wstate.query = e.target.value.trim().toLowerCase(); wstate.shown = batchSize(); renderGallery(); };
 const refresh = document.querySelector('#refresh-gallery');
 if (refresh) {
  refresh.hidden = !store.state.admin;
  refresh.onclick = async () => { try { await api('sync-plugin', { id: ID }); await refreshWallpapers(); updateGalleryData(); toast('壁纸已同步'); } catch (e) { toast(e.message); } };
 }
 if (store.state.admin && !store.isStatic) {
  document.querySelector('.gallery-actions').insertAdjacentHTML('beforeend', `<button class="text-link" id="upload-wallpaper" type="button">${icon('plus')}上传</button><input id="wallpaper-file" type="file" accept="image/jpeg,image/png,image/webp,image/gif,image/avif" multiple hidden>`);
  const upload = document.querySelector('#upload-wallpaper');
  const picker = document.querySelector('#wallpaper-file');
  upload.onclick = () => picker.click();
  picker.onchange = async () => {
   const files = [...picker.files];
   if (!files.length) return;
   const maxMb = o('upload_max_mb', 15);
   if (files.some(file => file.size > maxMb * 1024 * 1024)) { toast(`单张图片不能超过 ${maxMb}MB`); picker.value = ''; return; }
   upload.disabled = true;
   upload.textContent = `正在上传 ${files.length} 张…`;
   try {
    const data = new FormData();
    data.set('id', ID);
    data.set('op', 'upload');
    data.set('count', files.length);
    files.forEach(file => data.append('images[]', file));
    const response = await fetch('/api?action=plugin-admin', { method: 'POST', headers: { 'X-CSRF-Token': store.state.csrf }, body: data });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || '壁纸上传失败');
    toast(`已上传 ${result.uploaded.length} 张壁纸`);
    await refreshWallpapers();
    store.posts = (await api('posts')).posts;
   } catch (e) { toast(e.message); }
   finally { upload.disabled = false; upload.innerHTML = `${icon('plus')}上传`; picker.value = ''; }
  };
 }
 renderGallery();
}

// ---- 首页侧栏小组件 ----
function miniGalleryHTML() {
 const newest = wstate.items.slice(0, 4);
 if (!wstate.available || !newest.length) return '';
 return `<div class="mini-gallery"><div class="section-heading"><h3>壁纸收藏</h3><span>${wstate.items.length}</span></div><p>精选风景，随时取用。</p><div class="mini-grid">${newest.map((w, i) => `<a href="/wallpapers/" data-mini="${i}" aria-label="预览 ${esc(w.name)}"><img src="${esc(imageUrl(w, true))}" alt="${esc(w.name)}" loading="lazy"></a>`).join('')}</div><a class="text-link" href="/wallpapers/">查看全部壁纸 ${icon('arrow')}</a></div>`;
}
function renderMiniGallery() {
 const mount = document.querySelector(`[data-plugin-widget="${ID}"]`);
 if (mount) mount.innerHTML = miniGalleryHTML();
}

// ---- 注册页面与小组件 ----
MKBLOG.registerPage('/wallpapers', { render: renderGalleryPage });
MKBLOG.registerWidget(ID, miniGalleryHTML);

// ---- 管理删除（仅管理员）：确认后删源文件并从列表移除 ----
document.addEventListener('click', async e => {
 const btn = e.target.closest('[data-wall-delete]');
 if (!btn) return;
 e.preventDefault();
 const card = btn.closest('[data-wall-id]');
 const name = card.dataset.wallName;
 // 自建确认弹窗（不与核心 #delete-dialog 的处理器冲突）
 let dialog = document.querySelector('#wall-delete-dialog');
 if (!dialog) {
  dialog = document.createElement('dialog');
  dialog.id = 'wall-delete-dialog';
  dialog.innerHTML = `<h2>删除这张壁纸？</h2><p class="wall-delete-p"></p><div class="dialog-actions"><button class="secondary" data-wall-cancel type="button">取消</button><button class="danger" data-wall-confirm type="button">确认删除</button></div>`;
  document.body.appendChild(dialog);
  dialog.addEventListener('click', e => { if (e.target === dialog) dialog.close(); });
 }
 dialog.querySelector('.wall-delete-p').textContent = `将删除源文件「${name}」及动态中的这张图片；同批动态的其他图片会保留。删除后不可恢复。`;
 dialog.showModal();
 dialog.querySelector('[data-wall-cancel]').onclick = () => dialog.close();
 dialog.querySelector('[data-wall-confirm]').onclick = async () => {
  dialog.querySelector('[data-wall-confirm]').disabled = true;
  try {
   const result = await api('plugin-admin', { id: ID, op: 'delete', wall_id: card.dataset.wallId });
   const ref = 'wall:' + card.dataset.wallId;
   store.posts = store.posts.filter(post => !result.deleted_posts.includes(post.id));
   for (const post of store.posts) post.images = post.images.filter(image => image.ref !== ref);
   store.postsTotal = Math.max(0, store.postsTotal - result.deleted_posts.length);
   dialog.close();
   wstate.items = wstate.items.filter(w => w.id !== card.dataset.wallId);
   wstate.shown = Math.max(1, wstate.shown - 1);
   const scrollY = window.scrollY; // 重建网格：预览索引与列表同步重排，避免删除后取旧下标越界；同时护住滚动位置
   renderGallery();
   window.scrollTo(0, scrollY);
   updateNavCount();
   toast('壁纸已删除');
  } catch (err) { dialog.close(); toast(err.message); } finally { dialog.querySelector('[data-wall-confirm]').disabled = false; }
 };
});

// ---- 灯箱（大图预览 + 侧栏小图）----
document.addEventListener('click', e => {
 const wallPreview = e.target.closest('[data-preview-index]');
 if (wallPreview) { e.preventDefault(); openLightbox(wallpaperItems(wstate.galleryList), Number(wallPreview.dataset.previewIndex), wallPreview.querySelector('img')); return; }
 const miniPreview = e.target.closest('[data-mini]');
 if (miniPreview) { e.preventDefault(); openLightbox(wallpaperItems(wstate.items.slice(0, 4)), Number(miniPreview.dataset.mini), miniPreview.querySelector('img')); }
});

// ---- 启动：等核心数据就绪后加载；定时器在首次加载拿到真实 interval 后再建 ----
function start() {
 refreshWallpapers().catch(() => {}).finally(() => {
  if (!store.isStatic && wstate.interval > 0) {
   setInterval(() => { if (!document.hidden) refreshWallpapers().catch(() => {}); }, wstate.interval * 1000);
  }
 });
}
if (store.state) start(); else document.addEventListener('mkblog:data', start, { once: true });
