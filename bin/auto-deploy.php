<?php
// 自动部署 watcher：发布新动态时由 auto_deploy_touch() 派生，flock 保证单实例。
// 跟随 var/deploy.pending.json 的 due_at（期间再发动态会顺延），到期且无进行中部署时执行部署。
// 代码变化时退出（导出要用最新插件），待办仍在：worker 会用新代码重新拉起本 watcher。
require dirname(__DIR__) . '/app.php';
$lock = fopen(DATA_DIR . '/deploy.watcher.lock', 'c');
if (!$lock || !flock($lock, LOCK_EX | LOCK_NB)) exit(0); // 已有 watcher 在位，本进程退出
file_put_contents(DATA_DIR . '/deploy.watcher.pid', (string)getmypid());
$codeBaseline = code_fingerprint();
while (true) {
    if (code_fingerprint() !== $codeBaseline) { @unlink(DATA_DIR . '/deploy.watcher.pid'); exit(0); }
    $pending = auto_deploy_pending_read();
    $due = (int)($pending['due_at'] ?? 0);
    if ($due <= 0) break;                                        // 无待办（被手动部署/关闭开关清除）
    if (time() < $due) { sleep(min(30, $due - time())); continue; }
    if (opt('auto_deploy') !== '1') { auto_deploy_pending_clear($pending); break; }
    if (netlify_deploy_running()) { sleep(60); continue; }        // 手动部署在跑：等它结束再上
    if ((string)opt('netlify_token') === '' || (string)opt('netlify_site') === '') { auto_deploy_pending_clear($pending); break; }
    // 只清理本次处理的待办；部署期间的新更新继续按其到期时间执行。
    netlify_deploy_run($db, $pending);
}
@unlink(DATA_DIR . '/deploy.watcher.pid');
