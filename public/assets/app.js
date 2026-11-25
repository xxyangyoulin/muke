import { store } from './lib/store.js';
import { api, apiGet, embeddedJson } from './lib/api.js';
import { icon, esc, toast, empty, postItems, updateProfile } from './lib/ui.js';
import { openLightbox } from './components/lightbox.js';
import { renderComposeImages, renderEditImages } from './components/composer.js';
import { mountGiscus } from './components/giscus.js';
import { mountInlineComments } from './components/comments.js';
import { feed } from './views/feed.js';
import { loadArticles, articlesPage, articlePage } from './views/articles.js';
import { admin, loadPlugins } from './views/admin.js';
import { confirmEditorLeave, clearArticleDraft } from './components/editor-protection.js';
import { pushHistory, acceptHistoryNavigation } from './lib/navigation.js';

store.isStatic = document.documentElement.dataset.static === 'true';
store.main = document.querySelector('#main');
store.baseTitle = document.title;
try { store.likeToken = localStorage.getItem('muke-token') || ''; } catch { store.likeToken = ''; }
if (!store.likeToken) {
 store.likeToken = Array.from(crypto.getRandomValues(new Uint8Array(16))).map(b => b.toString(16).padStart(2, '0')).join('');
 try { localStorage.setItem('muke-token', store.likeToken); } catch {}
}
const pluginPages = window.MKBLOG._pages; // 插件注册的公开页面（plugin.js 各自注册）
// 静态页数据内联同步可得时，需要等本页其余模块脚本（插件的 registerCard/registerPage 等）执行完再渲染：
// setTimeout 宏任务排在所有已解析的模块任务之后，可保证注册完成（动态站走网络 await，天然满足，无需等待）
const modulesReady = () => new Promise(r => { if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', r, { once: true }); else setTimeout(r, 0); });

function renderPage() {
 if (store.page.startsWith('plugin:')) { const page = pluginPages['/' + store.page.slice(7)]; if (page) page.render(); else feed(); }
 else if (store.page === 'admin') admin();
 else if (store.page === 'articles') articlesPage();
 else if (store.page === 'article') articlePage(store.currentArticleId);
 else feed();
 const activeNav = store.page === 'article' ? 'articles' : store.page.startsWith('plugin:') ? store.page.slice(7) : store.page;
 document.querySelectorAll('nav>a').forEach(a => a.classList.toggle('active', a.dataset.nav === activeNav));
}
function goto(href, push = true) {
 let clean = href.replace(/\/index\.html$/, '');
 let path = clean.split('?')[0].replace(/\/+$/, '') || '/';
 let target = null, articleId = 0;
 const articleMatch = path.match(/^\/article\/(\d+)$/);
 if (path === '/') target = 'feed';
 else if (pluginPages[path]) target = 'plugin:' + path.slice(1);
 else if (path === '/articles') target = 'articles';
 else if (articleMatch) { target = 'article'; articleId = Number(articleMatch[1]); }
 else if (path === '/admin' && !store.isStatic) target = 'admin';
 if (!target) return false;
 const prevQuery = location.search.replace('?', '');
 const newQuery = clean.split('?')[1] || '';
 if (push && target === store.page && !(target === 'article' && articleId !== store.currentArticleId) && newQuery === prevQuery) { window.scrollTo({top: 0, behavior: 'smooth'}); return true; }
 if (push && !confirmEditorLeave()) return true;
 if (push) pushHistory(clean || '/');
 store.page = target;
 store.currentArticleId = articleId;
 renderPage();
 store.main.focus({preventScroll: true});
 window.scrollTo({top: 0});
 return true;
}
document.addEventListener('click',async e=>{
 const pin = e.target.closest('[data-pin]');
 if (pin) {
  const post = store.posts.find(p => p.id === Number(pin.dataset.pin));
  pin.disabled = true;
  try {
   await api('pin-post', {id:post.id, pinned:Number(post.pinned) ? 0 : 1});
   store.posts = (await api('posts')).posts;
   store.page === 'admin' ? admin() : feed();
   toast(Number(post.pinned) ? '已取消置顶' : '已置顶');
  } catch (error) { toast(error.message); pin.disabled = false; }
 }
 const close=e.target.closest('[data-close]');if(close)document.getElementById(close.dataset.close).close();
 const giscusLike = e.target.closest('[data-giscus-like]');
 if (giscusLike) {
  const id = Number(giscusLike.dataset.giscusLike);
  const slot = document.querySelector(`[data-comments-slot="${id}"]`);
  if (slot) {
   slot.hidden = false;
   if (!slot.firstChild) mountGiscus('post', id, slot, store.state.options);
   slot.scrollIntoView({block:'nearest', behavior:'smooth'});
   toast('在下方选择 ❤️，通过 GitHub 同步点赞');
  }
 }
 const like=e.target.closest('[data-like]');if(like){const id=Number(like.dataset.like);const was=store.likedIds.has(id);like.disabled=true;like.classList.toggle('liked',!was);like.querySelector('span').textContent=!was?'已点赞':'点赞';api('like',{id,token:store.likeToken}).then(r=>{store.likedIds.has(id)?store.likedIds.delete(id):store.likedIds.add(id);const small=like.querySelector('small')||like.appendChild(document.createElement('small'));small.textContent=r.count>0?` ${r.count}`:'';}).catch(()=>{like.classList.toggle('liked',was);like.querySelector('span').textContent=was?'已点赞':'点赞';toast('点赞失败，稍后再试');}).finally(()=>{like.disabled=false;});}
 const share=e.target.closest('[data-share]');if(share){const url=location.origin+'/#post-'+share.dataset.share;try{await navigator.clipboard.writeText(url);toast('动态链接已复制');}catch{toast('请复制链接：'+url);}}
 const edit=e.target.closest('[data-edit]');if(edit){const p=store.posts.find(p=>p.id===Number(edit.dataset.edit));const form=document.querySelector('#edit-form');form.elements.content.value=p.content;form.elements.id.value=p.id;store.editKeep=(p.images||[]).map(img=>img.ref).filter(Boolean);renderEditImages();document.querySelector('#edit-dialog').showModal();}
 const removeFile=e.target.closest('[data-remove-file]');if(removeFile){store.composeFiles.splice(Number(removeFile.dataset.removeFile),1);renderComposeImages();}
 const removeKeep=e.target.closest('[data-remove-keep]');if(removeKeep){store.editKeep=store.editKeep.filter(ref=>ref!==removeKeep.dataset.removeKeep);renderEditImages();}
 const shareArticle=e.target.closest('[data-share-article]');if(shareArticle){const url=location.origin+'/article/'+shareArticle.dataset.shareArticle;try{await navigator.clipboard.writeText(url);toast('文章链接已复制');}catch{toast('请复制链接：'+url);}}
 const media=e.target.closest('[data-media-post]');if(media){const post=store.posts.find(p=>p.id===Number(media.dataset.mediaPost));if(post?.images?.length){e.preventDefault();openLightbox(postItems(post.images),Number(media.dataset.mediaIndex),media.querySelector('img'));}}
 const postComments=e.target.closest('[data-post-comments]');if(postComments){const pid=Number(postComments.dataset.postComments);const slot=document.querySelector(`[data-comments-slot="${pid}"]`);if(slot){if(!slot.hidden){slot.hidden=true;}else{slot.hidden=false;if(!slot.firstChild){const post=store.posts.find(p=>p.id===pid);if(post)mountInlineComments(post,slot);}slot.scrollIntoView({block:'nearest',behavior:'smooth'});}}}
 const del=e.target.closest('[data-delete]');if(del){store.pendingDelete=Number(del.dataset.delete);const d2=document.querySelector('#delete-dialog');d2.querySelector('h2').textContent='删除这条动态？';d2.querySelector('p').textContent='删除后无法恢复。';d2.showModal();}
 const editArticle=e.target.closest('[data-edit-article]');if(editArticle){const id=Number(editArticle.dataset.editArticle);try{const a=(await apiGet('articles',{id})).article;store.editingArticle=a;store.page='admin';admin();}catch(error){toast(error.message);}}
 const delArticle=e.target.closest('[data-delete-article]');if(delArticle){store.pendingArticleDelete=Number(delArticle.dataset.deleteArticle);const dialog=document.querySelector('#delete-dialog');dialog.querySelector('h2').textContent='删除这篇文章？';dialog.querySelector('p').textContent='删除后无法恢复，修订历史及不再使用的图片也会一并清理。';dialog.showModal();}
 const articleImg=e.target.closest('.article-content img');if(articleImg){const container=articleImg.closest('.article-content');const imgs=[...container.querySelectorAll('img')];const items=imgs.map(i=>({src:i.currentSrc||i.src,width:i.naturalWidth||1600,height:i.naturalHeight||1200,downloadUrl:(i.currentSrc||i.src)+(i.src.includes('?')?'&download=1':''),caption:''}));e.preventDefault();openLightbox(items,imgs.indexOf(articleImg),articleImg);}
});
document.querySelector('#confirm-plugin-remove').onclick=async()=>{
 const id=store.pendingPluginRemove;
 if(!id)return;
 try{
  await api('remove-plugin',{id,purge:document.querySelector('#plugin-remove-purge').checked});
  store.pendingPluginRemove=null;
  document.querySelector('#plugin-remove-dialog').close();
  await loadPlugins();
  admin();
  toast('插件已移除');
 }catch(e){toast(e.message);}
};
document.querySelector('#confirm-delete').onclick=async()=>{
 const dialog=document.querySelector('#delete-dialog');
 if(store.pendingPluginPurge){const id=store.pendingPluginPurge;store.pendingPluginPurge=null;dialog.close();try{await api('purge-plugin-posts',{id});await loadPlugins();admin();toast('已清空该插件的动态');}catch(e){toast(e.message);}return;}
 if(store.pendingArticleDelete){try{await api('delete-article',{id:store.pendingArticleDelete});clearArticleDraft(store.pendingArticleDelete);await loadArticles(true);if(store.editingArticle!==false&&store.editingArticle.id===store.pendingArticleDelete)store.editingArticle=false;store.pendingArticleDelete=0;dialog.close();admin();toast('文章已删除');}catch(e){toast(e.message);}return;}
 try{await api('delete-post',{id:store.pendingDelete});store.posts=(await api('posts')).posts;dialog.close();store.page==='admin'?admin():feed();toast('动态已删除');}catch(e){toast(e.message);}};
document.querySelector('#edit-form').onsubmit=async e=>{e.preventDefault();try{
 const data=new FormData(e.target);data.set('keep',JSON.stringify(store.editKeep));
 const response=await fetch('/api?action=save-post',{method:'POST',headers:{'X-CSRF-Token':store.state.csrf},body:data});
 const result=await response.json();if(!response.ok)throw new Error(result.error||'请求失败，请重试');
 store.posts=(await api('posts')).posts;document.querySelector('#edit-dialog').close();store.page==='admin'?admin():feed();toast('修改已保存');
}catch(error){toast(error.message);}};
document.querySelectorAll('dialog').forEach(d=>d.addEventListener('click',e=>{if(e.target===d){const r=d.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)d.close();}}));
if(!store.isStatic)document.querySelector('#new-post').onclick=()=>{
 if (document.querySelector('.editor-page')) {
  if (!confirmEditorLeave()) return;
  store.editingArticle = false; store.page = 'admin'; store.adminTab = 'overview'; admin();
 }
 if(store.state?.admin){if(store.page==='feed'||store.page==='admin')document.querySelector('#compose-content')?.focus();else goto('/admin');}else goto('/admin');
};
document.querySelectorAll('[data-icon]').forEach(el=>el.innerHTML=icon(el.dataset.icon));
async function init(){
 try{
  if(store.isStatic){const d=embeddedJson('embedded-content')??await(await fetch('/assets/content.json').then(r=>{if(!r.ok)throw new Error('静态数据加载失败');return r.json();}));store.state=d.state;store.posts=d.posts;store.pluginsData=d.plugins||{};store.pluginsMeta=d.plugins_meta||{};}
  else{const results=await Promise.all([api('state'),api('posts')]);store.state=results[0];store.posts=results[1].posts;}
  if (!store.isStatic && store.state.options?.comment_provider !== 'giscus') { try { const lk = await apiGet('likes', { token: store.likeToken }); store.likedIds = new Set(lk.liked); } catch {} }
  if (store.isStatic) { await modulesReady(); await loadArticles().catch(() => {}); } // 静态：文章一并就绪，首帧渲染即完整结构（只出现一次「加载→内容」切换，无结构闪变）
  updateProfile();
  document.dispatchEvent(new CustomEvent('mkblog:data')); // 插件模块（如壁纸）由此加载数据、启动定时器
  if (!store.isStatic) loadArticles().then(() => {
    // 文章加载完成的重渲染会清掉发布框草稿/聚焦 —— 发布框有内容时跳过（数据下次渲染自然带上）
    const draft = document.querySelector('#compose-content');
    if (store.page === 'feed' && !(draft && (draft.value.trim() || document.activeElement === draft))) feed();
  }).catch(() => {});
  let route = location.pathname.replace(/\/+$/, '').replace(/\/index\.html$/, '') || '/';
  const articleMatch = route.match(/^\/article\/(\d+)$/);
  if (pluginPages[route]) store.page = 'plugin:' + route.slice(1);
  else if (route === '/articles') store.page = 'articles';
  else if (articleMatch) { store.page = 'article'; store.currentArticleId = Number(articleMatch[1]); }
  else if (route === '/admin' && !store.isStatic) store.page = 'admin';
  else store.page = 'feed';
  renderPage();
  if(location.hash.startsWith('#post-'))document.getElementById(location.hash.slice(1))?.scrollIntoView();
 }catch(e){store.main.innerHTML=empty('加载失败，请稍后重试',esc(e.message));}
}
init();
// 单页应用导航：站内链接客户端渲染，浏览器前进后退同步
document.addEventListener('click', e => {
 if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
 const link = e.target.closest('a[href]');
 if (!link || link.target || link.download || link.host !== location.host) return;
 if (link.pathname.replace(/\/+$/, '') === location.pathname.replace(/\/+$/, '') && link.search === location.search) return;
 if (goto(link.getAttribute('href'))) e.preventDefault();
});
window.addEventListener('popstate', () => {
 if (acceptHistoryNavigation(confirmEditorLeave)) goto(location.pathname + location.search, false);
});
