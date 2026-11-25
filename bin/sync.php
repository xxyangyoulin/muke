<?php
declare(strict_types=1);

// PM2 使用 --watch 常驻运行；也支持 cron 单次执行或页面后台派发。
require_once __DIR__ . '/../app.php';
$watch = in_array('--watch', $argv, true);
do {
    if (is_file(auto_deploy_pending_path()) && !auto_deploy_watcher_alive()) {
        deploy_spawn(__DIR__ . '/auto-deploy.php');
    }
    maybe_sync_plugins($db);
    echo date(DATE_ATOM) . " plugin sync done\n";
    if ($watch) sleep(30);
} while ($watch);
