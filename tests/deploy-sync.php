<?php
declare(strict_types=1);

$sandbox = $argv[1];
$dataDir = $sandbox . '/deploy-sync-data';
if (!is_dir($dataDir)) mkdir($dataDir);
define('DATA_DIR', $dataDir);
require $sandbox . '/src/Netlify.php';

function export_site(PDO $db): array {
    file_put_contents(DATA_DIR . '/entered', '1');
    $deadline = microtime(true) + 10;
    while (!is_file(DATA_DIR . '/release') && microtime(true) < $deadline) usleep(20000);
    throw new RuntimeException('injected export failure');
}
function check(bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
}
function opt(string $key): string {
    return ['auto_deploy' => '1', 'auto_deploy_debounce' => '10', 'netlify_token' => 'test', 'netlify_site' => 'test'][$key];
}
$db = new PDO('sqlite::memory:');
if (($argv[2] ?? '') === 'auto-deploy-worker') exit(netlify_deploy_run($db, auto_deploy_pending_read()) ? 1 : 0);

$original = ['due_at' => time() - 1, 'touched_at' => time() - 60, 'id' => 'original'];
file_put_contents(auto_deploy_pending_path(), json_encode($original));
$worker = proc_open([PHP_BINARY, __FILE__, $sandbox, 'auto-deploy-worker'], [0 => ['file', '/dev/null', 'r'], 1 => ['file', $dataDir . '/worker.log', 'a'], 2 => ['file', $dataDir . '/worker.log', 'a']], $pipes);
file_put_contents(DATA_DIR . '/deploy.watcher.pid', (string)proc_get_status($worker)['pid']);
try {
    $deadline = microtime(true) + 5;
    while (!is_file($dataDir . '/entered') && microtime(true) < $deadline) usleep(20000);
    check(is_file($dataDir . '/entered'), '部署进程未进入导出');
    $before = file_get_contents(netlify_status_path());
    check(netlify_deploy_running(), '未识别运行中的部署');
    check(!netlify_deploy_run($db, $original), '并发部署未被拒绝');
    check(auto_deploy_pending_read() === $original, '抢锁失败删除了待办');
    check(file_get_contents(netlify_status_path()) === $before, '冲突任务覆盖了部署状态');
    file_put_contents(netlify_status_path(), json_encode(['state' => 'uploading', 'updated_at' => 0]));
    check(netlify_deploy_running(), '心跳过期不应绕过活跃部署锁');
    auto_deploy_touch($db);
    $updated = auto_deploy_pending_read();
    auto_deploy_touch($db);
    $latest = auto_deploy_pending_read();
    check($updated['id'] !== $latest['id'], '连续更新缺少独立标识');
    auto_deploy_pending_clear($updated);
    check(auto_deploy_pending_read() === $latest, '旧记录清理误删新记录');
} finally {
    file_put_contents($dataDir . '/release', '1');
    $exit = proc_close($worker);
}
check($exit === 0, '部署失败路径异常');
check(netlify_status_read()['state'] === 'error', '部署失败状态未保存');
check(!netlify_deploy_running(), '部署失败后未释放锁');
check(auto_deploy_pending_read() === $latest, '部署结束删除了期间新增的待办');
check(!netlify_deploy_run($db, $latest), '失败部署应返回 false');
check(auto_deploy_pending_read() === [], '已处理的失败待办应清除而非无限重试');
@unlink(DATA_DIR . '/deploy.watcher.pid');
$lock = fopen($dataDir . '/deploy.lock', 'c');
check(flock($lock, LOCK_EX | LOCK_NB), '锁无法重新获取');
fclose($lock);
echo "deploy: 并发拒绝、状态隔离、过期心跳、新待办保留、旧待办清理和失败释放锁通过\n";
