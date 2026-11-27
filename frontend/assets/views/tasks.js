import { api, apiGet } from '../lib/api.js';
import { esc, toast } from '../lib/ui.js';
import { onViewLeave } from '../lib/lifecycle.js';

const states = {queued:'排队中',running:'执行中',succeeded:'已完成',failed:'失败',cancelled:'已取消',interrupted:'上次执行中断'};
const kinds = {image:'图片处理',plugin_sync:'插件同步',plugin_install:'插件安装',plugin_discover:'仓库扫描',export:'静态导出',deploy:'Netlify 部署',backup:'完整备份'};
const time = value => value ? new Date(value * 1000).toLocaleString('zh-CN', {hourCycle:'h23'}) : '暂无';

export function tasksHTML() {
 return '<section class="panel"><h2>后台任务</h2><p class="small-label" id="tasks-connection">正在连接…</p><div id="task-status" aria-live="polite">正在读取任务…</div></section><section class="panel" id="task-log-panel" hidden><div class="section-heading"><h2 id="task-log-title">任务日志</h2><button class="secondary" id="plugin-install-refresh" type="button" hidden>刷新页面使用插件</button></div><p class="small-label">显示最近 65,536 个字符。插件安装成功后刷新页面，在插件页启用和配置。</p><pre id="task-log-output" class="task-log-output"></pre></section><section class="panel"><div class="section-heading"><h2>定期备份设置</h2></div><p class="small-label">默认关闭。完整备份包含数据库、上传图片和插件声明的资源（含壁纸原图），可能占用较大磁盘空间。新备份成功后才清理超出保留数量的自动备份，手动备份不受影响。同盘备份不能代替异地备份。</p><div id="backup-settings"></div><div id="backup-files"></div></section>';
}
function statusHTML(jobs) {
 if (!jobs.length) return '<p>暂无任务。同步、导出和备份任务会在这里显示。</p>';
 return `<div class="task-table-wrap"><table class="task-table"><thead><tr><th>任务</th><th>状态</th><th>进度</th><th>结果</th><th>操作</th></tr></thead><tbody>${jobs.map(job => {
  const summary = job.result?.plugin_result?.summary;
  const state = job.state === 'succeeded' && summary?.failed > 0 ? (summary.succeeded > 0 ? '部分失败' : '失败') : states[job.state] || job.state;
  const result = summary ? `检查 ${summary.total} 个，成功 ${summary.succeeded} 个，失败 ${summary.failed} 个` : job.result?.plugin_result?.message;
  return `<tr><td>#${job.id} ${esc(kinds[job.kind] || job.kind)}</td><td>${esc(job.cancel_requested && job.state === 'running' ? '正在取消…' : state)}</td><td>${esc(job.phase)}${job.total === null ? '' : ` ${job.completed}/${job.total}`}</td><td>${esc(job.error || result || (job.result?.installed ? `已安装 ${job.result.id} ${job.result.version}` : null) || (job.result?.added !== undefined ? `新增 ${job.result.added} 条` : (job.result?.posts !== undefined ? `导出 ${job.result.posts} 条动态、${job.result.articles} 篇文章` : (job.result?.file ? `已备份 ${job.result.file}` : ''))))}</td><td>${['queued','running'].includes(job.state) ? `<button class="text-link" data-cancel-task="${job.id}" ${job.cancel_requested ? 'disabled' : ''}>取消</button>` : ''}${['plugin_install','plugin_sync'].includes(job.kind) ? `<button class="text-link" data-task-log="${job.id}">日志</button>` : ''}</td></tr>`; }).join('')}</tbody></table></div>`;
}
function backupFormHTML(backup) {
 return `<form id="scheduled-backup-form"><label class="switch-line"><input type="checkbox" name="enabled" ${backup.enabled ? 'checked' : ''}>启用定期备份</label><div class="options-grid"><label>间隔（小时）<input type="number" name="interval_hours" min="1" max="720" required value="${backup.interval_hours}"></label><label>保留份数<input type="number" name="keep" min="1" max="30" required value="${backup.keep}"></label></div><p class="small-label">下次执行：${esc(backup.next_run === null ? '未排期（保存并启用后开始）' : time(backup.next_run))}</p><div class="dialog-actions left"><button class="secondary" type="submit">保存备份设置</button></div></form>`;
}
function backupFilesHTML(backup) {
 const rows = (backup.files || []).map(file => `<tr><td>${esc(file.name)}</td><td>${(file.bytes / 1048576).toFixed(2)} MB</td><td>${esc(time(file.created_at))}</td></tr>`).join('');
 return rows ? `<div class="task-table-wrap"><table class="task-table"><thead><tr><th>自动备份文件</th><th>大小</th><th>时间</th></tr></thead><tbody>${rows}</tbody></table></div>` : '<p class="small-label">暂无自动备份</p>';
}
export function bindTasks() {
 const host = document.querySelector('#task-status');
 const connection = document.querySelector('#tasks-connection');
 const backupHost = document.querySelector('#backup-settings');
 const filesHost = document.querySelector('#backup-files');
 const controller = new AbortController();
 let logId = null;
 const logPanel = document.querySelector('#task-log-panel');
 const logOutput = document.querySelector('#task-log-output');
 const reload = document.querySelector('#plugin-install-refresh');
 reload.onclick = () => location.reload();
 const loadLog = async () => {
  const requested = logId;
  if (!requested) return;
  logPanel.hidden = false;
  document.querySelector('#task-log-title').textContent = `任务 #${requested} 日志`;
  try {
   const result = await apiGet('task-log', {id:requested}, {signal:controller.signal});
   if (logId === requested) logOutput.textContent = result.output || '暂无输出';
  } catch (error) { if (!controller.signal.aborted) logOutput.textContent = error.message; }
 };
 const render = data => {
  if (logId === null) logId = (data.jobs || []).find(job => job.kind === 'plugin_install')?.id || null;
  reload.hidden = !(data.jobs || []).some(job => job.id === logId && job.kind === 'plugin_install' && job.state === 'succeeded');
  loadLog();
  if (host.isConnected) host.innerHTML = statusHTML(data.jobs || []);
  // SSE 推送只带任务列表；备份面板数据来自完整查询（refresh）
  if (data.backup) {
   if (backupHost && !backupHost.querySelector('form')) {
    backupHost.innerHTML = backupFormHTML(data.backup);
    const form = backupHost.querySelector('form');
    form.onsubmit = async event => {
     event.preventDefault();
     const button = form.querySelector('button');
     button.disabled = true;
     try {
      await api('save-backup-settings', {enabled:form.elements.enabled.checked ? 1 : 0, interval_hours:Number(form.elements.interval_hours.value), keep:Number(form.elements.keep.value)});
      toast('备份设置已保存');
     } catch (error) { toast(error.message); }
     finally { button.disabled = false; }
    };
   }
   if (filesHost) filesHost.innerHTML = backupFilesHTML(data.backup);
  }
 };
 const refresh = () => apiGet('tasks', {}, {signal:controller.signal}).then(render).catch(error => {
  if (!controller.signal.aborted) connection.textContent = error.message;
 });
 host.onclick = async event => {
  const log = event.target.closest('[data-task-log]');
  if (log) { logId = Number(log.dataset.taskLog); await loadLog(); return; }
  const button = event.target.closest('[data-cancel-task]');
  if (!button) return;
  button.disabled = true;
  try { await api('cancel-task', {id:Number(button.dataset.cancelTask)}); await refresh(); }
  catch (error) { toast(error.message); button.disabled = false; }
 };
 const events = new EventSource('/api/tasks/events');
 events.addEventListener('tasks', event => render(JSON.parse(event.data)));
 events.onopen = () => { connection.textContent = '已连接，状态自动更新'; refresh(); };
 events.onerror = () => { connection.textContent = '连接中断，正在重新连接…'; };
 refresh();
 onViewLeave(() => { events.close(); controller.abort(); });
}
