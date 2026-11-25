import { onViewLeave } from '../lib/lifecycle.js';

let active = null;
const draftKey = id => 'muke:article-draft:' + (id ?? 'new');

export function readArticleDraft(id) {
 try {
  const value = JSON.parse(localStorage.getItem(draftKey(id)) || 'null');
  if (!value || value.version !== 1 || value.articleId !== (id ?? null) || !['md', 'html'].includes(value.format) || !Number.isFinite(value.savedAt)) return null;
  if (['title', 'content', 'tags', 'pendingTag'].some(key => typeof value[key] !== 'string')) return null;
  return value;
 } catch { return null; }
}

export function confirmEditorLeave() {
 return !active || active.allowLeave();
}

export function clearArticleDraft(id) {
 if (active?.id === id) active.saved();
 else { try { localStorage.removeItem(draftKey(id)); } catch {} }
}

export function bindEditorProtection(id, getValue, restoreValue) {
 const root = document.querySelector('.editor-page');
 const status = document.createElement('p');
 status.id = 'editor-local-status';
 status.className = 'small-label';
 status.setAttribute('role', 'status');
 root.querySelector('.editor-bottom').before(status);
 let baseline = JSON.stringify(getValue());
 let timer, lastStored = '', lastSavedAt, storageFailed = false;
 const dirty = () => JSON.stringify(getValue()) !== baseline;
 const flush = () => {
  clearTimeout(timer); timer = null;
  const value = getValue(), serialized = JSON.stringify(value);
  try {
   if (serialized === baseline) {
    if (lastStored) localStorage.removeItem(draftKey(id));
    lastStored = '';
    status.textContent = '没有未保存的修改';
   } else if (serialized !== lastStored) {
    const savedAt = Date.now();
    localStorage.setItem(draftKey(id), JSON.stringify({...value, version:1, articleId:id ?? null, savedAt}));
    lastStored = serialized;
    lastSavedAt = savedAt;
   }
   if (serialized !== baseline) status.textContent = '已暂存到此浏览器 · ' + new Date(lastSavedAt).toLocaleTimeString('zh-CN', {hour12:false}) + '（尚未保存到服务器）';
   storageFailed = false;
  } catch {
   storageFailed = true;
   status.textContent = '本地暂存失败：浏览器存储不可用或空间不足，请及时保存到服务器。';
  }
 };
 const schedule = () => {
  status.textContent = dirty() ? '有未保存修改 · 正在本地暂存…' : '没有未保存的修改';
  if (!timer) timer = setTimeout(flush, 700);
 };
 const beforeUnload = event => {
  if (!dirty()) return;
  flush();
  event.preventDefault(); event.returnValue = '';
 };
 const visibility = () => { if (document.visibilityState === 'hidden') flush(); };
 const protection = {
  id,
  saved() {
   baseline = JSON.stringify(getValue());
   lastStored = ''; clearTimeout(timer); timer = null;
   try { localStorage.removeItem(draftKey(id)); } catch { status.textContent = '已保存到服务器，但未能清除本地暂存。'; }
  },
  allowLeave() {
   if (!dirty()) return true;
   flush();
   return confirm(storageFailed ? '有未保存的修改，且本地暂存失败。离开可能丢失内容，确定离开？' : '有尚未保存到服务器的修改，已暂存到此浏览器。确定离开编辑器？');
  }
 };
 active = protection;
 root.addEventListener('input', schedule);
 root.addEventListener('change', schedule);
 window.addEventListener('beforeunload', beforeUnload);
 window.addEventListener('pagehide', flush);
 document.addEventListener('visibilitychange', visibility);
 let recovery;
 onViewLeave(() => {
  if (root.isConnected) flush();
  clearTimeout(timer);
  window.removeEventListener('beforeunload', beforeUnload);
  window.removeEventListener('pagehide', flush);
  document.removeEventListener('visibilitychange', visibility);
  recovery?.remove();
  if (active === protection) active = null;
 });
 const draft = readArticleDraft(id);
 status.textContent = '未保存的修改会自动暂存到此浏览器';
 if (draft) {
  recovery = document.createElement('dialog');
  recovery.id = 'editor-recovery';
  recovery.setAttribute('aria-labelledby', 'editor-recovery-title');
  recovery.innerHTML = '<h2 id="editor-recovery-title">发现本地暂存</h2><p class="small-label"></p><p>恢复只会填入编辑器，不会自动发布。丢弃将删除这份本地暂存。</p><div class="dialog-actions"><button type="button" class="secondary" data-draft-discard>丢弃暂存</button><button type="button" class="primary" data-draft-restore>恢复暂存</button></div>';
  recovery.querySelector('.small-label').textContent = new Date(draft.savedAt).toLocaleString('zh-CN', {hour12:false});
  root.append(recovery);
  recovery.addEventListener('cancel', event => event.preventDefault());
  recovery.querySelector('[data-draft-restore]').onclick = () => {
   restoreValue(draft);
   recovery.close();
   if (!dirty()) protection.saved();
   flush();
  };
  recovery.querySelector('[data-draft-discard]').onclick = () => {
   try {
    localStorage.removeItem(draftKey(id));
    recovery.close();
   } catch { recovery.querySelector('.small-label').textContent = '浏览器阻止清除暂存，请检查存储权限或选择恢复。'; }
  };
  recovery.showModal();
 }
 return protection;
}
