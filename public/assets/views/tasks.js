import { api, apiGet } from '../lib/api.js';
import { esc, toast } from '../lib/ui.js';
import { onViewLeave } from '../lib/lifecycle.js';

const time = value => value ? new Date(value * 1000).toLocaleString('zh-CN', {hourCycle:'h23'}) : '暂无';
const next = value => value === null ? '未排期' : value <= Date.now() / 1000 ? '已到期，等待后台检查' : time(value);
const phases = {sync:'检查插件同步', backup:'检查定期备份', waiting:'等待下轮检查', exporting:'导出中', uploading:'上传中', waiting_deploy:'等待部署', cancelled:'已取消', done:'已完成', error:'失败', running:'执行中', interrupted:'上次执行已中断'};

export function tasksHTML() {
 return '<div id="task-status" aria-live="polite"><div class="panel">正在读取后台任务…</div></div><div class="panel"><div class="section-heading"><h2>定期备份设置</h2></div><p class="small-label">默认关闭。完整备份包含数据库、上传图片和插件声明的资源（含壁纸原图），可能占用较大磁盘空间。新备份成功后才清理超出保留数量的自动备份，手动备份不受影响。同盘备份不能代替异地备份。</p><div id="backup-settings"></div></div>';
}

function statusHTML(data) {
 const {worker, plugins, deploy, backup} = data;
 const status = deploy.status;
 return `<section class="panel"><div class="section-heading"><h2>后台服务</h2><span>${worker.running ? '运行中' : '未检测到常驻进程'}</span></div><p>${worker.running ? esc(phases[worker.status.phase] || '运行中') : '请检查 PM2 中的 mkblog-sync；仅使用 cron 时不会显示常驻运行。'}</p><p class="small-label">最近检查：${esc(time(worker.status.checked_at))} · 本页每 5 秒刷新</p></section>
 <section class="panel"><div class="section-heading"><h2>插件同步</h2><span>${data.sync_running ? '同步执行中' : '空闲'}</span></div><div class="task-table-wrap"><table class="task-table"><thead><tr><th>插件</th><th>上次尝试</th><th>下次同步</th><th>最近错误</th></tr></thead><tbody>${plugins.map(plugin => `<tr><td>${esc(plugin.name)}${plugin.enabled ? '' : '（停用）'}</td><td>${esc(time(plugin.last_sync))}</td><td>${esc(plugin.next_sync === null ? (plugin.enabled ? '仅手动同步' : '未启用') : next(plugin.next_sync))}</td><td>${esc(plugin.error || '无')}</td></tr>`).join('')}</tbody></table></div></section>
 <section class="panel"><div class="section-heading"><h2>部署任务</h2><span>${esc(deploy.running ? (status.state === 'waiting' ? '等待部署完成' : phases[status.state] || '运行中') : phases[status.state] || '暂无记录')}</span></div>${deploy.running ? '<button type="button" class="secondary" data-cancel-deploy>取消部署</button>' : ''}<p>最近更新：${esc(time(status.updated_at))}${status.files_total ? ` · 文件 ${Number(status.files_done || 0)}/${Number(status.files_total)}` : ''}</p><p>自动部署排期：${esc(deploy.pending.due_at ? time(deploy.pending.due_at) : '无待办')}</p>${status.error ? `<p class="form-error">${esc(status.error)}</p>` : ''}</section>
 <section class="panel"><div class="section-heading"><h2>定期备份</h2><span>${esc(backup.running ? '执行中' : phases[backup.state] || '尚未执行')}</span></div><p>上次成功：${esc(time(backup.last_success))} · 下次执行：${esc(next(backup.next_run))}</p><p class="small-label">保存目录：${esc(backup.directory)} · ${backup.enabled ? '已启用' : '已关闭'}</p>${backup.error ? `<p class="form-error">${esc(backup.error)}</p>` : ''}<div class="task-table-wrap"><table class="task-table"><thead><tr><th>自动备份文件</th><th>大小</th><th>时间</th></tr></thead><tbody>${backup.files.map(file => `<tr><td>${esc(file.name)}</td><td>${(file.bytes / 1048576).toFixed(2)} MB</td><td>${esc(time(file.created_at))}</td></tr>`).join('')}</tbody></table></div>${backup.files.length ? '' : '<p class="small-label">暂无自动备份</p>'}</section>`;
}

export function bindTasks() {
 const host = document.querySelector('#task-status');
 const settings = document.querySelector('#backup-settings');
 const controller = new AbortController();
 let busy = false;
 const refresh = async () => {
  if (busy) return;
  busy = true;
  try {
   const data = await apiGet('tasks', {}, {signal:controller.signal});
   if (!host.isConnected) return;
   host.innerHTML = statusHTML(data);
   const cancel = host.querySelector('[data-cancel-deploy]');
   if (cancel && data.deploy.cancelling) { cancel.disabled = true; cancel.textContent = '正在取消…'; }
   if (cancel) cancel.onclick = async () => {
    cancel.disabled = true;
    try { await api('cancel-deploy', {}); cancel.textContent = '正在取消…'; }
    catch (error) { toast(error.message); cancel.disabled = false; }
   };
   if (!settings.firstElementChild) {
    const backup = data.backup;
    settings.innerHTML = `<form id="scheduled-backup-form"><label class="switch-line"><input type="checkbox" name="enabled" ${backup.enabled ? 'checked' : ''}>启用定期备份</label><div class="options-grid"><label>间隔（小时）<input type="number" name="interval_hours" min="1" max="720" required value="${backup.interval_hours}"></label><label>保留份数<input type="number" name="keep" min="1" max="30" required value="${backup.keep}"></label></div><div class="dialog-actions left"><button class="secondary" type="submit">保存备份设置</button></div></form>`;
    settings.firstElementChild.onsubmit = async event => {
     event.preventDefault();
     const form = event.target;
     const button = form.querySelector('button');
     button.disabled = true;
     try {
      await api('save-backup-settings', {enabled:form.elements.enabled.checked ? 1 : 0, interval_hours:Number(form.elements.interval_hours.value), keep:Number(form.elements.keep.value)});
      toast('备份设置已保存');
      await refresh();
     } catch (error) { toast(error.message); }
     finally { button.disabled = false; }
    };
   }
  } catch (error) {
   if (!controller.signal.aborted && host.isConnected) host.innerHTML = `<div class="panel form-error">读取任务状态失败：${esc(error.message)}</div>`;
  } finally { busy = false; }
 };
 refresh();
 const timer = setInterval(refresh, 5000);
 onViewLeave(() => { clearInterval(timer); controller.abort(); });
}
