import { store } from './store.js';

// 静态烘焙页内联的数据块（<script type="application/json">）：省去首屏 fetch 往返；缺失/损坏时由调用方回退到远程加载
export function embeddedJson(id) {
 const el = document.getElementById(id);
 if (!el) return null;
 try { return JSON.parse(el.textContent); } catch { return null; }
}

export async function api(action, data) {
 const response = await fetch(`/api?action=${action}`, data === undefined ? {} : {method:'POST', headers:{'Content-Type':'application/json','X-CSRF-Token':store.state.csrf}, body:JSON.stringify(data)});
 const result = await response.json();
 if (!response.ok) throw new Error(result.error || '请求失败，请重试');
 if (action === 'posts') { store.postsNext = result.next_cursor; store.postsTotal = result.total; store.postsQuery = ''; }
 return result;
}

export async function morePosts() {
 if (!store.postsNext) return;
 const cursor = store.postsNext;
 const previous = store.posts;
 const result = await apiGet('posts', {cursor, q: store.postsQuery});
 if (cursor !== store.postsNext || store.posts !== previous) return;
 const known = new Set(store.posts.map(post => post.id));
 store.posts.push(...result.posts.filter(post => !known.has(post.id)));
 store.postsNext = result.next_cursor;
}

// GET 查询（公开数据端点，如插件 plugin-data）
export async function apiGet(action, params = {}, options = {}) {
 const qs = new URLSearchParams({ action, ...params });
 const response = await fetch('/api?' + qs.toString(), options);
 const result = await response.json();
 if (!response.ok) throw new Error(result.error || '请求失败，请重试');
 return result;
}
