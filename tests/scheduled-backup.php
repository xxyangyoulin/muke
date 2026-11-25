<?php
declare(strict_types=1);

$sandbox = $argv[1];
putenv('MKBLOG_DATA_DIR=' . $sandbox . '/tasks-data');
require $sandbox . '/app.php';
session_write_close();
function check(bool $ok, string $message): void { if (!$ok) throw new RuntimeException($message); }

$walls = $sandbox . '/tasks-walls';
mkdir($walls);
file_put_contents($walls . '/resource.txt', 'backup resource');
save_setting($db, 'wallpaper_dir', $walls);
$db->exec("INSERT INTO posts (content,created_at,updated_at,pinned) VALUES ('kept','2020','2020',1)");
// 模拟旧版本迁移：保留已有动态，补上置顶列。
$db->exec("DROP INDEX idx_posts_pinned_timeline; ALTER TABLE posts DROP COLUMN pinned; UPDATE schema_versions SET version=2 WHERE component='core'");
migrate_database($db);
check($db->query('SELECT pinned FROM posts')->fetchColumn() === 0, '旧动态迁移后的默认置顶值错误');
$db->exec("DROP TABLE article_revisions; UPDATE schema_versions SET version=3 WHERE component='core'");
migrate_database($db);
$db->exec("INSERT INTO articles(title,content,created_at,updated_at) VALUES('revision backup','current','2020','2020')");
$db->exec("INSERT INTO article_revisions(article_id,title,content,format,tags,draft,images,updated_at,saved_at) VALUES(1,'old','history','md','',0,'[]','2019','2020')");
$db->exec('UPDATE posts SET pinned=1');
maybe_scheduled_backup($db);
check(!scheduled_backup_files(), '默认关闭仍执行了备份');
save_setting($db, 'backup_enabled', '1');
save_setting($db, 'backup_interval_hours', '24');
save_setting($db, 'backup_keep', '2');
$lock = fopen(DATA_DIR . '/backup.lock', 'c');
flock($lock, LOCK_EX);
maybe_scheduled_backup($db);
check(!scheduled_backup_files(), '并发备份未被拒绝');
fclose($lock);
maybe_scheduled_backup($db);
$first = task_status_read('backup');
check($first['state'] === 'done', '首次定期备份失败：' . ($first['error'] ?? ''));
maybe_scheduled_backup($db);
check(count(scheduled_backup_files()) === 1, '未到期重复备份');
$archive = DATA_DIR . '/backups/' . $first['file'];
check((fileperms($archive) & 0777) === 0600, '自动备份权限不正确');
$restored = $sandbox . '/tasks-restored';
mkdir($restored);
backup_restore($archive, $restored);
$copy = new PDO('sqlite:' . $restored . '/blog.sqlite');
check($copy->query('SELECT pinned FROM posts')->fetchColumn() === 1, '完整备份未保留置顶');
check($copy->query('SELECT content FROM article_revisions')->fetchColumn() === 'history', '完整备份未保留文章修订历史');
check(file_get_contents($restored . '/plugins/wallpapers/wallpaper_dir/resource.txt') === 'backup resource', '资源文件未恢复');
file_put_contents(DATA_DIR . '/backups/manual.zip', 'manual');
for ($i = 0; $i < 3; $i++) {
    $status = task_status_read('backup');
    $status['started_at'] = time() - 90000;
    task_status_write('backup', $status);
    maybe_scheduled_backup($db);
    check(task_status_read('backup')['state'] === 'done', '轮换备份失败');
}
check(count(scheduled_backup_files()) === 2, '未按保留份数清理');
check(is_file(DATA_DIR . '/backups/manual.zip'), '误删手工备份');
$before = scheduled_backup_files();
$status = task_status_read('backup');
$status['started_at'] = time() - 90000;
task_status_write('backup', $status);
save_setting($db, 'wallpaper_dir', $walls . '/missing');
maybe_scheduled_backup($db);
$failure = task_status_read('backup');
check($failure['state'] === 'error' && $failure['error'] !== '', '失败未记录');
check($before === scheduled_backup_files(), '失败时清理了旧备份');
maybe_scheduled_backup($db);
check(task_status_read('backup') === $failure, '失败后紧密重试');
check(!glob(DATA_DIR . '/backups/*.partial'), '失败留下不完整文件');
save_setting($db, 'wallpaper_dir', $walls);
try { backup_create($db, $walls . '/recursive.zip'); throw new RuntimeException('未阻止备份自身'); }
catch (RuntimeException $e) { check(str_contains($e->getMessage(), '备份目标不能'), '备份目录重叠检查失败'); }
check(!is_file($walls . '/recursive.zip'), '目录重叠留下备份文件');
check(!task_running('worker'), '空闲 worker 被判定为运行');
$lock = fopen(DATA_DIR . '/worker.lock', 'c');
flock($lock, LOCK_EX);
check(task_running('worker'), '未检测到运行中的 worker');
fclose($lock);
echo "tasks: 旧库迁移、定期备份、恢复、并发互斥、保留策略、失败保护和运行状态通过\n";
