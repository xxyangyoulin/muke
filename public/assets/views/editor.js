import { store } from '../lib/store.js';
import { api } from '../lib/api.js';
import { icon, esc, toast, header } from '../lib/ui.js';
import { loadArticles } from './articles.js';
import { admin } from './admin.js';
import { initTagInput } from '../components/taginput.js';

export function editorContent() { return document.querySelector('#article-content'); }

export function getEditorValue() {
 const el = editorContent();
 if (el?.tagName === 'TEXTAREA') return el.value;
 return el?.innerHTML || '';
}

export function editorCharCount() {
 const el = editorContent();
 const text = el?.tagName === 'TEXTAREA' ? el.value : (el?.textContent || '');
 return Array.from(text).length;
}

export function updateEditorCount() { const el = document.querySelector('#editor-count'); if (el) el.textContent = `${editorCharCount()} 字`; }

export function sanitizeRichText(root) {
 root.querySelectorAll('script,style,iframe,object,embed,link,meta,form,input,button').forEach(n => n.remove());
 root.querySelectorAll('*').forEach(el => {
  [...el.attributes].forEach(attr => {
   const name = attr.name.toLowerCase();
   if (name.startsWith('on') || name === 'style' || name === 'class' || name === 'id' || name === 'contenteditable') el.removeAttribute(attr.name);
   if ((name === 'href' || name === 'src') && /^\s*javascript:/i.test(attr.value)) el.removeAttribute(attr.name);
  });
 });
}

export function uploadEditorImage(file) {
 const data = new FormData();
 data.append('image', file);
 return fetch('/api?action=upload-image', {method: 'POST', headers: {'X-CSRF-Token': store.state.csrf}, body: data})
  .then(r => r.json().then(result => { if (!r.ok) throw new Error(result.error || '上传失败'); return result.url; }));
}

export function editorToolbarHTML(format) {
 if (format === 'html') return `<button type="button" data-cmd="bold" title="加粗"><strong>B</strong></button><button type="button" data-cmd="italic" title="斜体"><em>I</em></button><button type="button" data-block="h2" title="二级标题">H2</button><button type="button" data-block="h3" title="三级标题">H3</button><button type="button" data-block="p" title="正文">正文</button><button type="button" data-cmd="insertUnorderedList" title="无序列表">•&nbsp;列表</button><button type="button" data-cmd="insertOrderedList" title="有序列表">1.&nbsp;列表</button><button type="button" data-block="blockquote" title="引用">“&nbsp;引用</button><button type="button" data-wrap-code="1" title="行内代码">&lt;/&gt;</button><button type="button" data-link="1" title="插入链接">链接</button><button type="button" data-image="1" title="插入图片">${icon('image')}</button><select id="img-width" title="插入图片的宽度比例；富文本中选中图片后选择即可改其宽度"><option value="100">图片 100%</option><option value="75">75%</option><option value="50">50%</option><option value="25">25%</option></select>`;
 return `<button type="button" data-md="**" title="加粗"><strong>B</strong></button><button type="button" data-md="*" title="斜体"><em>I</em></button><button type="button" data-md-line="## " title="二级标题">H2</button><button type="button" data-md-line="### " title="三级标题">H3</button><button type="button" data-md-line="- " title="列表项">•&nbsp;列表</button><button type="button" data-md-line="> " title="引用">“&nbsp;引用</button><button type="button" data-md="\`" title="行内代码">&lt;/&gt;</button><button type="button" data-link="1" title="插入链接">链接</button><button type="button" data-image="1" title="插入图片">${icon('image')}</button><select id="img-width" title="插入图片的宽度比例；富文本中选中图片后选择即可改其宽度"><option value="100">图片 100%</option><option value="75">75%</option><option value="50">50%</option><option value="25">25%</option></select><button type="button" data-codeblock="1" title="代码块">代码块</button>`;
}

