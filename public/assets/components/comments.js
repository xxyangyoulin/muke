// 评论组件：文章详情页（整段面板）与动态卡片（行内展开）共用。
// 动态部署专用（静态导出走第三方注入/只读烘焙，见 P2）；身份为匿名昵称，浏览器本地记住。
import { store } from '../lib/store.js';
import { api, apiGet } from '../lib/api.js';
import { icon, esc, toast, dateLabel, thirdPartyComments } from '../lib/ui.js';

let profile = null;
try { profile = JSON.parse(localStorage.getItem('muke-commenter') || 'null'); } catch { profile = null; }

// X 式默认头像：灰圆底 + 中性人形剪影（无 Gravatar 的访客使用）
const DEFAULT_AVATAR = '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"/></svg>';

function commentRowHTML(c) {
 return `<div class="comment-row" data-cid="${c.id}">${c.email_hash ? `<img class="comment-avatar" src="/image?gravatar=${esc(c.email_hash)}" alt="" loading="lazy">` : `<span class="comment-avatar">${DEFAULT_AVATAR}</span>`}<div class="comment-main"><div class="comment-head"><strong>${esc(c.author)}</strong>${c.reply_name ? `<span class="comment-reply-at">回复 @${esc(c.reply_name)}</span>` : ''}${c.site ? `<a class="comment-site" href="${esc(c.site)}" target="_blank" rel="noopener nofollow">${icon('arrow')}</a>` : ''}<time>${esc(dateLabel(c.created_at))}</time></div><p class="comment-content">${esc(c.content)}</p><button class="text-link comment-reply" data-reply="${c.id}" data-reply-name="${esc(c.author)}" type="button">回复</button></div></div>`;
}

export function commentsSectionHTML(kind, id, count) {
 const n = count || 0;
 return `<section class="panel comments-panel" data-comments-section="${kind}:${id}">
  <div class="section-heading"><h3>${icon('chat')} 评论 ${n ? `<small>${n} 条</small>` : ''}</h3><span class="small-label">温柔地说话，就像这里有人听。</span></div>
  <div class="comment-list" data-comment-list>...</div>
  <form class="comment-form" data-comment-form>
   <div class="comment-form-row">
    <input name="author" maxlength="40" placeholder="昵称 *" required value="${esc(profile?.author || '')}" aria-label="昵称">
    <input name="email" type="email" maxlength="100" placeholder="邮箱（选填，不公开）" value="${esc(profile?.email || '')}" aria-label="邮箱">
    <input name="site" maxlength="200" placeholder="网址（选填）" value="${esc(profile?.site || '')}" aria-label="网址">
   </div>
   <textarea name="content" maxlength="1000" rows="3" placeholder="写下评论…" required aria-label="评论内容"></textarea>
   <input type="text" name="website_url" class="hp-field" tabindex="-1" autocomplete="off" aria-hidden="true">
   <div class="comment-form-bottom"><span class="comment-context" data-comment-context></span><button class="primary comment-submit" type="submit">发表评论</button></div>
  </form>
 </section>`;
}

export async function loadComments(section) {
 const key = section.dataset.commentsSection;
 const [kind, id] = key.split(':');
 const list = section.querySelector('[data-comment-list]');
 try {
  const data = await apiGet('comments', { kind, target: id });
  list.innerHTML = data.comments.length
   ? data.comments.map(commentRowHTML).join('')
   : '<p class="small-label comment-empty">还没有评论，坐个沙发？</p>';
 } catch (e) {
  list.innerHTML = `<p class="small-label">${esc(e.message)}</p>`;
 }
}

export function bindCommentSection(section) {
 const key = section.dataset.commentsSection;
 const form = section.querySelector('[data-comment-form]');
 const context = form.querySelector('[data-comment-context]');
 let replyTo = 0;
 section.addEventListener('click', e => {
  const btn = e.target.closest('[data-reply]');
  if (!btn) return;
  replyTo = Number(btn.dataset.reply);
  context.textContent = `回复 @${btn.dataset.replyName}：`;
  form.querySelector('textarea').focus();
 });
 form.addEventListener('submit', async e => {
  e.preventDefault();
  const [kind, id] = key.split(':');
  const data = new FormData(form);
  try {
   await api('comment', {
    kind, target: Number(id), reply_to: replyTo,
    author: data.get('author'), email: data.get('email'), site: data.get('site'),
    content: data.get('content'), website_url: data.get('website_url'),
   });
   try { localStorage.setItem('muke-commenter', JSON.stringify({ author: data.get('author'), email: data.get('email'), site: data.get('site') })); } catch {}
   form.querySelector('textarea').value = '';
   replyTo = 0; context.textContent = '';
   toast('评论已发表');
   await loadComments(section);
   // 刷新计数显示
   const heading = section.querySelector('.section-heading h3 small');
   const list = section.querySelector('[data-comment-list]');
   if (heading) heading.textContent = `${list.querySelectorAll('.comment-row').length} 条`;
  } catch (err) { toast(err.message); }
 });
 loadComments(section);
}

/** 当前评论提供方（'' = 原生；第三方 = giscus/waline/twikoo/custom） */
function provider() {
 return store.state?.options?.comment_provider || '';
}

/** 静态站只读评论列表（烘焙数据来自 articles.json） */
function readonlySectionHTML(list) {
 if (!list || !list.length) return '';
 const rows = list.map(c => `<div class="comment-row">${c.email_hash ? `<img class="comment-avatar" src="/image?gravatar=${esc(c.email_hash)}" alt="" loading="lazy">` : `<span class="comment-avatar">${DEFAULT_AVATAR}</span>`}<div class="comment-main"><div class="comment-head"><strong>${esc(c.author)}</strong>${c.reply_name ? `<span class="comment-reply-at">回复 @${esc(c.reply_name)}</span>` : ''}<time>${esc(dateLabel(c.created_at))}</time></div><p class="comment-content">${esc(c.content)}</p></div></div>`).join('');
 return `<section class="panel comments-panel comments-baked"><div class="section-heading"><h3>评论 <small>${list.length} 条</small></h3></div>${rows}</section>`;
}

/** 挂载完整评论面板（文章页用；按部署形态与提供方分流） */
export function mountComments(kind, id, count, host, bakedList) {
 if (!host || provider() === 'off') return;
 const p = provider();
 if (p === 'giscus' || p === 'custom' || p === 'waline' || p === 'twikoo') {
  // 第三方：统一 iframe（giscus/custom 直连外部；waline/twikoo 走同源嵌入页）
  const src = (p === 'giscus' || p === 'custom') ? store.state.options.comment_embed_url : '/embed/comments/';
  if (!src) return;
  host.innerHTML = `<div class="panel comments-panel embed-host"><iframe class="comment-embed" src="${esc(src)}" title="评论" loading="lazy" allow="clipboard-write"></iframe></div>`;
  return;
 }
 if (store.isStatic) { // 原生 + 静态站：只读烘焙
  host.innerHTML = readonlySectionHTML(bakedList);
  return;
 }
 host.innerHTML = commentsSectionHTML(kind, id, count);
 bindCommentSection(host.firstElementChild);
}

/** 动态卡片行内评论位（点「评论」按钮时懒挂载；第三方模式下按钮已隐藏） */
export function mountInlineComments(post, host) {
 mountComments('post', post.id, post.comment_count, host);
}
