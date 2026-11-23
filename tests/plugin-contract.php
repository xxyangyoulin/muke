<?php
declare(strict_types=1);

$sandbox = $argv[1];
$dataDir = $sandbox . '/contract-data';
mkdir($dataDir);
putenv('MKBLOG_DATA_DIR=' . $dataDir);
putenv('MKBLOG_GALLERY_BATCH=12');
$legacy = new PDO('sqlite:' . $dataDir . '/blog.sqlite');
$legacy->exec('CREATE TABLE plugin_tombstones (key TEXT PRIMARY KEY, plugin TEXT NOT NULL, deleted_at TEXT NOT NULL)');
$legacy->exec("INSERT INTO plugin_tombstones VALUES ('shared', 'douban', '2026-11-24')");
$legacy = null;
require $sandbox . '/app.php';

function verify(bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
}

verify(config('wallpaper_sync_interval') === 30, '插件默认配置未加载');
verify(config('gallery_batch') === '12', '插件环境变量未覆盖默认值');
verify(config('wallpaper_dir') === $sandbox . '/walls', '个人配置未覆盖插件默认值');
verify($db->query("SELECT COUNT(*) FROM plugin_tombstones WHERE plugin = 'douban' AND key = 'shared'")->fetchColumn() === 1, '迁移丢失旧墓碑');
$db->exec("INSERT INTO plugin_tombstones VALUES ('shared', 'weibo', '2026-11-24')");
verify($db->query('SELECT COUNT(*) FROM plugin_tombstones')->fetchColumn() === 2, '不同插件同名墓碑不能共存');
verify(plugin_sync_items($db, 'douban', [['key' => 'shared', 'content' => 'deleted']]) === 0, '墓碑未阻止同插件恢复');
$GLOBALS['contract_events'] = [];
verify(plugin_sync_items($db, 'contract', [['key' => 'shared', 'content' => 'visible', 'images' => array_fill(0, 6, 'up:test.jpg')]]) === 1, '其他插件墓碑干扰入库');
verify(count($GLOBALS['contract_events']) === 1, '插件入库未触发保存事件');
verify(count(json_decode($db->query("SELECT images FROM posts WHERE source = 'contract'")->fetchColumn(), true)) === 6, '插件图片被截断');
verify(plugin_sync_items($db, 'contract', [['key' => 'shared']]) === 0, '重复同步未去重');
$events = [];
$db->beginTransaction();
plugin_sync_items($db, 'contract', [['key' => 'rollback', 'content' => 'rollback']], $events);
verify(count($GLOBALS['contract_events']) === 1 && count($events) === 1, '事务内提前发出事件');
$db->rollBack();
verify(count($GLOBALS['contract_events']) === 1, '回滚发出了事件');
$events = [];
$db->beginTransaction();
plugin_sync_items($db, 'contract', [['key' => 'commit', 'content' => 'commit']], $events);
$db->commit();
foreach ($events as $event) plugin_fire('post.saved', $event);
verify(count($GLOBALS['contract_events']) === 2, '提交后事件缺失');
$GLOBALS['contract_events'] = array_slice($GLOBALS['contract_events'], 0, 1);
migrate_database($db);
verify($GLOBALS['contract_migrations'] === 1, '重复执行已完成迁移');
verify($db->query("SELECT version FROM schema_versions WHERE component = 'core'")->fetchColumn() === 2, '迁移版本未保存');
save_setting($db, 'plugin:contract:enabled', '0');
verify(plugin_config($db, 'contract')['_enabled'] === false, '默认启用插件无法关闭');
plugin_fire('post.saved', ['id' => 123]);
verify(count($GLOBALS['contract_events']) === 1, '停用插件仍收到事件');
$GLOBALS['contract_exports'] = [];
$ctx = ['db' => $db, 'dest' => $sandbox . '/contract-export', 'files' => []];
save_setting($db, 'wallpapers_enabled', '0');
export_plugins($ctx);
verify($GLOBALS['contract_exports'] === [false], '历史内容未以只读模式导出');
$db->exec("DELETE FROM posts WHERE source = 'contract'");
export_plugins($ctx);
verify($GLOBALS['contract_exports'] === [false], '停用且无历史内容的插件仍执行导出');

// 停用壁纸插件后，导出保留引用的原图，不扫描登记目录里的其他图片。
$image = $sandbox . '/walls/w1.jpg';
$info = getimagesize($image);
$id = hash('sha256', 'w1.jpg');
$db->prepare('INSERT INTO wallpapers (id, path, name, width, height, bytes, mtime, post_id) VALUES (?, ?, ?, ?, ?, ?, ?, 1)')->execute([$id, 'w1.jpg', 'w1.jpg', $info[0], $info[1], filesize($image), filemtime($image)]);
plugin_sync_items($db, 'wallpapers', [['key' => 'history', 'content' => 'history', 'images' => ['wall:' . $id]]]);
mkdir($ctx['dest'] . '/media', 0755, true);
mkdir($ctx['dest'] . '/thumbnails');
export_plugins($ctx);
verify(isset($ctx['pluginRefs']['wall:' . $id]), '停用后历史图片未导出');
verify($db->query('SELECT COUNT(*) FROM wallpapers')->fetchColumn() === 1, '停用后仍扫描登记新壁纸');
verify(!isset(plugin_pages($db)['/wallpapers']), '停用插件页面仍公开');
echo "plugin-contract: 旧表迁移、配置、墓碑隔离、事件、完整图片和停用导出通过\n";
