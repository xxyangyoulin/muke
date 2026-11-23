import { store } from '../lib/store.js';
import { icon, esc, dateLabel, empty, profileName, avatar, postHTML, verifiedBadge } from '../lib/ui.js';
import { composer, bindComposer } from '../components/composer.js';
import { articleExcerpt, articleText, readingMinutes, tagChips, articleCover, moreArticles } from './articles.js';
import { apiGet, morePosts } from '../lib/api.js';
import { beginView, onViewLeave } from '../lib/lifecycle.js';
import { toast } from '../lib/ui.js';
let stopMore = () => {};

// 插件侧栏小组件：各 plugin.js 经 MKBLOG.registerWidget 注册，feed 渲染后统一挂载
export function mountPluginWidgets() {
 const host = document.querySelector('#plugin-widgets');
 if (!host) return;
 host.innerHTML = '';
 for (const [id, fn] of Object.entries(window.MKBLOG._widgets || {})) {
  const div = document.createElement('div');
  div.dataset.pluginWidget = id;
  try { div.innerHTML = fn() || ''; } catch { div.innerHTML = ''; }
  host.appendChild(div);
 }
}

export function feedItems() {
 const boundary = [store.postsNext ? store.posts.at(-1) : null, store.articlesNext ? store.articles.at(-1) : null]
  .filter(Boolean).map(item => ({time: new Date(item.created_at).getTime(), id: item.id}))
  .sort((x, y) => y.time - x.time || y.id - x.id)[0];
 return [
  ...store.posts.map(p => ({type: 'post', time: new Date(p.created_at).getTime(), id: p.id, data: p})),
  ...store.articles.map(a => ({type: 'article', time: new Date(a.created_at).getTime(), id: a.id, data: a}))
 ].filter(item => !boundary || item.time > boundary.time || (item.time === boundary.time && item.id >= boundary.id)).sort((x, y) => y.time - x.time || y.id - x.id);
}
export function feedHTML() {
 const items = feedItems();
 const hasMore = items.length > store.feedShown || !!store.postsNext || !!store.articlesNext;
 return (items.length ? items.slice(0, store.feedShown).map(i => i.type === 'article' ? articleTeaserHTML(i.data) : postHTML(i.data)).join('') : '') + (hasMore ? '<div id="feed-more"></div>' : '');
}

export function bindFeedMore() {
 stopMore();
 const sentinel = document.querySelector('#feed-more');
 if (!sentinel) return;
 let busy = false;
 const loadMore = async () => {
  if (busy) return;
  busy = true;
  window.removeEventListener('scroll', onScroll, { passive: true });
  const list = document.querySelector('#post-list');
  try { if (feedItems().length <= store.feedShown + 15) await Promise.all([morePosts(), moreArticles()]); }
  catch (e) { toast(e.message); busy = false; if (sentinel.isConnected) window.addEventListener('scroll', onScroll, { passive: true }); return; }
  if (!list?.isConnected) return;
  store.feedShown += 15;
  if (list) list.innerHTML = feedHTML() || '';
  requestAnimationFrame(() => bindFeedMore());
 };
 // 双保险：IO 偶发不回调（快速滚动/双渲染时序）时，scroll 兜底检查 sentinel 位置
 const check = () => { if (!sentinel.isConnected || busy) return; const r = sentinel.getBoundingClientRect(); if (r.top < window.innerHeight + 300) { io.disconnect(); loadMore(); } };
 const onScroll = () => check();
 const io = new IntersectionObserver(entries => { if (entries.some(en => en.isIntersecting)) { io.disconnect(); window.removeEventListener('scroll', onScroll, { passive: true }); loadMore(); } });
 window.addEventListener('scroll', onScroll, { passive: true });
 stopMore = onViewLeave(() => { io.disconnect(); window.removeEventListener('scroll', onScroll); });
 io.observe(sentinel);
 check();
}

