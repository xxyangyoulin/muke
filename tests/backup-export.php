<?php
declare(strict_types=1);
require $argv[1] . '/app.php';
require $argv[1] . '/src/Backup.php';
$root = $argv[1];
function check_result(bool $ok, string $message): void { if (!$ok) throw new RuntimeException($message); }
file_put_contents(DATA_DIR . '/avatar.webp', file_get_contents($root . '/walls/w1.jpg'));
$archive = $root . '/complete.zip';
backup_create($db, $archive);
$restored = $root . '/restored';
mkdir($restored);
backup_restore($archive, $restored);
$copy = new PDO('sqlite:' . $restored . '/blog.sqlite');
foreach (['users','posts','articles','comments','wallpapers','plugin_tombstones'] as $table) check_result($db->query('SELECT COUNT(*) FROM ' . $table)->fetchColumn() === $copy->query('SELECT COUNT(*) FROM ' . $table)->fetchColumn(), '恢复表不一致：' . $table);
check_result(is_file($restored . '/avatar.webp'), '头像没有恢复');
$path = $copy->query("SELECT value FROM settings WHERE key = 'wallpaper_dir'")->fetchColumn();
check_result(is_file($path . '/w1.jpg') && str_starts_with($path, $restored), '插件资源没有恢复或路径未重定位');
$original = hash_file('sha256', $root . '/dist/index.html');
save_setting($db, 'plugin:contract:enabled', '1');
$GLOBALS['contract_export_fail'] = true;
try { export_site($db); throw new RuntimeException('预期导出失败'); }
catch (RuntimeException $e) { check_result($e->getMessage() === 'injected export failure', $e->getMessage()); }
check_result(hash_file('sha256', $root . '/dist/index.html') === $original, '失败导出损坏上一版');
check_result(!glob($root . '/.export-*'), '失败导出留下临时目录');
echo "backup-export: 空目录完整恢复、插件路径重定位和导出失败保留旧版通过\n";
