import { store } from '../lib/store.js';
import { api } from '../lib/api.js';
import { icon, esc, toast, o, avatar } from '../lib/ui.js';
import { feed } from '../views/feed.js';
import { admin } from '../views/admin.js';

export function composer() { return `<form id="compose-form" class="composer">${avatar()}<div><label class="sr-only" for="compose-content">分享此刻的想法</label><textarea id="compose-content" name="content" placeholder="有什么新鲜事想要分享？" maxlength="${o('post_max_chars',500)}"></textarea><div id="compose-images" class="compose-images"></div><div class="composer-bottom"><span>${icon('sun')} 每个小瞬间，都值得被记录</span><div><button type="button" class="icon-button attach-button" id="attach-images" aria-label="添加图片">${icon('image')}</button><input type="file" id="compose-files" accept="image/jpeg,image/png,image/webp,image/gif,image/avif" multiple hidden><span id="char-count">0 / ${o('post_max_chars',500)}</span><button class="primary" type="submit">发布 ${icon('arrow')}</button></div></div></div></form>`; }

export function bindComposer() {
 const form=document.querySelector('#compose-form'); if(!form)return;
 // 草稿/图片存在时标记 has-content（展开逻辑由 CSS :focus-within + has-content 驱动）
 const syncOpen=()=>form.classList.toggle('has-content', form.content.value.trim()!=='' || form.querySelector('#compose-images').children.length>0);
 form.content.addEventListener('input', syncOpen);
 form.content.oninput=()=>document.querySelector('#char-count').textContent=`${Array.from(form.content.value).length} / ${o('post_max_chars',500)}`;
 const attach=form.querySelector('#attach-images'),input=form.querySelector('#compose-files');
 if(attach&&input)attach.onclick=()=>input.click();
 if(input)input.onchange=()=>{addComposeFiles(input.files);input.value='';};
 form.onsubmit=async e=>{e.preventDefault();const button=form.querySelector('button[type=submit]');button.disabled=true;try{
  const data=new FormData();data.set('content',form.content.value);
  for(const file of store.composeFiles)data.append('images[]',file);
  const response=await fetch('/api?action=save-post',{method:'POST',headers:{'X-CSRF-Token':store.state.csrf},body:data});
  const result=await response.json();if(!response.ok)throw new Error(result.error||'请求失败，请重试');
  store.composeFiles=[];store.posts=(await api('posts')).posts;store.page==='admin'?admin():feed();toast('这一刻，已记录');
 }catch(error){toast(error.message);}finally{button.disabled=false;}};
}

export function addComposeFiles(list) {
 for(const file of list) {
  const maxImages=o('post_max_images',4), maxMb=o('upload_max_mb',15);
  if(store.composeFiles.length>=maxImages){toast(`每条动态最多 ${maxImages} 张图片`);break;}
  if(file.size>maxMb*1024*1024){toast(`图片不能超过 ${maxMb}MB：`+file.name);continue;}
  store.composeFiles.push(file);
 }
 renderComposeImages();
}

export function renderComposeImages() {
 const box=document.querySelector('#compose-images');if(!box)return;
 box.innerHTML=store.composeFiles.map((file,i)=>`<div class="compose-image"><img src="${URL.createObjectURL(file)}" alt=""><button type="button" data-remove-file="${i}" aria-label="移除图片">${icon('close')}</button></div>`).join('');
 document.querySelector('#compose-form')?.classList.toggle('has-content', store.composeFiles.length>0);
}

export function renderEditImages() {
 const box=document.querySelector('#edit-images');if(!box)return;
 const post=store.posts.find(p=>String(p.id)===document.querySelector('#edit-form').elements.id.value);
 box.innerHTML=(post?.images||[]).filter(img=>img.ref&&store.editKeep.includes(img.ref)).map(img=>`<div class="compose-image"><img src="${esc(img.thumb)}" alt=""><button type="button" data-remove-keep="${esc(img.ref)}" aria-label="移除图片">${icon('close')}</button></div>`).join('');
}