export function bindFeedTags() {
 const host = document.querySelector('#feed-tags-panel');
 if (!host) return;
 const counts = store.articleTags;
 const cloud = Object.entries(counts).sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
 if (!cloud.length) { host.innerHTML = ''; return; }
 host.innerHTML = `<div class="tags-panel feed-tags"><div class="section-heading"><h3>标签</h3><span>${cloud.length}</span></div><div class="tags-cloud collapsible" id="feed-tags">${cloud.map(([t, n]) => `<a class="tag-chip" href="/articles/?tag=${encodeURIComponent(t)}">#${esc(t)}<small>${n}</small></a>`).join('')}</div><button class="text-link" id="feed-tags-toggle" hidden type="button"></button></div>`;
 const cloudEl = host.querySelector('#feed-tags');
 const toggle = host.querySelector('#feed-tags-toggle');
 if (cloudEl.scrollHeight > cloudEl.clientHeight + 4) {
  const total = cloudEl.querySelectorAll('.tag-chip').length;
  const paint = expanded => { toggle.innerHTML = (expanded ? '收起' : `展开全部 ${total} 个`) + ' ' + icon(expanded ? 'close' : 'arrow'); };
  toggle.hidden = false;
  paint(false);
  toggle.onclick = () => { paint(cloudEl.classList.toggle('expanded')); };
 }
}

// 右栏搜索：全量过滤 —— 输入时从全部动态/文章重建匹配结果（未加载的历史也能搜到），清空恢复原时间线
export function bindFeedSearch() {
 const input = document.querySelector('#feed-search');
 if (!input) return;
 let request;
 let timer;
 let articles = store.articles;
 let posts = store.posts;
 let next = store.postsNext;
 let articlesNext = store.articlesNext;
 onViewLeave(() => {
  request?.abort(); clearTimeout(timer);
  if (store.postsQuery) { store.posts = posts; store.postsNext = next; store.articles = articles; store.articlesNext = articlesNext; store.postsQuery = ''; }
 });
 input.addEventListener('input', () => {
  if (!store.postsQuery) { articles = store.articles; posts = store.posts; next = store.postsNext; articlesNext = store.articlesNext; }
  stopMore();
  request?.abort(); clearTimeout(timer);
  const q = input.value.trim().toLowerCase();
  const list = document.querySelector('#post-list');
  if (!list) return;
  if (!store.isStatic) {
   request = new AbortController();
   const signal = request.signal;
   timer = setTimeout(async () => {
    try {
     const [result, matches] = await Promise.all([apiGet('posts', {q}, {signal}), q ? apiGet('articles', {q}, {signal}) : Promise.resolve({articles, next_cursor: articlesNext})]);
     if (signal.aborted || !list.isConnected) return;
     store.posts = result.posts; store.postsNext = result.next_cursor; store.postsQuery = q;
     store.articles = matches.articles; store.articlesNext = matches.next_cursor; store.feedShown = 15;
     list.innerHTML = feedHTML() || empty('没有找到匹配的内容', '换个关键词试试。');
     bindFeedMore();
    } catch (e) { if (e.name !== 'AbortError') toast(e.message); }
   }, 250);
   return;
  }
  if (!q) {
   list.innerHTML = feedHTML() || empty('故事，从这一刻开始', store.isStatic ? '这里即将装满日常与灵感，欢迎下次来看看。' : '进入管理后台，写下你的第一条动态。');
   bindFeedMore();
   return;
  }
  const matches = feedItems().filter(i => {
   const d = i.data;
   const hay = i.type === 'article' ? `${d.title} ${d.tags || ''} ${articleText(d)}` : d.content || '';
   return hay.toLowerCase().includes(q);
  });
  list.innerHTML = matches.length ? matches.map(i => i.type === 'article' ? articleTeaserHTML(i.data) : postHTML(i.data)).join('') : empty('没有找到匹配的内容', '换个关键词试试。', 'search');
 });
}

const SEARCH_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.8"/><path d="m16 16 5 5"/></svg>';

