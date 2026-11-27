import { api, apiGet } from '../lib/api.js';
import { esc, toast } from '../lib/ui.js';
import { onViewLeave } from '../lib/lifecycle.js';

export function modelsHTML() {
 return '<section class="panel"><h2>模型服务</h2><p class="small-label">配置一次，供独立插件调用。API Key 留空保留原值。</p><div id="model-list">正在读取…</div><form id="model-form"><input name="id" type="hidden"><div class="options-grid"><label>名称<input name="name" required maxlength="120" placeholder="代码分析"></label><label>协议<select name="protocol"><option value="responses">OpenAI Responses</option><option value="chat_completions">OpenAI 兼容 Chat Completions</option><option value="anthropic">Anthropic Messages</option></select></label><label>接口基础地址<input name="base_url" required value="https://api.openai.com/v1"></label><label>模型名称<input name="model" required placeholder="填写服务商提供的模型名称"></label><label>API Key<input name="api_key" type="password" autocomplete="new-password"></label><label>输出上限（tokens）<input name="max_tokens" type="number" min="256" max="16384" value="2048" required></label></div><div class="dialog-actions left"><button class="primary" type="submit">保存配置</button><button class="secondary" type="reset">新增配置</button></div></form></section><section class="panel"><h2>最近调用</h2><div id="model-calls"></div></section>';
}

export function bindModels() {
 const form = document.querySelector('#model-form');
 if (!form) return;
 const controller = new AbortController();
 onViewLeave(() => controller.abort());
 let profiles = [];
 const refresh = async () => {
  const data = await apiGet('llm-profiles', {}, {signal:controller.signal});
  profiles = data.profiles;
  document.querySelector('#model-list').innerHTML = profiles.map(p => `<p><strong>${esc(p.name)}</strong> · ${esc(p.model)} <button type="button" class="text-link" data-model-edit="${p.id}">编辑</button> <button type="button" class="text-link" data-model-delete="${p.id}">删除</button></p>`).join('') || '<p class="small-label">还没有模型配置。</p>';
  document.querySelector('#model-calls').innerHTML = data.calls.length ? `<div class="task-table-wrap"><table class="task-table"><thead><tr><th>任务 / 插件</th><th>模型</th><th>状态</th><th>输入 / 输出 tokens</th></tr></thead><tbody>${data.calls.map(c => `<tr><td>#${c.task_id} ${esc(c.plugin)}</td><td>${esc(c.model)}</td><td>${esc(({running:'调用中',succeeded:'成功',failed:'失败',cancelled:'已取消',interrupted:'已中断'})[c.status] || c.status)}</td><td>${c.input_tokens ?? '—'} / ${c.output_tokens ?? '—'}</td></tr>`).join('')}</tbody></table></div>` : '<p class="small-label">暂无调用。</p>';
 };
 form.onsubmit = async event => {
  event.preventDefault();
  const button = form.querySelector('[type=submit]'); button.disabled = true;
  try {
   const values = Object.fromEntries(new FormData(form));
   values.id = values.id ? Number(values.id) : null;
   values.max_tokens = Number(values.max_tokens);
   const result = await api('save-llm-profile', values);
   form.elements.id.value = result.id; form.elements.api_key.value = '';
   await refresh(); toast('模型配置已保存');
  } catch (e) { if (!controller.signal.aborted) toast(e.message); }
  finally { button.disabled = false; }
 };
 form.onreset = () => { form.elements.api_key.placeholder = ''; };
 form.elements.protocol.onchange = () => {
  form.elements.base_url.value = form.elements.protocol.value === 'anthropic' ? 'https://api.anthropic.com/v1' : 'https://api.openai.com/v1';
 };
 document.querySelector('#model-list').onclick = async event => {
  const edit = event.target.closest('[data-model-edit]');
  if (edit) {
   const profile = profiles.find(p => p.id === Number(edit.dataset.modelEdit));
   for (const key of ['id','name','protocol','base_url','model','max_tokens']) form.elements[key].value = profile[key];
   form.elements.api_key.value = ''; form.elements.api_key.placeholder = '已保存，留空保留';
  }
  const remove = event.target.closest('[data-model-delete]');
  if (remove && confirm('删除这份模型配置？使用它的插件需要重新选择。')) {
   try { await api('delete-llm-profile', {id:Number(remove.dataset.modelDelete)}); form.reset(); await refresh(); }
   catch (e) { toast(e.message); }
  }
 };
 refresh().catch(e => { if (!controller.signal.aborted) toast(e.message); });
}
