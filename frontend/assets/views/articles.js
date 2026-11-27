import { store } from '../lib/store.js';
import { api, apiGet, embeddedJson } from '../lib/api.js';
import { icon, esc, o, dateLabel, empty, avatar, profileName, verifiedBadge, toast } from '../lib/ui.js';
import { feed, articleTeaserHTML, topTabsHTML } from './feed.js';
import { admin } from './admin.js';
import { mountComments } from '../components/comments.js';
import { beginView, currentView } from '../lib/lifecycle.js';
import { pushHistory } from '../lib/navigation.js';

// 文章相关页面共用顶栏 tabs（与首页同款，文章侧选中；含插件声明的 tab 页）
const TABS = () => topTabsHTML('articles');

export function articleText(a) {
 if (a._text === undefined) {
  const div = document.createElement('div');
  div.innerHTML = a.html || '';
  a._text = a.excerpt ?? (div.textContent || '').replace(/\s+/g, ' ').trim();
 }
 return a._text;
}

export function articleExcerpt(a) { const t = articleText(a); return a.excerpt ?? t.slice(0, 110) + (t.length > 110 ? '…' : ''); }

export function readingMinutes(a) { return Math.max(1, Math.ceil((a.text_length ?? articleText(a).length) / (o('reading_chars_per_minute', 500) || 500))); }

export function articleMetaHTML(a) {
 return `<div class="article-meta"><span>${esc(dateLabel(a.created_at))}</span><span>${readingMinutes(a)} 分钟阅读</span>${a.updated_at !== a.created_at ? '<span>已编辑</span>' : ''}</div>`;
}

export function tagChips(a, clickable = true) {
 const tags = (a.tags || '').split(',').filter(Boolean);
 if (!tags.length) return '';
 const chip = t => clickable
  ? `<a class="tag-chip" href="/articles/?tag=${encodeURIComponent(t)}">#${esc(t)}</a>`
  : `<span class="tag-chip">#${esc(t)}</span>`;
 return `<span class="article-tags">${tags.map(chip).join('')}</span>`;
}

export async function loadArticles(force = false) {
 if (store.articlesLoaded && !force) return store.articles;
 // 静态页优先用烘焙时内联的数据块，缺失（旧缓存页面）再回退到远程 JSON
 const data = store.isStatic ? (embeddedJson('embedded-articles') ?? await (await fetch('/assets/articles.json').then(r => { if (!r.ok) throw new Error('文章数据加载失败'); return r.json(); }))) : await api('articles');
 store.articles = data.articles || [];
 store.articlesNext = data.next_cursor ?? null;
 store.articlesTotal = data.total ?? store.articles.length;
 store.articleTags = data.tags || {};
 if (store.isStatic) store.articles.forEach(a => (a.tags || '').split(',').filter(Boolean).forEach(t => store.articleTags[t] = (store.articleTags[t] || 0) + 1));
 const drafts = !store.isStatic && store.state.admin ? await apiGet('articles', {draft: 1}) : {articles: [], total: 0, next_cursor: null};
 store.articlesDrafts = drafts.articles;
 store.draftsNext = drafts.next_cursor;
 store.draftsTotal = drafts.total;
 store.articlesLoaded = true;
 return store.articles;
}

export async function moreArticles(draft = false) {
 const key = draft ? 'draftsNext' : 'articlesNext';
 const list = draft ? 'articlesDrafts' : 'articles';
 const cursor = store[key];
 if (!cursor) return;
 const previous = store[list];
 const result = await apiGet('articles', {cursor, draft: draft ? 1 : 0, q: draft ? '' : store.postsQuery});
 if (store[key] !== cursor || store[list] !== previous) return;
 const known = new Set(previous.map(a => a.id));
 previous.push(...result.articles.filter(a => !known.has(a.id)));
 store[key] = result.next_cursor;
}