export function renderArticleEditor() {
 const article = store.editingArticle === 'new' ? null : store.editingArticle;
 const format = article ? article.format : store.editorFormat;
 document.title = (article ? '编辑文章' : '写新文章') + ' · ' + store.baseTitle.split(' · ')[0];
 store.main.innerHTML = header('文章随笔', article ? '修改这篇文章' : '新的一篇', '<button id="editor-back" class="text-link" type="button">返回后台</button>') + `<div class="editor-page"><input id="article-title" placeholder="给这篇文章起个标题" maxlength="80" value="${article ? esc(article.title) : ''}"><div class="tag-box" id="tag-box"><div class="tag-chips" id="tag-chips"></div><input id="tag-field" autocomplete="off" maxlength="20"><div class="tag-suggest" id="tag-suggest" hidden></div><input type="hidden" id="article-tags" value="${article ? esc(article.tags || '') : ''}"></div><div class="format-tabs">${article ? `<span class="format-locked">${format === 'md' ? 'Markdown 模式（创建后不可切换）' : '富文本模式（创建后不可切换）'}</span>` : `<button type="button" data-format="md" class="${format === 'md' ? 'active' : ''}">Markdown</button><button type="button" data-format="html" class="${format === 'html' ? 'active' : ''}">富文本</button>`}</div><div class="editor-toolbar" id="editor-toolbar">${editorToolbarHTML(format)}</div>${format === 'html' ? `<div class="rt-editor" id="article-content" contenteditable="true">${article ? article.content : ''}</div>` : `<textarea id="article-content" placeholder="支持 Markdown：# 标题 / **加粗** / *斜体* / - 列表 / > 引用 / \`行内代码\` / ![](图片) / \`\`\`代码块\`\`\`">${article ? esc(article.content) : ''}</textarea>`}<div class="editor-bottom"><span id="editor-count" class="small-label"></span><div class="dialog-actions">${article ? `<button class="danger" id="article-delete" type="button">删除文章</button>` : ''}<button class="secondary" id="preview-toggle" type="button">预览</button><button class="secondary" id="article-draft" type="button">存草稿</button><button class="primary" id="article-save" type="button">${article && article.draft ? '发布' : article ? '保存更新' : '发布'} ${icon('arrow')}</button></div></div><div class="editor-preview" id="editor-preview" hidden><div class="section-heading"><h3>实时预览</h3><span class="small-label">由服务端渲染，与线上一致</span></div><div class="article-content" id="preview-body"></div></div><input type="file" id="editor-image-input" accept="image/jpeg,image/png,image/webp,image/gif,image/avif" hidden></div>`;
 bindArticleEditor(format, article);
}

export function mdApply(ta, before, after = before) {
 const {selectionStart: s, selectionEnd: e, value: v} = ta;
 ta.value = v.slice(0, s) + before + v.slice(s, e) + after + v.slice(e);
 ta.selectionStart = s + before.length;
 ta.selectionEnd = e + before.length;
 ta.focus();
 updateEditorCount();
}

export function mdLinePrefix(ta, prefix) {
 const {selectionStart: s, value: v} = ta;
 const lineStart = v.lastIndexOf('\n', s - 1) + 1;
 ta.value = v.slice(0, lineStart) + prefix + v.slice(lineStart);
 ta.selectionStart = ta.selectionEnd = s + prefix.length;
 ta.focus();
 updateEditorCount();
}

export function mdInsert(ta, text) {
 const {selectionStart: s, value: v} = ta;
 ta.value = v.slice(0, s) + text + v.slice(s);
 ta.selectionStart = ta.selectionEnd = s + text.length;
 ta.focus();
 updateEditorCount();
}

export function renderEditorPreview(format) {
 clearTimeout(previewTimer);
 previewTimer = setTimeout(async () => {
  try {
    const result = await api('render', {content: getEditorValue(), format});
    const body = document.querySelector('#preview-body');
    if (body) body.innerHTML = result.html;
  } catch (e) { toast(e.message); }
 }, 600);
}

export let previewTimer;

