import { toast } from '../lib/ui.js';

export let lbItems = [], lbIndex = 0, lbScale = 1, lbPan = {x: 0, y: 0}, lbDragging = null, lbSwipe = null, lbCurrentLayer = null, lbToken = 0, lbSourceRect = null;

export const lightboxEl = () => document.querySelector('#lightbox');

export const lbImage = () => lbCurrentLayer?.querySelector('img');

export const lbPointers = new Map();

export let lbPinch = null;

export const lbPointerEnd = e => {
 lbPointers.delete(e.pointerId);
 if (lbPointers.size < 2) lbPinch = null;
 if (lbPointers.size > 0) { lbSwipe = null; lbDragging = null; return; }
 const box = lightboxEl();
 if (!box.hidden && lbSwipe && lbSwipe.moved && lbScale <= 1.001 && Math.abs(e.clientX - lbSwipe.x) > 48 && Math.abs(e.clientX - lbSwipe.x) > Math.abs(e.clientY - lbSwipe.y)) {
  showLightbox(lbIndex + (e.clientX < lbSwipe.x ? 1 : -1), e.clientX < lbSwipe.x ? 1 : -1);
 }
 lbSwipe = null;
 lbDragging = null;
};

export function lbLockScroll() { document.documentElement.classList.add('lb-lock'); }

export function lbUnlockScroll() { document.documentElement.classList.remove('lb-lock'); }

export function openLightbox(items, index, sourceEl) {
 if (!Array.isArray(items) || !items.length || index < 0 || index >= items.length) return;
 lbItems = items;
 lbSourceRect = sourceEl?.getBoundingClientRect() || null;
 lbLockScroll();
 lightboxEl().hidden = false;
 requestAnimationFrame(() => lightboxEl().classList.add('open'));
 showLightbox(index, 0);
}

