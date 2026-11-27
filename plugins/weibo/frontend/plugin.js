// 微博动态 · 前端卡片渲染器（weibo）—— 日期/原文链接由外层动态框架提供，卡片内不再重复
MKBLOG.registerCard('weibo', post => {
 const m = post.meta || {};
 const e = MKBLOG.esc;
 const covers = (post.images || []).slice(0, 3);
 // 封面用 data-media-post/index 挂到全局灯箱委托（app.js）：点击预览该动态全部图片（含第 4 张）
 return `<div class="post-weibo-card"><p class="weibo-text"><span class="weibo-brand">微博</span>${e(m.text || post.content)}</p>${covers.length ? `<div class="weibo-grid count${covers.length}">${covers.map((i, idx) => `<button class="post-image" type="button" data-media-post="${post.id}" data-media-index="${idx}" aria-label="查看图片"><img src="${e(i.thumb || i.url)}" alt="" loading="lazy"></button>`).join('')}</div>` : ''}</div>`;
});
