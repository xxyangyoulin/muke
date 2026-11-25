<?php
declare(strict_types=1);

$sandbox = $argv[1];
putenv('MKBLOG_DATA_DIR=' . $sandbox . '/dms-test-data');
require $sandbox . '/app.php';
function check(bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
}
$directory = DATA_DIR . '/desktop';
mkdir($directory . '/plugins/first', 0755, true);
file_put_contents($directory . '/plugins/first/plugin.json', json_encode(['id' => 'first', 'name' => 'First', 'version' => '1.0']));
$packages = DATA_DIR . '/packages';
mkdir($packages . '/dux-shell-git-2.0', 0755, true);
file_put_contents($packages . '/dux-shell-git-2.0/desc', "%NAME%\ndux-shell-git\n\n%VERSION%\n2.0-1\n\n");
$snapshot = dms_read_snapshot($directory, $packages);
check($snapshot['dms'] === ['package' => 'dux-shell-git', 'version' => '2.0-1'], '未解析 DMS 软件包版本');
check(dms_read_snapshot($directory, DATA_DIR . '/no-packages')['dms'] === null, '无软件包记录时应跳过版本检测');
$plugin = plugin_list()['dms'];
$config = ['config_dir' => $directory];
check(!plugin_config($db, 'dms')['_enabled'], 'DMS 插件不应默认启用');
$items = ($plugin['sync'])($db, $config);
check($items === [], '首次同步不得发布已有插件');
check(!isset(settings($db)['plugin:dms:snapshot']), '同步完成前不得推进基线');
($plugin['after_sync'])($db, $items);
$baseline = settings($db)['plugin:dms:snapshot'];

mkdir($directory . '/plugins/second');
file_put_contents($directory . '/plugins/second/plugin.json', json_encode(['id' => 'second', 'name' => 'Second', 'version' => '1.0']));
file_put_contents($directory . '/plugins/first/plugin.json', json_encode(['id' => 'first', 'name' => 'First', 'version' => '2.0']));
$old = json_decode($baseline, true);
$old['kernel'] = 'old-kernel';
if ($old['dms'] !== null) $old['dms']['version'] = 'old-dms';
save_setting($db, 'plugin:dms:snapshot', json_encode($old));
$items = ($plugin['sync'])($db, $config);
check(count($items) === ($old['dms'] === null ? 3 : 4), '新增插件、插件升级、内核或 DMS 版本事件缺失');
check(str_contains($items[0]['content'], 'Second'), '新增插件名称丢失');
check(str_contains($items[1]['content'], '1.0 → 2.0'), '插件版本变化丢失');
check(plugin_sync_items($db, 'dms', $items) === count($items), '变更未生成动态');
$retry = ($plugin['sync'])($db, $config);
check(array_column($retry, 'key') === array_column($items, 'key'), '重试时事件键发生变化');
check(plugin_sync_items($db, 'dms', $retry) === 0, '重试产生重复动态');
($plugin['after_sync'])($db, $retry);
check(($plugin['sync'])($db, $config) === [], '没有变化仍生成动态');
($plugin['after_sync'])($db, []);
$baseline = settings($db)['plugin:dms:snapshot'];

file_put_contents($directory . '/plugins/second/plugin.json', '{');
$rejected = false;
try { ($plugin['sync'])($db, $config); } catch (RuntimeException $e) { $rejected = true; }
check($rejected, '无效清单未报错');
check(settings($db)['plugin:dms:snapshot'] === $baseline, '读取失败覆盖了基线');
file_put_contents($directory . '/plugins/second/plugin.json', json_encode(['id' => 'second', 'name' => 'Second', 'version' => '1.0']));
check(($plugin['sync'])($db, $config) === [], '清单恢复后误报新增插件');
$rejected = false;
try { ($plugin['sync'])($db, ['config_dir' => DATA_DIR . '/missing']); } catch (RuntimeException $e) { $rejected = true; }
check($rejected, '缺失目录未报错');

mkdir(DATA_DIR . '/other/plugins', 0755, true);
check(($plugin['sync'])($db, ['config_dir' => DATA_DIR . '/other']) === [], '切换目录应重新建立基线');
($plugin['after_sync'])($db, []);

foreach (['events_db', 'cursor', 'music_day'] as $key) save_setting($db, 'plugin:dms:' . $key, 'old');
($plugin['tables'])($db);
check(!isset(settings($db)['plugin:dms:events_db']), '旧采集配置未清理');
check(isset(settings($db)['plugin:dms:snapshot']), '迁移删除了当前基线');
echo "dms: 纯文件读取、首次基线、插件与版本变化、重试去重、失败保护及旧配置清理通过\n";