export function showLightbox(index, direction = 1) {
 lbIndex = (index + lbItems.length) % lbItems.length;
 lbScale = 1;
 lbPan = {x: 0, y: 0};
 const item = lbItems[lbIndex];
 const box = lightboxEl();
 const stage = box.querySelector('.lightbox-stage');
 // 旧图层沿反方向退场
 if (lbCurrentLayer) {
  const outgoing = lbCurrentLayer;
  outgoing.classList.remove('current', 'enter-next', 'enter-prev');
  outgoing.classList.add(direction >= 0 ? 'leave-next' : 'leave-prev');
  setTimeout(() => outgoing.remove(), 380);
  lbCurrentLayer = null;
 }
 box.querySelector('#lb-counter').textContent = `${lbIndex + 1} / ${lbItems.length}`;
 box.querySelector('#lb-caption').innerHTML = item.caption || '';
 const dl = box.querySelector('#lb-download');
 dl.href = item.downloadUrl || item.src;
 dl.setAttribute('download', (item.downloadName || 'image').replace(/[\\/:*?"<>|]/g, '_'));
 box.classList.toggle('single', lbItems.length === 1);
 // 新图层：先摆到入场位，加载完成后滑入
 const layer = document.createElement('div');
 layer.className = 'lb-layer' + (direction === 0 ? '' : direction > 0 ? ' enter-next' : ' enter-prev');
 const img = document.createElement('img');
 img.alt = item.downloadName || '图片';
 layer.appendChild(img);
 stage.appendChild(layer);
 const token = ++lbToken;
 box.classList.add('loading');
 const settle = () => {
  if (token !== lbToken) { layer.remove(); return; }
  lbCurrentLayer = layer;
  img.removeAttribute('width');
  img.removeAttribute('height');
  img.style.aspectRatio = '';
  // 从点击的缩略图位置展开（FLIP）：初始定位到缩略图，再过渡回最终居中位置
  if (direction === 0 && lbSourceRect) {
   const final = img.getBoundingClientRect();
   const k = lbSourceRect.width / final.width;
   const dx = lbSourceRect.left + lbSourceRect.width / 2 - (final.left + final.width / 2);
   const dy = lbSourceRect.top + lbSourceRect.height / 2 - (final.top + final.height / 2);
   layer.style.transform = `translate(${dx}px, ${dy}px) scale(${k})`;
  }
  layer.getBoundingClientRect(); // 先让入场位移生效，再加过渡进入
  layer.classList.add('current');
  if (direction === 0) layer.style.transform = '';
  layer.classList.remove('enter-next', 'enter-prev');
  box.classList.remove('loading');
  applyZoom();
  // 预加载相邻图片，翻页即可秒开
  [lbIndex + 1, lbIndex - 1].forEach(i => {
   const neighbor = lbItems[(i + lbItems.length) % lbItems.length];
   if (neighbor && !neighbor._prefetched) { const pre = new Image(); pre.src = neighbor.src; neighbor._prefetched = true; }
  });
 };
 img.onload = settle;
 img.onerror = () => { if (token === lbToken) { box.classList.remove('loading'); toast('图片加载失败'); } };
 if (item.width) img.style.aspectRatio = `${item.width} / ${item.height}`;
 img.src = item.src;
 if (img.complete && img.naturalWidth) settle();
}

export function closeLightbox() {
 const box = lightboxEl();
 box.classList.remove('open');
 lbUnlockScroll();
 lbToken++;
 // 收起错觉：当前图层缩回缩略图位置并淡出
 if (lbCurrentLayer && lbSourceRect) {
  const layer = lbCurrentLayer;
  const img = layer.querySelector('img');
  if (img) {
   const final = img.getBoundingClientRect();
   const k = lbSourceRect.width / final.width;
   const dx = lbSourceRect.left + lbSourceRect.width / 2 - (final.left + final.width / 2);
   const dy = lbSourceRect.top + lbSourceRect.height / 2 - (final.top + final.height / 2);
   layer.style.transition = 'transform .3s cubic-bezier(.3,.7,.3,1), opacity .28s ease';
   layer.getBoundingClientRect();
   layer.style.transform = `translate(${dx}px, ${dy}px) scale(${k})`;
   layer.style.opacity = '0';
  }
 }
 setTimeout(() => {
  if (box.classList.contains('open')) return;
  box.hidden = true;
  box.querySelectorAll('.lb-layer').forEach(l => l.remove());
  lbCurrentLayer = null;
  lbSourceRect = null;
 }, 320);
}

export function applyZoom() {
 const img = lbImage();
 if (!img) return;
 if (lbScale <= 1.001) lbPan = {x: 0, y: 0};
 else {
  // 拖拽边界：任何方向至少保留一半图片在视野内
  const maxX = img.offsetWidth * (lbScale - 1) / 2;
  const maxY = img.offsetHeight * (lbScale - 1) / 2;
  lbPan = {x: Math.min(maxX, Math.max(-maxX, lbPan.x)), y: Math.min(maxY, Math.max(-maxY, lbPan.y))};
 }
 const zoomed = lbScale > 1.001;
 img.classList.toggle('zoomed', zoomed);
 img.style.transform = zoomed ? `translate(${lbPan.x}px, ${lbPan.y}px) scale(${lbScale})` : '';
 lightboxEl().querySelector('#lb-zoom').classList.toggle('active', zoomed);
}


// ---- 灯箱接线（事件注册，模块私有状态在作用域内）----
document.addEventListener('click', e => {
 const box = lightboxEl();
 if (box.hidden || !(e.target instanceof Element)) return;
 if (e.target.closest('[data-lb-close]')) { closeLightbox(); return; }
 if (e.target.closest('[data-lb-prev]')) { showLightbox(lbIndex - 1, -1); return; }
 if (e.target.closest('[data-lb-next]')) { showLightbox(lbIndex + 1, 1); return; }
 if (e.target.closest('#lb-zoom')) {
  if (lbScale > 1.001) { lbScale = 1; lbPan = {x: 0, y: 0}; }
  else lbScale = 2.2;
  applyZoom();
  return;
 }
 if (e.target === lbImage() && lbScale <= 1.001) { closeLightbox(); return; }
 if (e.target.classList?.contains('lightbox-backdrop')) closeLightbox();
});

document.addEventListener('wheel', e => {
 const box = lightboxEl();
 if (box.hidden || !lbCurrentLayer || !e.deltaY) return;
 e.preventDefault();
 const img = lbImage();
 if (!img) return;
 const rect = img.getBoundingClientRect();
 const mx = e.clientX - (rect.left + rect.width / 2);
 const my = e.clientY - (rect.top + rect.height / 2);
 const next = Math.min(5, Math.max(1, lbScale * Math.exp(-e.deltaY * 0.0016)));
 const ratio = next / lbScale;
 lbPan = {x: lbPan.x + mx * (1 - ratio), y: lbPan.y + my * (1 - ratio)};
 lbScale = next;
 if (lbScale <= 1.001) { lbScale = 1; lbPan = {x: 0, y: 0}; }
 applyZoom();
}, {passive: false});

document.addEventListener('keydown', e => {
 const box = lightboxEl();
 if (box.hidden) return;
 if (e.key === 'Escape') closeLightbox();
 else if (e.key === 'ArrowLeft') showLightbox(lbIndex - 1, -1);
 else if (e.key === 'ArrowRight') showLightbox(lbIndex + 1, 1);
});

document.addEventListener('pointerdown', e => {
 const box = lightboxEl();
 if (box.hidden) return;
 const targetEl = e.target instanceof Element ? e.target : null;
 if (targetEl?.closest('.lb-arrow, .lightbox-topbar, .lb-caption')) return;
 if (e.pointerType === 'touch') {
  lbPointers.set(e.pointerId, {x: e.clientX, y: e.clientY});
  if (lbPointers.size === 2) {
   const [p1, p2] = [...lbPointers.values()];
   const img = lbImage();
   if (img) {
    const rect = img.getBoundingClientRect();
    lbPinch = {
     dist: Math.hypot(p1.x - p2.x, p1.y - p2.y) || 1,
     mid: {x: (p1.x + p2.x) / 2, y: (p1.y + p2.y) / 2},
     scale: lbScale,
     pan: {...lbPan},
     origin: {x: rect.left + rect.width / 2 - lbPan.x, y: rect.top + rect.height / 2 - lbPan.y},
    };
   }
   lbSwipe = null;
   lbDragging = null;
   return;
  }
 }
 lbSwipe = {x: e.clientX, y: e.clientY, moved: false};
 if (lbScale > 1.001) lbDragging = {x: e.clientX, y: e.clientY, sx: lbPan.x, sy: lbPan.y};
});

document.addEventListener('pointermove', e => {
 if (lbPointers.has(e.pointerId)) {
  lbPointers.set(e.pointerId, {x: e.clientX, y: e.clientY});
  if (lbPinch && lbPointers.size === 2) {
   const [p1, p2] = [...lbPointers.values()];
   const dist = Math.hypot(p1.x - p2.x, p1.y - p2.y) || 1;
   const mid = {x: (p1.x + p2.x) / 2, y: (p1.y + p2.y) / 2};
   const next = Math.min(5, Math.max(1, lbPinch.scale * dist / lbPinch.dist));
   const ratio = next / lbPinch.scale;
   // 以捏合起点中点为锚，跟随中点移动平移
   lbPan = {
    x: mid.x - lbPinch.origin.x - ratio * (lbPinch.mid.x - lbPinch.origin.x - lbPinch.pan.x),
    y: mid.y - lbPinch.origin.y - ratio * (lbPinch.mid.y - lbPinch.origin.y - lbPinch.pan.y),
   };
   lbScale = next;
   applyZoom();
   return;
  }
 }
 if (lbSwipe && (Math.abs(e.clientX - lbSwipe.x) > 6 || Math.abs(e.clientY - lbSwipe.y) > 6)) lbSwipe.moved = true;
 if (lbDragging) {
  lbPan = {x: lbDragging.sx + e.clientX - lbDragging.x, y: lbDragging.sy + e.clientY - lbDragging.y};
  applyZoom();
 }
});

document.addEventListener('pointerup', lbPointerEnd);

document.addEventListener('pointercancel', lbPointerEnd);
