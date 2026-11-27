// 豆瓣动态 · 前端卡片渲染器（movie）
// 与 plugin.php 的 render_post 成对：无 JS 时由静态烘焙版本呈现。
// 通过 MKBLOG.registerCard 注册，meta.card = 'movie' 的动态会使用本卡片。
MKBLOG.registerCard('movie', post => {
 const m = post.meta || {};
 const cover = (post.images && post.images[0]) || null;
 const e = MKBLOG.esc;
 return `<a class="post-movie-card" href="${e(post.link || '#')}" ${post.link ? 'target="_blank" rel="noopener"' : ''}>${cover ? `<img src="${e(cover.thumb || cover.url)}" alt="${e(m.title || '')}" loading="lazy">` : ''}<div class="post-movie-info"><strong>${e(m.title || '')}</strong>${m.intro ? `<span class="movie-intro">${e(m.intro)}</span>` : ''}<div class="movie-meta">${m.rating ? `<span class="movie-stars">${e(m.rating)}</span>` : ''}${m.date ? `<span class="movie-date">${e(m.date)} ${e(m.category || '')}</span>` : ''}</div>${m.comment ? `<span class="movie-comment">“${e(m.comment)}”</span>` : ''}</div></a>`;
});