export async function articlesPage() {
 const view = beginView();
 document.title = `文章随笔 · ${store.state.profile.name}`;
 let list;
 try { list = await loadArticles(); } catch (e) { if (currentView(view)) store.main.innerHTML = empty('暂时没能加载文章', esc(e.message)); return; }
 if (!currentView(view)) return;
 const tag = new URLSearchParams(location.search).get('tag') || '';
 let next = store.articlesNext;
 if (tag && !store.isStatic) {
  try {
   const result = await apiGet('articles', {tag});
   if (!currentView(view)) return;
   list = result.articles; next = result.next_cursor;
  } catch (e) { if (currentView(view)) store.main.innerHTML = empty('暂时没能加载文章', esc(e.message)); return; }
 }
 const filtered = tag && store.isStatic ? list.filter(a => (a.tags || '').split(',').includes(tag)) : list;
 const filterBar = tag ? `<div class="tag-filter">正在查看 <strong>#${esc(tag)}</strong><a class="text-link" href="/articles/">查看全部 ${icon('close')}</a></div>` : '';
 const counts = store.articleTags;
 const cloud = Object.entries(counts).sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
 // 登录用户：右栏创作入口 + 草稿箱（点击直接进编辑器）
 const adminSide = store.state.admin && !store.isStatic ? `<section class="side-card admin-side-card"><div class="section-heading"><h3>创作</h3></div><div class="admin-side-actions"><button class="pill" type="button" data-new-article>写新文章 ${icon('arrow')}</button></div><a class="text-link" href="/admin?tab=articles">管理全部文章与草稿</a>${store.articlesDrafts.length ? `<div class="admin-draft-list">${store.articlesDrafts.map(a => `<button type="button" class="admin-draft-item" data-edit-article="${a.id}"><strong>${esc(a.title)}</strong><span>${esc(dateLabel(a.created_at))}</span></button>`).join('')}</div>` : '<p class="small-label side-note">没有草稿</p>'}</section>` : '';
 store.main.innerHTML = `<div class="feed-layout"><section class="timeline">${TABS()}<div class="articles-hero"><h1>文章随笔<span>。</span></h1><p>一些想认真写下来的事，比动态更长，比记忆更可靠。</p></div>${filterBar}<div id="article-rows">${filtered.length ? filtered.map(a => articleTeaserHTML(a)).join('') : empty(tag ? '这个标签下还没有文章' : '第一篇文章，等你落笔', tag ? '换个标签看看吧。' : store.isStatic ? '这里即将装满更长的故事。' : '到管理后台，用 Markdown 或富文本写下第一篇。', 'edit')}</div></section><aside class="right-column">${adminSide}<div class="tags-panel side-tags"><div class="section-heading"><h3>全部标签</h3><span>${cloud.length}</span></div><div class="tags-cloud">${cloud.map(([t, n]) => `<a class="tag-chip${t === tag ? ' active' : ''}" href="/articles/?tag=${encodeURIComponent(t)}">#${esc(t)}<small>${n}</small></a>`).join('')}</div></div><section class="side-card about-card"><h2>订阅</h2><p>通过 RSS 跟踪最新动态与文章，用你喜欢的阅读器即可。</p><div class="about-actions"><a class="pill" href="/rss.xml">订阅 RSS ${icon('arrow')}</a></div></section></aside></div>`;
 const newBtn = document.querySelector('[data-new-article]');
 if (newBtn) newBtn.onclick = () => { store.editingArticle = 'new'; store.editorFormat = 'md'; pushHistory('/admin?tab=articles'); admin(); };
 if (next) {
  const rows = document.querySelector('#article-rows');
  rows.insertAdjacentHTML('afterend', '<button class="secondary" id="more-articles" type="button">加载更多文章</button>');
  const button = document.querySelector('#more-articles');
  const known = new Set(filtered.map(a => a.id));
  button.onclick = async () => {
   button.disabled = true;
   try {
    const result = await apiGet('articles', {tag, cursor: next});
    if (!currentView(view)) return;
    const added = result.articles.filter(a => !known.has(a.id));
    added.forEach(a => known.add(a.id));
    rows.insertAdjacentHTML('beforeend', added.map(articleTeaserHTML).join(''));
    next = result.next_cursor;
    if (!next) button.remove();
   } catch (e) { if (currentView(view)) toast(e.message); }
   finally { button.disabled = false; }
  };
 }
}

export async function articlePage(id) {
 const view = beginView();
 let a;
 try { a = store.isStatic ? (await loadArticles()).find(x => x.id === id) : (await apiGet('articles', {id})).article; }
 catch (e) { if (currentView(view)) store.main.innerHTML = empty('暂时没能加载文章', esc(e.message)); return; }
 if (!currentView(view)) return;
 if (!a) { document.title = store.baseTitle; store.main.innerHTML = empty('这篇文章不存在', '它可能已被删除，或链接有误。', 'edit'); return; }
 document.title = `${a.title} · ${store.state.profile.name}`;
 store.main.innerHTML = `<div class="article-frame"><div class="article-shell"><article class="article-page"><div class="article-head"><div class="article-author">${avatar()}<div class="article-author-id"><strong>${profileName()}${verifiedBadge}</strong><span>@${esc(store.state.profile.handle)} · ${esc(dateLabel(a.created_at))} · ${readingMinutes(a)} 分钟阅读${a.updated_at !== a.created_at ? ' · 已编辑' : ''}</span></div></div><h1>${esc(a.title)}</h1>${tagChips(a)}</div><div class="article-content">${a.html}</div><footer class="article-footer"><a class="secondary" href="/articles/">${icon('arrow')}回到文章列表</a><a class="text-link" href="/">回首页看看</a></footer></article><div class="article-comments" data-article-comments></div></div></div>`;
 mountComments('article', a.id, a.comment_count, document.querySelector('[data-article-comments]'), a.comments);
 if (location.hash === '') window.scrollTo({top: 0});
}

export function adminArticleRows() {
 const row = (a, draft) => `<div class="article-row">${draft ? `<button class="article-row-title" data-edit-article="${a.id}" type="button">${esc(a.title)} <em class="draft-badge">草稿</em></button>` : `<a class="article-row-title" href="/article/${a.id}/">${esc(a.title)}</a>`}${a.tags ? `<span class="article-row-tags">${esc(a.tags.split(',').map(t => '#' + t).join(' '))}</span>` : ''}<span class="article-row-date">${esc(dateLabel(a.created_at))}</span><div class="post-admin"><button class="icon-button" data-edit-article="${a.id}" aria-label="编辑文章" type="button">${icon('edit')}</button><button class="icon-button" data-delete-article="${a.id}" aria-label="删除文章" type="button">${icon('trash')}</button></div></div>`;
 return store.articlesDrafts.map(a => row(a, true)).join('') + store.articles.map(a => row(a, false)).join('') || empty('第一篇文章，等你落笔', '支持 Markdown 和富文本两种编辑模式。', 'edit');
}

export function articleCover(a) {
 if ('cover' in a) return a.cover;
 if (a._cover === undefined) {
  const div = document.createElement('div');
  div.innerHTML = a.html || '';
  const img = div.querySelector('img');
  const src = img ? (img.getAttribute('src') || '') : '';
  a._cover = src.includes('/image?upload=') ? src + '&thumb=1' : src.startsWith('/media/uploads/') ? '/thumbnails/up-' + src.split('/').pop() + '.webp' : src || null;
 }
 return a._cover;
}
