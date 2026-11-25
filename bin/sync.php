<?php
declare(strict_types=1);

// PM2 使用 --watch 常驻运行；也支持 cron 单次执行或页面后台派发。
// 常驻进程缓存了启动时的插件注册表与核心代码：检测到磁盘代码变化即退出，由 PM2 autorestart 拉起新进程
// （否则新增/修改插件后 worker 仍跑旧代码，例如永不同步新插件、也谈不上触发自动部署）。
require_once __DIR__ . '/../app.php';
$watch = in_array('--watch', $argv, true);
session_write_close();
if ($watch) {
    $workerLock = fopen(DATA_DIR . '/worker.lock', 'c');
    if (!flock($workerLock, LOCK_EX | LOCK_NB)) exit(0);
}
$codeBaseline = code_fingerprint();
do {
    if ($watch && code_fingerprint() !== $codeBaseline) {
        echo date(DATE_ATOM) . " code changed, restarting via PM2\n";
        exit(0);
    }
    if ($watch) task_status_write('worker', ['pid' => getmypid(), 'checked_at' => time(), 'phase' => 'sync']);
    if (is_file(auto_deploy_pending_path()) && !auto_deploy_watcher_alive()) {
        deploy_spawn(__DIR__ . '/auto-deploy.php');
    }
    maybe_sync_plugins($db);
    if ($watch) task_status_write('worker', ['pid' => getmypid(), 'checked_at' => time(), 'phase' => 'backup']);
    maybe_scheduled_backup($db);
    if ($watch) task_status_write('worker', ['pid' => getmypid(), 'checked_at' => time(), 'phase' => 'waiting']);
    echo date(DATE_ATOM) . " plugin sync done\n";
    if ($watch) sleep(30);
} while ($watch);
