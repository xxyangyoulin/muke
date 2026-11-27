import { giscusComments, giscusReactions } from '../components/giscus.js';
import { store } from '../lib/store.js';
import { icon, esc, dateLabel, empty, profileName, avatar, postHTML, verifiedBadge } from '../lib/ui.js';
import { composer, bindComposer } from '../components/composer.js';
import { articleExcerpt, articleText, readingMinutes, tagChips, articleCover, moreArticles } from './articles.js';
import { apiGet, morePosts } from '../lib/api.js';
import { beginView, onViewLeave } from '../lib/lifecycle.js';
import { toast } from '../lib/ui.js';
let stopMore = () => {};
let searchType = 'all';

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
 const posts = searchType === 'article' ? [] : store.posts;
 const articles = searchType === 'post' ? [] : store.articles;
 const boundary = [store.postsNext ? posts.at(-1) : null, store.articlesNext ? articles.at(-1) : null]
  .filter(Boolean).map(item => ({time: Number(item.pinned) ? Infinity : new Date(item.created_at).getTime(), id: item.id}))
  .sort((x, y) => y.time - x.time || y.id - x.id)[0];
 return [
  ...posts.map(p => ({type: 'post', time: new Date(p.created_at).getTime(), id: p.id, data: p})),
  ...articles.map(a => ({type: 'article', time: new Date(a.created_at).getTime(), id: a.id, data: a}))
 ].filter(item => {
  if (store.isStatic && store.postsQuery) {
   const d = item.data;
   const text = item.type === 'article' ? `${d.title} ${d.tags || ''} ${articleText(d)}` : d.content || '';
   if (!text.toLowerCase().includes(store.postsQuery.toLowerCase())) return false;
  }
  return Number(item.data.pinned) || !boundary || item.time > boundary.time || (item.time === boundary.time && item.id >= boundary.id);
 }).sort((x, y) => Number(y.data.pinned || 0) - Number(x.data.pinned || 0) || y.time - x.time || y.id - x.id);
}
export function feedHTML() {
 const items = feedItems();
 const hasMore = items.length > store.feedShown || (searchType !== 'article' && !!store.postsNext) || (searchType !== 'post' && !!store.articlesNext);
 return (items.length ? items.slice(0, store.feedShown).map(i => i.type === 'article' ? articleTeaserHTML(i.data) : postHTML(i.data)).join('') : '') + (hasMore ? '<div id="feed-more"></div>' : '');
}

export function bindFeedMore() {
 stopMore();
 highlightSearch();
 const sentinel = document.querySelector('#feed-more');
 if (!sentinel) return;
 let busy = false;
 let active = true;
 const loadMore = async () => {
  if (busy) return;
  busy = true;
  window.removeEventListener('scroll', onScroll, { passive: true });
  const list = document.querySelector('#post-list');
  try { if (feedItems().length <= store.feedShown + 15) await Promise.all([searchType !== 'article' && morePosts(), searchType !== 'post' && moreArticles()]); }
  catch (e) { if (!active) return; toast(e.message); busy = false; if (sentinel.isConnected) window.addEventListener('scroll', onScroll, { passive: true }); return; }
  if (!active || !list?.isConnected) return;
  store.feedShown += 15;
  if (list) list.innerHTML = feedHTML() || '';
  requestAnimationFrame(() => { if (active) bindFeedMore(); });
 };
 // 双保险：IO 偶发不回调（快速滚动/双渲染时序）时，scroll 兜底检查 sentinel 位置
 const check = () => { if (!sentinel.isConnected || busy) return; const r = sentinel.getBoundingClientRect(); if (r.top < window.innerHeight + 300) { io.disconnect(); loadMore(); } };
 const onScroll = () => check();
 const io = new IntersectionObserver(entries => { if (entries.some(en => en.isIntersecting)) { io.disconnect(); window.removeEventListener('scroll', onScroll, { passive: true }); loadMore(); } });
 window.addEventListener('scroll', onScroll, { passive: true });
 stopMore = onViewLeave(() => { active = false; io.disconnect(); window.removeEventListener('scroll', onScroll); });
 io.observe(sentinel);
 check();
}

