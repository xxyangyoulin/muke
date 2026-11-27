import { api, apiGet } from '../lib/api.js';
import { toast, esc } from '../lib/ui.js';
import { onViewLeave } from '../lib/lifecycle.js';

export function pluginInstallHTML() {
 return `<section class="panel"><div class="section-heading"><h2>安装独立插件</h2><button class="secondary" id="plugin-install-toggle" type="button">安装插件</button></div><form id="plugin-install-form" hidden><p class="small-label">安装会在服务机器上编译并执行仓库中的代码，请使用你信任的插件仓库。</p><p id="plugin-build-environment" class="small-label" role="status"></p><div class="options-grid"><label>来源<select name="source_kind"><option value="github">GitHub 仓库</option><option value="local">本地仓库</option></select></label><label><span id="plugin-source-label">GitHub 仓库地址</span><input name="source" type="text" required placeholder="https://github.com/作者/插件仓库"></label></div><p class="small-label" id="plugin-source-hint">支持公开 GitHub 仓库，编译默认分支。</p><button class="secondary" id="plugin-scan" type="button">扫描仓库</button><p id="plugin-scan-status" class="small-label" role="status"></p><label>插件子目录<select name="path"><option value="">仓库根目录</option></select></label><button class="primary" type="submit">编译并安装</button></form></section>`;
}

export function bindPluginInstall(onQueued) {
 const toggle = document.querySelector('#plugin-install-toggle');
 if (!toggle) return;
 const form = document.querySelector('#plugin-install-form');
 const controller = new AbortController();
 let events = null, scanned = '';
 onViewLeave(() => { controller.abort(); events?.close(); });
 const source = () => ({source_kind:form.elements.source_kind.value, source:form.elements.source.value.trim()});
 const identity = () => JSON.stringify(source());
 toggle.onclick = async () => {
  form.hidden = !form.hidden;
  if (form.hidden) return;
  const status = document.querySelector('#plugin-build-environment');
  status.textContent = '正在检查编译环境…';
  try {
   const tools = await apiGet('plugin-environment', {}, {signal:controller.signal});
   const missing = Object.entries(tools).filter(([, tool]) => !tool.available).map(([name]) => name);
   status.textContent = missing.length ? `缺少 ${missing.join('、')}，请先在服务机器上安装。` : 'Git、Rust/Cargo 和 C 编译器已就绪。';
   form.querySelector('[type=submit]').disabled = missing.length > 0;
  } catch (error) { if (!controller.signal.aborted) status.textContent = error.message; }
 };
 const reset = () => { scanned = ''; form.elements.path.innerHTML = '<option value="">仓库根目录</option>'; };
 form.elements.source.oninput = reset;
 form.elements.source_kind.onchange = () => {
  reset();
  const local = form.elements.source_kind.value === 'local';
  document.querySelector('#plugin-source-label').textContent = local ? '本地仓库路径' : 'GitHub 仓库地址';
  document.querySelector('#plugin-source-hint').textContent = local ? '填写 MUKE 服务机器上的绝对路径；包含未提交修改。' : '支持公开 GitHub 仓库，编译默认分支。';
  form.elements.source.placeholder = local ? '/home/用户名/my-plugin' : 'https://github.com/作者/插件仓库';
 };
 document.querySelector('#plugin-scan').onclick = async event => {
  if (!form.reportValidity()) return;
  const button = event.target, status = document.querySelector('#plugin-scan-status'), requested = identity();
  button.disabled = true; status.textContent = '正在获取仓库…';
  try {
   const result = await api('discover-plugins', source());
   const render = data => {
    const job = data.jobs.find(job => job.id === result.task_id);
    if (!job || controller.signal.aborted) return;
    status.textContent = `任务 #${job.id}：${job.phase || job.state}（可在任务页取消）`;
    if (['queued','running'].includes(job.state)) return;
    events?.close(); button.disabled = false;
    if (requested !== identity()) { status.textContent = '来源已改变，请重新扫描。'; return; }
    if (job.state !== 'succeeded') { status.textContent = job.error || '扫描已取消'; return; }
    scanned = requested;
    form.elements.path.innerHTML = job.result.plugins.map(p => `<option value="${esc(p.path)}">${esc(p.name)} ${esc(p.version)} · ${esc(p.path || '根目录')}</option>`).join('');
    status.textContent = `找到 ${job.result.plugins.length} 个插件，请选择后安装。`;
   };
   events?.close(); events = new EventSource('/api/tasks/events');
   events.addEventListener('tasks', event => render(JSON.parse(event.data)));
   render(await apiGet('tasks', {}, {signal:controller.signal}));
  } catch (error) { if (!controller.signal.aborted) status.textContent = error.message; button.disabled = false; }
 };
 form.onsubmit = async event => {
  event.preventDefault();
  const button = form.querySelector('[type=submit]'); button.disabled = true;
  try {
   const result = await api('install-plugin', {...source(), path:scanned === identity() ? form.elements.path.value : ''});
   toast(`已加入编译任务 #${result.task_id}`); onQueued();
  } catch (error) { toast(error.message); button.disabled = false; }
 };
}