export function bindArticleEditor(format, article = null) {
 updateEditorCount();
 const tagPool = [...(store.articles || []), ...(store.articlesDrafts || [])].flatMap(a => (a.tags || '').split(',')).map(t => t.trim()).filter(Boolean);
 initTagInput({ initial: article?.tags || '', pool: tagPool });
 document.querySelector('#editor-back').onclick = () => { store.editingArticle = false; admin(); };
 document.querySelectorAll('.format-tabs [data-format]').forEach(button => button.onclick = () => {
  const next = button.dataset.format;
  if (next === format) return;
  if (getEditorValue().trim() !== '' && !confirm('切换编辑模式会清空当前内容，确定吗？')) return;
  store.editorFormat = next;
  renderArticleEditor();
 });
 const content = editorContent();
 content?.addEventListener('input', () => { updateEditorCount(); if (!document.querySelector('#editor-preview').hidden) renderEditorPreview(format); });
 const toolbar = document.querySelector('#editor-toolbar');
 const imageInput = document.querySelector('#editor-image-input');
 toolbar.addEventListener('click', e => {
  const button = e.target.closest('button');
  if (!button) return;
  const cmd = button.dataset.cmd, block = button.dataset.block, wrap = button.dataset.md, line = button.dataset.mdLine;
  content?.focus();
  if (cmd) document.execCommand(cmd, false, null);
  else if (block) document.execCommand('formatBlock', false, block === 'p' ? 'p' : block);
  else if (wrap) mdApply(content, wrap);
  else if (line) mdLinePrefix(content, line);
  else if (button.dataset.link) {
   const url = prompt('链接地址：', 'https://');
   if (!url) return;
   if (format === 'html') document.execCommand('createLink', false, url);
   else mdApply(content, '[', `](${url})`);
  } else if (button.dataset.codeblock) {
   const sel = content.value.slice(content.selectionStart, content.selectionEnd);
   mdInsert(content, '\n```\n' + sel + '\n```\n');
  } else if (button.dataset.wrapCode) {
   const sel = window.getSelection().toString();
   document.execCommand('insertHTML', false, `<code>${esc(sel)}</code>&nbsp;`);
  } else if (button.dataset.image) imageInput.click();
  updateEditorCount();
  if (!document.querySelector('#editor-preview').hidden) renderEditorPreview(format);
 });
 imageInput.onchange = async () => {
  const file = imageInput.files[0];
  imageInput.value = '';
  if (!file) return;
  if (file.size > 15 * 1024 * 1024) { toast('图片不能超过 15MB'); return; }
  toast('正在上传图片…');
  try {
   const url = await uploadEditorImage(file);
   content?.focus();
   const ratio = document.querySelector('#img-width')?.value || '100';
   if (format === 'html') document.execCommand('insertHTML', false, `<img src="${esc(url)}" alt=""${ratio !== '100' ? ` width="${ratio}%"` : ''}><br>`);
   else mdInsert(content, ratio === '100' ? `\n![](${url})\n` : `\n<img src="${url}" width="${ratio}%">\n`);
   if (!document.querySelector('#editor-preview').hidden) renderEditorPreview(format);
  } catch (error) { toast(error.message); }
 };
 document.querySelector('#preview-toggle').onclick = () => {
  const box = document.querySelector('#editor-preview');
  box.hidden = !box.hidden;
  if (!box.hidden) { renderEditorPreview(format); document.querySelector('.editor-page')?.classList.add('previewing'); }
  else document.querySelector('.editor-page')?.classList.remove('previewing');
 };
 const persist = async (draft, button) => {
  const title = document.querySelector('#article-title').value.trim();
  const value = getEditorValue();
  const tags = document.querySelector('#article-tags')?.value.trim() || '';
  if (!title) { toast('先给文章起个标题'); return; }
  if (value.trim() === '') { toast('正文还空着呢'); return; }
  if (format === 'html' && content) sanitizeRichText(content);
  button.disabled = true;
  try {
   await api('save-article', {id: article2Id(), title, content: format === 'html' ? content.innerHTML : value, format, tags, draft: draft ? 1 : 0});
   await loadArticles(true);
   store.editingArticle = false;
   admin();
   toast(draft ? '草稿已保存' : '文章已发布');
  } catch (error) { toast(error.message); } finally { button.disabled = false; }
 };
 document.querySelector('#article-save').onclick = async e => persist(false, e.currentTarget);
 document.querySelector('#article-draft').onclick = async e => persist(true, e.currentTarget);
 const deleteButton = document.querySelector('#article-delete');
 if (deleteButton) deleteButton.onclick = () => {
  store.pendingArticleDelete = article2Id();
  const dialog = document.querySelector('#delete-dialog');
  dialog.querySelector('h2').textContent = '删除这篇文章？';
  dialog.querySelector('p').textContent = '删除后无法恢复，文章里的图片也会一并清理。';
  dialog.showModal();
 };
 function article2Id() { return store.editingArticle === 'new' ? null : store.editingArticle.id; }
}
