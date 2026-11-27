import { giscusReactions, giscusComments } from '../components/giscus.js';
import { store } from './store.js';
import { admin } from '../views/admin.js';

const icons = {
 pin:'<path d="m16 3 5 5-4 1-3 5-1 4-3-3-6 6 6-6-3-3 4-1 5-3z"/>',
 home:'<path d="m3 10 9-7 9 7v10a1 1 0 0 1-1 1h-5v-7H9v7H4a1 1 0 0 1-1-1z"/>',
 image:'<rect x="3" y="3" width="18" height="18" rx="4"/><circle cx="8" cy="8" r="1.5"/><path d="m21 15-5-5L5 21"/>',
 grid:'<rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/>',
 plus:'<path d="M12 5v14M5 12h14"/>', more:'<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
 search:'<circle cx="10.5" cy="10.5" r="7"/><path d="m16 16 5 5"/>', arrow:'<path d="M5 12h14m-5-5 5 5-5 5"/>',
 download:'<path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/>', close:'<path d="m6 6 12 12M6 18 18 6"/>',
 sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1"/>',
 check:'<path d="m5 12 4 4L19 6"/>', refresh:'<path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 6a8 8 0 0 1 13 2M5 16a8 8 0 0 0 13 2"/>',
 heart:'<path d="M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1.1-1.1a5.5 5.5 0 0 0-7.8 7.8L12 21l8.8-8.6a5.5 5.5 0 0 0 0-7.8Z"/>',
 clock:'<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>', edit:'<path d="m15 4 5 5M4 20l5-1L21 7l-5-5L4 14z"/>', trash:'<path d="M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7"/>',
 chat:'<path d="M21 12a8 8 0 0 1-8 8H4l2-3a8 8 0 1 1 15-5z"/><path d="M9 11h6M9 14h3"/>'
};

export const icon = name => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] || icons.image}</svg>`;

// X 官方认证徽章（fill 型，22×22 viewBox，不能走 stroke 型 icon()）
export const verifiedBadge = '<svg viewBox="0 0 22 22" fill="currentColor" aria-label="认证账户" role="img" class="verified-badge"><path d="M20.396 11c-.018-.646-.215-1.275-.57-1.816-.354-.54-.852-.972-1.438-1.246.223-.607.27-1.264.14-1.897-.131-.634-.437-1.218-.882-1.687-.47-.445-1.053-.75-1.687-.882-.633-.13-1.29-.083-1.897.14-.273-.587-.704-1.086-1.245-1.44S11.647 1.62 11 1.604c-.646.017-1.273.213-1.813.568s-.969.854-1.24 1.44c-.608-.223-1.267-.272-1.902-.14-.635.13-1.22.436-1.69.882-.445.47-.749 1.055-.878 1.688-.13.633-.08 1.29.144 1.896-.587.274-1.087.705-1.443 1.245-.356.54-.555 1.17-.574 1.817.02.647.218 1.276.574 1.817.356.54.856.972 1.443 1.245-.224.606-.274 1.263-.144 1.896.13.634.433 1.218.877 1.688.47.443 1.054.747 1.687.878.633.132 1.29.084 1.897-.136.274.586.705 1.084 1.246 1.439.54.354 1.17.551 1.816.569.647-.016 1.276-.213 1.817-.567s.972-.854 1.245-1.44c.604.239 1.266.296 1.903.164.636-.132 1.22-.447 1.68-.907.46-.46.776-1.044.908-1.681s.075-1.299-.165-1.903c.586-.274 1.084-.705 1.439-1.246.354-.54.551-1.17.569-1.816zM9.662 14.85l-3.429-3.428 1.293-1.302 2.072 2.072 4.4-4.794 1.347 1.246z"/></svg>';

export const esc = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

export function toast(message) { const el = document.querySelector('#toast'); el.textContent = message; el.classList.add('visible'); clearTimeout(toast.timer); toast.timer = setTimeout(() => el.classList.remove('visible'), 3500); }

export const o = (k, d) => (store.state?.options?.[k] ?? d);

// 日期格式常用 token 的客户端渲染（自定义格式用）：Y y m n d j H G i s，其余字符原样输出
function formatDateTokens(fmt, d) {
 let out = '';
 for (let i = 0; i < fmt.length; i++) {
  const c = fmt[i];
  const p2 = n => String(n).padStart(2, '0');
  switch (c) {
   case 'Y': out += d.getFullYear(); break;
   case 'y': out += String(d.getFullYear()).slice(-2); break;
   case 'm': out += p2(d.getMonth() + 1); break;
   case 'n': out += d.getMonth() + 1; break;
   case 'd': out += p2(d.getDate()); break;
   case 'j': out += d.getDate(); break;
   case 'H': out += p2(d.getHours()); break;
   case 'G': out += d.getHours(); break;
   case 'i': out += p2(d.getMinutes()); break;
   case 's': out += p2(d.getSeconds()); break;
   default: out += c;
  }
 }
 return out;
}

export function dateLabel(date) {
 const d = new Date(date);
 const style = o('feed_time_style', '');
 const hideTime = o('feed_hide_time', '0') === '1';
 const locale = o('locale', 'zh-CN');
 const dateOnly = hideTime || style === 'date';
 if (!hideTime && style === 'ago') { // 相对时间：7 天内人性化，之外回退默认日期
  const diff = Math.round((Date.now() - d.getTime()) / 60000);
  if (diff < 1) return '刚刚';
  if (diff < 60) return `${diff} 分钟前`;
  if (diff < 1440) return `${Math.floor(diff / 60)} 小时前`;
  if (diff < 10080) return `${Math.floor(diff / 1440)} 天前`;
 }
 if (!hideTime && style === 'custom') {
  const fmt = (o('feed_time_custom', '') || '').trim();
  if (fmt) return formatDateTokens(fmt, d);
 }
 if (style === 'compact' && !hideTime) {
  const p = n => String(n).padStart(2, '0');
  return sameYear(d) ? `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}` : `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
 }
 // 默认 / 只显示日期 / 隐藏时间：日期部分（今年省年份，其余带年；隐藏时间不含时分）
 return new Intl.DateTimeFormat(locale, sameYear(d)
  ? (dateOnly ? { month: 'long', day: 'numeric' } : { month: 'long', day: 'numeric', hour: '2-digit', minute: '2-digit' })
  : { year: 'numeric', month: 'long', day: 'numeric' }).format(d);
}
function sameYear(d) { return d.getFullYear() === new Date().getFullYear(); }

