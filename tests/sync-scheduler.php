<?php
declare(strict_types=1);

$sandbox = $argv[1];
putenv('MKBLOG_DATA_DIR=' . $sandbox . '/scheduler-data');
require $sandbox . '/app.php';
function check(bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
}
save_setting($db, 'wallpapers_enabled', '0');
save_setting($db, 'auto_deploy', '1');
save_setting($db, 'netlify_token', 'test');
save_setting($db, 'netlify_site', 'test');
// 本测试以 auto-deploy-test 参数运行，模拟 watcher，禁止产生真实部署进程。
file_put_contents(DATA_DIR . '/deploy.watcher.pid', (string)getmypid());
check(auto_deploy_watcher_alive(), '测试 watcher 未被识别');
$GLOBALS['contract_sync_items'] = [['key' => 'scheduled', 'content' => 'scheduled']];
maybe_sync_plugins($db);
check(is_file(auto_deploy_pending_path()), '独立同步新增动态未触发部署');
unlink(auto_deploy_pending_path());
save_setting($db, 'plugin:contract:last_sync', '0');
maybe_sync_plugins($db);
check(!is_file(auto_deploy_pending_path()), '重复同步不应触发部署');

save_setting($db, 'plugin:contract:last_sync', '0');
$due = time() + 3600;
file_put_contents(auto_deploy_pending_path(), json_encode(['due_at' => $due, 'touched_at' => time()]));
unlink(DATA_DIR . '/deploy.watcher.pid');
$watcher = proc_open([PHP_BINARY, $sandbox . '/bin/sync.php', '--watch'], [0 => ['file', '/dev/null', 'r'], 1 => ['file', DATA_DIR . '/sync.log', 'a'], 2 => ['file', DATA_DIR . '/sync.log', 'a']], $pipes);
try {
    // 新进程使用数据库中的同步间隔，不依赖页面请求。
    $deadline = microtime(true) + 5;
    do {
        usleep(20000);
        $last = (int)(settings($db)['plugin:contract:last_sync'] ?? 0);
    } while ($last === 0 && microtime(true) < $deadline);
    check($last > 0, '常驻同步未执行');
    check(proc_get_status($watcher)['running'], '常驻同步首轮后退出');
    do {
        usleep(20000);
    } while (!auto_deploy_watcher_alive() && microtime(true) < $deadline);
    check(auto_deploy_watcher_alive(), '重启后的待部署任务未恢复');
    check((int)auto_deploy_pending_read()['due_at'] === $due, '恢复部署不应改变原定到期时间');
} finally {
    proc_terminate($watcher);
    proc_close($watcher);
    if (auto_deploy_watcher_alive()) posix_kill((int)file_get_contents(DATA_DIR . '/deploy.watcher.pid'), SIGTERM);
    @unlink(auto_deploy_pending_path());
    @unlink(DATA_DIR . '/deploy.watcher.pid');
}
echo "scheduler: 独立进程同步、常驻检查、待部署恢复、新动态部署及重复同步去重通过\n";
