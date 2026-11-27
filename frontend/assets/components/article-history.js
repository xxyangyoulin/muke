import { api, apiGet } from '../lib/api.js';
import { esc, toast } from '../lib/ui.js';
import { onViewLeave } from '../lib/lifecycle.js';

export function revisionDiff(before, after) {
 const a = before.split('\n'), b = after.split('\n');
 let start = 0, end = 0;
 while (start < a.length && start < b.length && a[start] === b[start]) start++;
 while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++;
 const left = a.slice(start, a.length - end), right = b.slice(start, b.length - end);
 const chunks = [];
 const add = (type, lines) => {
  if (!lines.length) return;
  const last = chunks.at(-1);
  if (last?.type === type) last.lines.push(...lines);
  else chunks.push({type, lines});
 };
 add('same', a.slice(0, start));
 // 大篇幅重写按块展示，避免行级差异矩阵占用过多内存或阻塞编辑器。
 if ((left.length + 1) * (right.length + 1) > 1000000) {
  add('removed', left); add('added', right);
 } else {
  const width = right.length + 1;
  const lengths = new Uint32Array((left.length + 1) * width);
  for (let i = left.length - 1; i >= 0; i--) for (let j = right.length - 1; j >= 0; j--) {
   lengths[i * width + j] = left[i] === right[j] ? 1 + lengths[(i + 1) * width + j + 1] : Math.max(lengths[(i + 1) * width + j], lengths[i * width + j + 1]);
  }
  let i = 0, j = 0;
  while (i < left.length || j < right.length) {
   if (i < left.length && j < right.length && left[i] === right[j]) { add('same', [left[i++]]); j++; }
   else if (i < left.length && (j === right.length || lengths[(i + 1) * width + j] >= lengths[i * width + j + 1])) add('removed', [left[i++]]);
   else add('added', [right[j++]]);
  }
 }
 add('same', end ? a.slice(a.length - end) : []);
 return chunks.map(chunk => ({type:chunk.type, text:chunk.lines.join('\n')}));
}

export function bindArticleHistory(article, getCurrent, onRestore) {
 const button = document.querySelector('#article-history');
 if (!button) return;
 const dialog = document.createElement('dialog');
 dialog.className = 'article-history-dialog';
 dialog.setAttribute('aria-label', '文章修订历史');
 dialog.innerHTML = `<div class="section-heading"><h2>修订历史</h2><button type="button" class="text-link" data-history-close>关闭</button></div><p class="small-label">保存修改前自动保留原版本；关闭此窗口不会丢失编辑内容。</p><div class="history-layout"><div class="history-sidebar"><div id="revision-list"></div><button type="button" class="text-link" id="revision-more" hidden>加载更早版本</button></div><section id="revision-detail" aria-live="polite">选择一个版本查看差异。</section></div>`;
 document.querySelector('.editor-page').append(dialog);
 let request, detailRequest, current, cursor;
 const list = dialog.querySelector('#revision-list');
 const detail = dialog.querySelector('#revision-detail');
 const more = dialog.querySelector('#revision-more');
 const time = value => new Date(value).toLocaleString('zh-CN', {year:'numeric', month:'2-digit', day:'2-digit', hour:'2-digit', minute:'2-digit', second:'2-digit', hourCycle:'h23'});
 const loadList = async () => {
  more.disabled = true;
  try {
   const result = await apiGet('article-revisions', {id:article.id, ...(cursor ? {cursor} : {})}, {signal:request.signal});
   if (!dialog.open) return;
   list.insertAdjacentHTML('beforeend', result.revisions.map(r => `<button type="button" class="revision-item" data-revision="${r.id}" aria-pressed="false"><strong>#${r.id} · ${esc(time(r.saved_at))}</strong><span>${esc(r.title)}</span><small>${Number(r.draft) ? '草稿' : '已发布'}</small></button>`).join(''));
   if (!list.children.length) list.textContent = '暂无历史版本，修改并保存后会自动保留。';
   cursor = result.next_cursor;
   more.hidden = !cursor;
  } catch (e) { if (e.name !== 'AbortError') { toast(e.message); more.hidden = false; } }
  finally { more.disabled = false; }
 };
 button.onclick = () => {
  current = getCurrent();
  cursor = null;
  list.innerHTML = '';
  detail.textContent = '选择一个版本查看差异。';
  more.hidden = true;
  request = new AbortController();
  dialog.showModal();
  loadList();
 };
 more.onclick = loadList;
 dialog.querySelector('[data-history-close]').onclick = () => dialog.close();
 dialog.addEventListener('close', () => { request?.abort(); detailRequest?.abort(); });
 onViewLeave(() => { request?.abort(); detailRequest?.abort(); dialog.remove(); });
 list.onclick = async event => {
  const selected = event.target.closest('[data-revision]');
  if (!selected) return;
  detailRequest?.abort();
  detailRequest = new AbortController();
  const signal = detailRequest.signal;
  list.querySelectorAll('[data-revision]').forEach(item => item.setAttribute('aria-pressed', String(item === selected)));
  detail.textContent = '正在加载版本…';
  try {
   const {revision} = await apiGet('article-revisions', {id:article.id, revision:selected.dataset.revision}, {signal});
   if (signal.aborted || !dialog.open) return;
   const rows = [['标题', revision.title, current.title], ['标签', revision.tags, current.tags], ['状态', Number(revision.draft) ? '草稿' : '已发布', Number(current.draft) ? '草稿' : '已发布'], ['格式', revision.format, current.format]];
   const chunks = revisionDiff(revision.content, current.content);
   detail.innerHTML = `<h3>版本 #${revision.id}</h3><p class="small-label">版本更新时间：${esc(time(revision.updated_at))}<br>归档时间：${esc(time(revision.saved_at))}</p><div class="revision-table-wrap"><table class="revision-meta"><thead><tr><th>字段</th><th>历史版本</th><th>当前编辑内容</th></tr></thead><tbody>${rows.map(([label, old, now]) => `<tr class="${old !== now ? 'changed' : ''}"><th>${label}</th><td>${esc(old || '—')}</td><td>${esc(now || '—')}</td></tr>`).join('')}</tbody></table></div><h3>正文差异${revision.format === 'html' ? '（HTML 源码）' : ''}</h3><p class="small-label">历史 → 当前编辑内容（含未保存修改）：红色 − 表示移除，绿色 + 表示新增。</p>${revision.content === current.content ? '<p class="small-label">正文无差异。</p>' : ''}<div class="revision-diff">${chunks.map(c => `<pre class="revision-${c.type}">${esc(c.text)}</pre>`).join('')}</div><button type="button" class="primary" id="revision-restore">恢复此版本</button>`;
   detail.querySelector('#revision-restore').onclick = async event => {
    if (!confirm(`恢复为${Number(revision.draft) ? '草稿（将从公开页面撤下）' : '已发布文章'}？标题、正文、标签和状态会立即恢复。当前已保存版本会留入历史；未保存的编辑内容将被丢弃。`)) return;
    const restore = event.currentTarget;
    restore.disabled = true;
    try {
     const result = await api('restore-article-revision', {id:article.id, revision:revision.id});
     if (!dialog.isConnected) return;
     dialog.close();
     await onRestore(result.article);
    } catch (e) { toast(e.message); }
    finally { restore.disabled = false; }
   };
  } catch (e) { if (e.name !== 'AbortError') { detail.textContent = '版本加载失败，请重新选择。'; toast(e.message); } }
 };
}