export function header(title, sub, extra = '') { return `<header class="topbar"><div class="breadcrumb">我的空间 <span>/</span> <strong>${title}</strong></div><div class="topbar-right"><span class="online-dot"></span>${sub}${extra}</div></header>`; }

export function empty(title, text, glyph = 'edit') { return `<div class="empty-state"><span>${icon(glyph)}</span><h3>${title}</h3><p>${text}</p></div>`; }

export function profileName() { return esc(store.state.profile.name); }

export function avatar(extra = '') { return store.state.profile.avatar ? `<img class="avatar ${extra}" src="${esc(store.state.profile.avatar)}" alt="${profileName()}">` : `<span class="avatar ${extra}">${esc(store.state.profile.name.slice(0,1))}</span>`; }

export const postsByDate = () => [...store.posts].sort((x, y) => Number(y.pinned || 0) - Number(x.pinned || 0) || new Date(y.created_at) - new Date(x.created_at) || y.id - x.id);

const cardRenderers = window.MKBLOG._cards;
window.MKBLOG.esc = esc;
window.MKBLOG.icon = icon;

export function postItems(images) {
 return images.map(img => ({src:img.url,width:img.width,height:img.height||1200,downloadUrl:img.url+(img.url.includes('?')?'&download=1':''),downloadName:img.name||'动态图片',caption:img.width?`<small>${img.width} × ${img.height}${img.bytes?` · ${(img.bytes/1024/1024).toFixed(1)} MB`:''}</small>`:''}));
}

// 插件徽标名称 / 图片全尺寸标记：来自 state.plugins（后端插件清单）
function pluginLabel(src) { return store.state?.plugins?.[src]?.name || src; }
// 插件主题色（state.plugins[id].color，服务端已消毒为 hex）→ 徽章类。
// CSP style-src 'self' 禁 inline style 属性，走构造式样式表注入 .source-badge.sb-{id}{--badge-color:…}
let badgeSheet = null;
function pluginBadgeClass(src) {
 const plugins = store.state?.plugins || {};
 if (!badgeSheet) {
  const rules = Object.entries(plugins).filter(([, v]) => /^#[0-9a-f]{3,8}$/i.test(v?.color || ''))
   .map(([id, v]) => `.source-badge.sb-${id}{--badge-color:${v.color}}`).join('');
  if (!rules) return '';
  badgeSheet = new CSSStyleSheet();
  badgeSheet.replaceSync(rules);
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, badgeSheet];
 }
 return plugins[src]?.color ? ` sb-${src}` : '';
}
function pluginFullImages(src) { return !!store.state?.plugins?.[src]?.full_images; }
export function thirdPartyComments() { return ['giscus', 'waline', 'twikoo', 'custom'].includes(store.state?.options?.comment_provider || ''); }
export function postCommentsOn() { const o = store.state?.options || {}; return o.comment_provider !== 'off' && o.comment_scope !== 'article'; }

export function updateProfile(){document.querySelectorAll('[data-profile="name"]').forEach(el=>el.textContent=store.state.profile.name);document.querySelectorAll('[data-profile="handle"]').forEach(el=>el.textContent='@'+(store.state.profile.handle||''));const sb=document.querySelector('.sidebar-bottom');const av=sb?.querySelector('.avatar');if(sb&&av)av.outerHTML=avatar('small');}

