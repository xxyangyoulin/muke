const origin = 'https://giscus.app';
export const giscusReactions = new Map();
export const giscusComments = new Map();
let session = '';
try { session = JSON.parse(localStorage.getItem('giscus-session') || '""'); } catch {}
const callback = new URL(location.href);
if (callback.searchParams.has('giscus')) {
 session = callback.searchParams.get('giscus');
 try { localStorage.setItem('giscus-session', JSON.stringify(session)); } catch {}
 callback.searchParams.delete('giscus');
 history.replaceState(history.state, '', callback.href);
}

window.addEventListener('message', event => {
 if (event.origin !== origin || !event.data?.giscus) return;
 const frame = [...document.querySelectorAll('iframe[data-giscus]')].find(frame => frame.contentWindow === event.source);
 if (!frame) return;
 const message = event.data.giscus;
 const discussion = message.discussion;
 if (Number.isInteger(discussion?.totalCommentCount) && Number.isInteger(discussion?.totalReplyCount) && !discussion.pageInfo?.hasNextPage && !discussion.pageInfo?.hasPreviousPage) {
  const count = discussion.totalCommentCount + discussion.totalReplyCount;
  giscusComments.set(frame.dataset.giscus, count);
  for (const label of document.querySelectorAll(`[data-giscus-comments="${frame.dataset.giscus}"]`)) label.textContent = `${count} 评论`;
 }
 const heart = discussion?.reactions?.HEART;
 if (heart && Number.isInteger(heart.count) && heart.count >= 0 && typeof heart.viewerHasReacted === 'boolean') {
  giscusReactions.set(frame.dataset.giscus, heart);
  if (frame.dataset.giscus.startsWith('post:')) {
   const id = frame.dataset.giscus.slice(5);
   for (const button of document.querySelectorAll(`[data-giscus-like="${id}"]`)) {
    button.classList.toggle('liked', heart.viewerHasReacted);
    button.setAttribute('aria-pressed', String(heart.viewerHasReacted));
    button.querySelector('span').textContent = heart.count ? String(heart.count) : '点赞';
   }
  }
 }
 if (Number.isFinite(message.resizeHeight) && message.resizeHeight > 0) frame.style.height = `${message.resizeHeight}px`;
 if (message.signOut || /Bad credentials|Invalid state value|State has expired/.test(message.error || '')) {
  session = '';
  giscusReactions.clear();
  for (const button of document.querySelectorAll('[data-giscus-like]')) {
   button.classList.remove('liked');
   button.setAttribute('aria-pressed', 'false');
   button.querySelector('span').textContent = '点赞';
  }
  try { localStorage.removeItem('giscus-session'); } catch {}
  for (const item of document.querySelectorAll('iframe[data-giscus]')) {
   const url = new URL(item.src);
   if (!url.searchParams.has('session')) continue;
   url.searchParams.delete('session');
   item.src = url.href;
  }
 }
});

export function mountGiscus(kind, id, host, options) {
 host.replaceChildren();
 if (!options.giscus_repo || !options.giscus_repo_id || !options.giscus_category_id) {
  const text = document.createElement('p');
  text.className = 'small-label';
  text.textContent = '评论尚未配置完成。';
  host.append(text);
  return;
 }
 const page = new URL(location.href);
 page.searchParams.delete('giscus');
 // 动态的哈希用于登录返回和原文链接；讨论身份始终使用内容类型与 ID。
 if (kind === 'post') page.hash = `post-${id}`;
 const params = new URLSearchParams({
  repo: options.giscus_repo, repoId: options.giscus_repo_id, categoryId: options.giscus_category_id,
  term: `${kind}:${id}`, strict: '1', theme: 'light', reactionsEnabled: '1', emitMetadata: '1',
  inputPosition: 'bottom', origin: page.href, backLink: page.href,
 });
 if (session) params.set('session', session);
 const panel = document.createElement('div');
 panel.className = 'panel comments-panel embed-host';
 const frame = document.createElement('iframe');
 frame.className = 'comment-embed';
 frame.dataset.giscus = `${kind}:${id}`;
 frame.title = 'giscus 评论';
 const loading = document.createElement('p');
 loading.className = 'small-label';
 loading.setAttribute('role', 'status');
 loading.textContent = '评论加载中…';
 frame.style.display = 'none';
 frame.loading = 'eager';
 frame.addEventListener('load', () => {
  loading.remove();
  frame.style.removeProperty('display');
 }, {once:true});
 frame.allow = 'clipboard-write';
 frame.src = `${origin}/zh-CN/widget?${params}`;
 panel.append(loading, frame);
 host.append(panel);
}