function highlightSearch() {
 const list = document.querySelector('#post-list');
 if (!list || !store.postsQuery) return;
 const pattern = new RegExp(store.postsQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi');
 const walker = document.createTreeWalker(list, NodeFilter.SHOW_TEXT, {
  acceptNode: node => node.parentElement.closest('.post-body') && !node.parentElement.closest('mark, script, style, button, .post-meta, .post-actions, .post-comments-slot, .pinned-label') ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT
 });
 const nodes = [];
 while (walker.nextNode()) nodes.push(walker.currentNode);
 for (const node of nodes) {
  const matches = [...node.textContent.matchAll(pattern)];
  if (!matches.length) continue;
  const fragment = document.createDocumentFragment();
  let offset = 0;
  for (const match of matches) {
   fragment.append(node.textContent.slice(offset, match.index));
   const mark = document.createElement('mark');
   mark.className = 'search-highlight';
   mark.textContent = match[0];
   fragment.append(mark);
   offset = match.index + match[0].length;
  }
  fragment.append(node.textContent.slice(offset));
  node.replaceWith(fragment);
 }
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

// 动态站查询服务端全文，静态站过滤导出的完整数据；清空后恢复原时间线。
export function bindFeedSearch() {
 const input = document.querySelector('#feed-search');
 if (!input) return;
 let request;
 let timer;
 let articles = store.articles;
 let posts = store.posts;
 let next = store.postsNext;
 let articlesNext = store.articlesNext;
 let searching = false;
 onViewLeave(() => {
  request?.abort(); clearTimeout(timer);
  if (searching) { store.posts = posts; store.postsNext = next; store.articles = articles; store.articlesNext = articlesNext; }
  store.postsQuery = ''; searchType = 'all';
 });
 const search = () => {
  if (!searching) { articles = store.articles; posts = store.posts; next = store.postsNext; articlesNext = store.articlesNext; }
  stopMore();
  request?.abort(); clearTimeout(timer);
  const q = input.value.trim();
  const list = document.querySelector('#post-list');
  if (!list) return;
  const render = () => {
   store.feedShown = 15;
   list.innerHTML = feedHTML() || empty('没有找到匹配的内容', '换个关键词或筛选条件试试。', 'search');
   bindFeedMore();
  };
  if (!q || store.isStatic) {
   store.posts = posts; store.postsNext = next; store.articles = articles; store.articlesNext = articlesNext;
   store.postsQuery = q; searching = !!q;
   render();
   return;
  }
  request = new AbortController();
  const signal = request.signal;
  timer = setTimeout(async () => {
   try {
    const [result, matches] = await Promise.all([apiGet('posts', {q}, {signal}), apiGet('articles', {q}, {signal})]);
    if (signal.aborted || !list.isConnected) return;
    store.posts = result.posts; store.postsNext = result.next_cursor; store.postsQuery = q;
    store.articles = matches.articles; store.articlesNext = matches.next_cursor; searching = true;
    render();
   } catch (e) { if (e.name !== 'AbortError') toast(e.message); }
  }, 250);
 };
 input.addEventListener('input', search);
 document.querySelectorAll('[data-search-type]').forEach(button => {
  button.addEventListener('click', () => {
   searchType = button.dataset.searchType;
   document.querySelectorAll('[data-search-type]').forEach(item => item.setAttribute('aria-pressed', String(item === button)));
   search();
  });
 });
}

const SEARCH_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.8"/><path d="m16 16 5 5"/></svg>';

// 顶部标签：日常动态 / 文章随笔 / 插件声明 page.tab 的页面（如 RSS 订阅，排最后）
export function topTabsHTML(active = 'feed') {
 const tabs = [['feed', '/', '日常动态']];
 tabs.push(['articles', '/articles/', '文章随笔']);
 for (const [id, p] of Object.entries(store.state?.plugins || {})) if (p.page?.tab) tabs.push([id, p.page.route + '/', p.page.title]);
 return `<div class="top-tabs">${tabs.map(([key, href, label]) => `<a class="tab${key === active ? ' selected' : ''}" href="${esc(href)}">${esc(label)}</a>`).join('')}</div>`;
}

// 首页：X 式布局 —— 顶栏 tabs / 资料头 / 发布框 / 扁平时间线 + 右栏（搜索 · 订阅 · 最新文章 · 标签）
export function feed() {
 beginView();
 store.feedShown = 15;
 document.title = store.baseTitle;
 const cover = store.state.profile.cover;
 const badge = esc((store.state.options && store.state.options.cover_badge) || '');
 const coverHTML = `<div class="profile-cover">${cover ? `<img class="cover-img" src="${esc(cover)}" alt="">` : ''}${badge ? `<small>${badge}</small>` : ''}</div>`;
 store.main.innerHTML = `<div class="feed-layout"><section class="timeline">${topTabsHTML('feed')}${coverHTML}<div class="profile-details"><div class="profile-head-row">${avatar()}<div class="profile-name-col"><h1>${profileName()}${verifiedBadge}<small class="handle">@${esc(store.state.profile.handle)}</small></h1><div class="profile-facts"><span><strong>${store.isStatic ? store.posts.length : store.postsTotal}</strong> 动态</span>${store.articles.length ? `<span><strong>${store.articlesTotal}</strong> 文章</span>` : ''}</div></div></div><p class="profile-bio">${esc(store.state.profile.bio)}</p></div>${store.state.admin ? composer() : ''}<div id="post-list">${feedHTML() || empty('故事，从这一刻开始', store.isStatic ? '这里即将装满日常与灵感，欢迎下次来看看。' : '进入管理后台，写下你的第一条动态。')}</div></section><aside class="right-column"><div class="side-search"><div class="search-wrap">${SEARCH_SVG}<input id="feed-search" type="text" placeholder="搜索动态和文章" aria-label="搜索动态和文章" maxlength="200"></div><div class="search-types" role="group" aria-label="搜索内容类型">${[['all', '全部'], ['post', '动态'], ['article', '文章']].map(([type, label]) => `<button type="button" data-search-type="${type}" aria-pressed="${type === searchType}">${label}</button>`).join('')}</div></div><section class="side-card about-card"><h2>订阅</h2><p>通过 RSS 跟踪最新动态与文章，用你喜欢的阅读器即可。</p><div class="about-actions"><a class="pill" href="/rss.xml">订阅 RSS ${icon('arrow')}</a></div></section><div class="mini-articles side-card"><div class="section-heading"><h3>最新文章</h3></div>${store.articles.slice(0, 3).map(a => `<a class="mini-article" href="/article/${a.id}/"><strong>${esc(a.title)}</strong><span>${esc(dateLabel(a.created_at))} · ${readingMinutes(a)} 分钟</span></a>`).join('') || '<p class="small-label">第一篇长文，正在酝酿。</p>'}<a class="text-link" href="/articles/">全部文章 ${icon('arrow')}</a></div><div id="feed-tags-panel"></div><div id="plugin-widgets"></div><footer class="site-footer">由 <a href="https://github.com/xxyangyoulin/muke" target="_blank" rel="noopener">MUKE</a> 强力驱动</footer></aside></div>`;
 mountPluginWidgets();
 bindComposer();
 bindFeedTags();
 bindFeedMore();
 bindFeedSearch();
}

export function articleTeaserHTML(a) {
 const cover = articleCover(a);
 return `<article class="post" id="article-${a.id}">${avatar()}<div class="post-body"><div class="post-meta"><strong>${profileName()}</strong>${verifiedBadge}<span>@${esc(store.state.profile.handle)} · ${esc(dateLabel(a.created_at))}</span>${store.state.admin ? `<div class="post-admin"><button class="icon-button" data-edit-article="${a.id}" aria-label="编辑文章">${icon('edit')}</button><button class="icon-button" data-delete-article="${a.id}" aria-label="删除文章">${icon('trash')}</button></div>` : ''}</div><a class="article-teaser" href="/article/${a.id}/"><div class="article-teaser-text"><span class="article-tag">${icon('edit')} 文章</span><h3>${esc(a.title)}</h3><p>${esc(articleExcerpt(a))}</p>${a.tags ? tagChips(a, false) : ''}${store.state.options?.comment_provider === 'giscus' ? `<span class="teaser-comments"><span data-giscus-comments="article:${a.id}">${giscusComments.get(`article:${a.id}`) ?? a.giscus_counts?.comments ?? '—'} 评论</span> · ${giscusReactions.get(`article:${a.id}`)?.count ?? a.giscus_counts?.likes ?? '—'} 点赞</span>` : (a.comment_count ? `<span class="teaser-comments">${icon('chat')} ${a.comment_count} 评论</span>` : '')}</div>${cover ? `<img class="article-teaser-cover" src="${esc(cover)}" alt="" loading="lazy">` : ''}</a><div class="post-actions"><button data-share-article="${a.id}">${icon('arrow')}<span>分享</span></button></div></div></article>`;
}