// 动态头部身份：插件动态可声明来源身份（meta.avatar 图标 + meta.name 名称 + meta.handle 域名），
// 如 RSS 订阅显示订阅站点的 favicon 与站名；未声明时用站长头像 / 昵称
export function postAvatarHTML(post, extra = '') {
 const a = post.meta?.avatar;
 return a?.url ? `<img class="avatar ${extra}" src="${esc(a.thumb || a.url)}" alt="${esc(post.meta?.name || '')}">` : avatar(extra);
}

export function postHTML(post) {
 const giscus = store.state?.options?.comment_provider === 'giscus';
 const reaction = giscusReactions.get(`post:${post.id}`);
 const liked = giscus ? !!reaction?.viewerHasReacted : store.likedIds.has(post.id);
 const likes = giscus ? (reaction?.count ?? post.giscus_counts?.likes) : post.like_count;
 const comments = giscus ? (giscusComments.get(`post:${post.id}`) ?? post.giscus_counts?.comments) : post.comment_count;
 const singleCompact = post.images.length === 1 && !pluginFullImages(post.source);
 const feed = post.meta?.avatar?.url ? post.meta : null; // 来源身份三件套齐全才整体切换
 const images = (post.images || []).map((img, i) => { const ar = img.width && img.height ? Math.max(img.width / img.height, 0.5) : 1.5; return `<button class="post-image" type="button" data-media-post="${post.id}" data-media-index="${i}" aria-label="查看图片"${singleCompact ? ` style="aspect-ratio:${ar.toFixed(3)}"` : ''}><img src="${esc(img.thumb)}" alt="${esc(img.name || '动态图片')}" loading="lazy"></button>`; }).join('');
 return `<article class="post" id="post-${post.id}" data-source="${esc(post.source || '')}">${postAvatarHTML(post)}<div class="post-body">${Number(post.pinned) ? '<span class="pinned-label">置顶</span>' : ''}<div class="post-meta"><strong>${feed ? esc(feed.name || post.source) : profileName()}</strong>${feed ? '' : verifiedBadge}<span>@${feed ? esc(feed.handle || '') : esc(store.state.profile.handle)}${(() => { const t = dateLabel(post.created_at); return t ? ` · ${esc(t)}` : ''; })()}</span>${post.source ? `<span class="source-badge${pluginBadgeClass(post.source)}">${esc(pluginLabel(post.source))}</span>` : ''}${store.state.admin ? `<div class="post-admin"><button class="icon-button" data-pin="${post.id}" aria-label="${Number(post.pinned) ? '取消置顶' : '置顶动态'}" title="${Number(post.pinned) ? '取消置顶' : '置顶动态'}" aria-pressed="${!!Number(post.pinned)}">${icon('pin')}</button>${post.source ? '' : `<button class="icon-button" data-edit="${post.id}" aria-label="编辑动态">${icon('edit')}</button>`}<button class="icon-button" data-delete="${post.id}" aria-label="删除动态">${icon('trash')}</button></div>` : ''}</div>${post.meta && cardRenderers[post.meta.card] ? cardRenderers[post.meta.card](post) : `<p class="post-content">${esc(post.content)}</p>${images ? `<div class="post-images count${post.images.length}${singleCompact ? ' single-compact' : ''}">${images}</div>` : ''}`}<div class="post-actions">${((store.isStatic || thirdPartyComments()) && store.state?.options?.comment_provider !== 'giscus') || !postCommentsOn() ? '' : `<button data-post-comments="${post.id}">${icon('chat')}<span${giscus ? ` data-giscus-comments="post:${post.id}"` : ''}>${giscus ? (comments === undefined ? '评论' : `${comments} 评论`) : (thirdPartyComments() ? '评论' : (post.comment_count || '评论'))}</span></button>`}<button class="like-button ${liked ? 'liked' : ''}" ${giscus ? 'data-giscus-like' : 'data-like'}="${post.id}" aria-pressed="${liked}"${giscus ? ' title="通过 GitHub 点赞，展开后选择 ❤️"' : ''}>${icon('heart')}<span>${giscus && likes !== undefined ? likes : (likes ? likes : (liked ? '已点赞' : '点赞'))}</span></button><button data-share="${post.id}">${icon('arrow')}<span>分享</span></button>${post.link && /^https?:\/\//i.test(post.link) && !post.meta?.hide_source ? `<a class="post-link" href="${esc(post.link)}" target="_blank" rel="noopener">${icon('arrow')}原文</a>` : ''}${post.updated_at !== post.created_at ? '<span class="edited">已编辑</span>' : ''}</div><div class="post-comments-slot" data-comments-slot="${post.id}" hidden></div></div></article>`;
}
