// 文章标签输入：chips 化 —— 可从历史标签挑选，也可输入新标签回车成 chip；逗号键同样成 chip（不再手输分隔符）。
import { esc } from '../lib/ui.js';
import { toast } from '../lib/ui.js';

const MAX_TAGS = 5;

export function initTagInput({ initial = '', pool = [] } = {}) {
 const box = document.querySelector('#tag-box');
 const chipsEl = document.querySelector('#tag-chips');
 const field = document.querySelector('#tag-field');
 const suggest = document.querySelector('#tag-suggest');
 const hidden = document.querySelector('#article-tags');
 if (!box || !field || !hidden) return;
 let chips = initial.split(',').map(t => t.trim()).filter(Boolean).slice(0, MAX_TAGS);

 const sync = () => { hidden.value = chips.join(','); };
 const render = () => {
  chipsEl.innerHTML = chips.map((t, i) => `<span class="tag-chip">#${esc(t)}<button type="button" data-tag-remove="${i}" aria-label="移除标签 ${esc(t)}">×</button></span>`).join('');
  field.placeholder = chips.length ? '继续输入，回车添加' : '输入标签后回车（可选历史标签，最多 5 个）';
  sync();
 };
 const add = raw => {
  const t = raw.trim().replace(/[,，]/g, '').slice(0, 20);
  if (!t) return false;
  if (chips.length >= MAX_TAGS) { toast(`最多 ${MAX_TAGS} 个标签`); return false; }
  if (chips.some(c => c.toLowerCase() === t.toLowerCase())) { toast('这个标签已添加'); return false; }
  chips.push(t);
  render();
  return true;
 };
 const renderSuggest = () => {
  const q = field.value.trim().toLowerCase();
  const hits = [...new Set(pool)].filter(t => !chips.some(c => c.toLowerCase() === t.toLowerCase()) && (q === '' || t.toLowerCase().includes(q))).slice(0, 6);
  if (!hits.length) { suggest.hidden = true; suggest.innerHTML = ''; return; }
  suggest.innerHTML = hits.map(t => `<button type="button" data-tag-pick="${esc(t)}">#${esc(t)}</button>`).join('');
  suggest.hidden = false;
 };

 field.addEventListener('keydown', e => {
  if (e.key === 'Enter' || e.key === ',' || e.key === '，') { e.preventDefault(); if (add(field.value)) { field.value = ''; suggest.hidden = true; } }
  else if (e.key === 'Backspace' && field.value === '' && chips.length) { chips.pop(); render(); }
 });
 field.addEventListener('input', renderSuggest);
 field.addEventListener('focus', renderSuggest);
 field.addEventListener('blur', () => setTimeout(() => { suggest.hidden = true; }, 150));
 suggest.addEventListener('mousedown', e => {
  const btn = e.target.closest('[data-tag-pick]');
  if (!btn) return;
  e.preventDefault();
  add(btn.dataset.tagPick);
  field.value = '';
  suggest.hidden = true;
  field.focus();
 });
 chipsEl.addEventListener('click', e => {
  const btn = e.target.closest('[data-tag-remove]');
  if (!btn) return;
  chips.splice(Number(btn.dataset.tagRemove), 1);
  render();
 });

 render();
}