// 首页：X 式布局 —— 顶栏 tabs / 资料头 / 发布框 / 扁平时间线 + 右栏（搜索 · 订阅 · 最新文章 · 标签）
export function feed() {
 beginView();
 store.feedShown = 15;
 document.title = store.baseTitle;
 const cover = store.state.profile.cover;
 const badge = esc((store.state.options && store.state.options.cover_badge) || '');
 const coverHTML = `<div class="profile-cover">${cover ? `<img class="cover-img" src="${esc(cover)}" alt="">` : ''}${badge ? `<small>${badge}</small>` : ''}</div>`;
 store.main.innerHTML = `<div class="feed-layout"><section class="timeline"><div class="top-tabs"><button class="tab selected" type="button">日常动态</button><a class="tab" href="/articles/">文章随笔</a></div>${coverHTML}<div class="profile-details"><div class="profile-head-row">${avatar()}<div class="profile-name-col"><h1>${profileName()}${verifiedBadge}<small class="handle">@${esc(store.state.profile.handle)}</small></h1><div class="profile-facts"><span><strong>${store.isStatic ? store.posts.length : store.postsTotal}</strong> 动态</span>${store.articles.length ? `<span><strong>${store.articlesTotal}</strong> 文章</span>` : ''}</div></div></div><p class="profile-bio">${esc(store.state.profile.bio)}</p></div>${store.state.admin ? composer() : ''}<div id="post-list">${feedHTML() || empty('故事，从这一刻开始', store.isStatic ? '这里即将装满日常与灵感，欢迎下次来看看。' : '进入管理后台，写下你的第一条动态。')}</div></section><aside class="right-column"><div class="side-search"><div class="search-wrap">${SEARCH_SVG}<input id="feed-search" type="text" placeholder="搜索动态" aria-label="搜索动态"></div></div><section class="side-card about-card"><h2>订阅</h2><p>通过 RSS 跟踪最新动态与文章，用你喜欢的阅读器即可。</p><div class="about-actions"><a class="pill" href="/rss.xml">订阅 RSS ${icon('arrow')}</a></div></section><div class="mini-articles side-card"><div class="section-heading"><h3>最新文章</h3></div>${store.articles.slice(0, 3).map(a => `<a class="mini-article" href="/article/${a.id}/"><strong>${esc(a.title)}</strong><span>${esc(dateLabel(a.created_at))} · ${readingMinutes(a)} 分钟</span></a>`).join('') || '<p class="small-label">第一篇长文，正在酝酿。</p>'}<a class="text-link" href="/articles/">全部文章 ${icon('arrow')}</a></div><div id="feed-tags-panel"></div><div id="plugin-widgets"></div><footer class="site-footer">由 <a href="https://github.com/xxyangyoulin/muke" target="_blank" rel="noopener">MUKE</a> 强力驱动</footer></aside></div>`;
 mountPluginWidgets();
 bindComposer();
 bindFeedTags();
 bindFeedMore();
 bindFeedSearch();
}

export function articleTeaserHTML(a) {
 const cover = articleCover(a);
 return `<article class="post" id="article-${a.id}">${avatar()}<div class="post-body"><div class="post-meta"><strong>${profileName()}</strong>${verifiedBadge}<span>@${esc(store.state.profile.handle)} · ${esc(dateLabel(a.created_at))}</span>${store.state.admin ? `<div class="post-admin"><button class="icon-button" data-edit-article="${a.id}" aria-label="编辑文章">${icon('edit')}</button><button class="icon-button" data-delete-article="${a.id}" aria-label="删除文章">${icon('trash')}</button></div>` : ''}</div><a class="article-teaser" href="/article/${a.id}/"><div class="article-teaser-text"><span class="article-tag">${icon('edit')} 文章</span><h3>${esc(a.title)}</h3><p>${esc(articleExcerpt(a))}</p>${a.tags ? tagChips(a, false) : ''}${a.comment_count ? `<span class="teaser-comments">${icon('chat')} ${a.comment_count} 评论</span>` : ''}</div>${cover ? `<img class="article-teaser-cover" src="${esc(cover)}" alt="" loading="lazy">` : ''}</a><div class="post-actions"><button data-share-article="${a.id}">${icon('arrow')}<span>分享</span></button></div></div></article>`;
}
