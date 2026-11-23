<?php
declare(strict_types=1);

// 手动演练：php tests/restore-rehearsal.php <mktemp 创建的空目录>。不启动应用或同步任务。
require dirname(__DIR__) . '/src/Config.php';
require dirname(__DIR__) . '/src/Plugins.php';
require dirname(__DIR__) . '/src/Settings.php';
require dirname(__DIR__) . '/src/Backup.php';
require dirname(__DIR__) . '/src/Database.php';

$target = realpath($argv[1] ?? '');
if (!$target || !is_dir($target) || count(scandir($target)) !== 2) throw new RuntimeException('演练目录必须存在且为空');
$sourceDir = (string)config('data_dir');
define('DATA_DIR', $target . '/snapshot');
mkdir(DATA_DIR, 0700);
$archive = $target . '/complete.zip';
$restoredDir = $target . '/restored';
$files = 0;
try {
    $source = new SQLite3($sourceDir . '/blog.sqlite', SQLITE3_OPEN_READONLY);
    $snapshot = new SQLite3(DATA_DIR . '/blog.sqlite');
    if (!$source->backup($snapshot)) throw new RuntimeException('快照失败');
    $source->close(); $snapshot->close();
    foreach (['uploads', 'avatar.webp', 'cover.webp'] as $name) {
        $path = $sourceDir . '/' . $name;
        if (!file_exists($path)) continue;
        if (is_file($path)) {
            if (!copy($path, DATA_DIR . '/' . $name)) throw new RuntimeException('资源复制失败');
            continue;
        }
        mkdir(DATA_DIR . '/' . $name, 0700);
        foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($path, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST) as $file) {
            if ($file->isLink()) throw new RuntimeException('不复制符号链接');
            $dest = DATA_DIR . '/' . $name . '/' . substr($file->getPathname(), strlen($path) + 1);
            if ($file->isDir()) mkdir($dest, 0700);
            elseif (!copy($file->getPathname(), $dest)) throw new RuntimeException('资源复制失败');
        }
    }
    $db = new PDO('sqlite:' . DATA_DIR . '/blog.sqlite');
    $db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
    $paths = [];
    foreach (plugin_list() as $id => $plugin) {
        foreach (($plugin['backup_paths'] ?? fn() => [])() as $key => $path) {
            if ($path !== '') $paths[$key] = ['relative' => 'plugins/' . $id . '/' . $key, 'source' => $path];
        }
    }
    backup_create($db, $archive);
    mkdir($restoredDir, 0700);
    backup_restore($archive, $restoredDir);
    $restored = new PDO('sqlite:' . $restoredDir . '/blog.sqlite');
    $tables = $db->query("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")->fetchAll(PDO::FETCH_COLUMN);
    foreach ($tables as $table) {
        $quoted = '"' . str_replace('"', '""', $table) . '"';
        $before = $db->query('SELECT * FROM ' . $quoted . ' ORDER BY rowid')->fetchAll(PDO::FETCH_ASSOC);
        $after = $restored->query('SELECT * FROM ' . $quoted . ' ORDER BY rowid')->fetchAll(PDO::FETCH_ASSOC);
        if ($table === 'settings') {
            foreach ($after as &$row) {
                if (!isset($paths[$row['key']])) continue;
                if ($row['value'] !== $restoredDir . '/' . $paths[$row['key']]['relative']) throw new RuntimeException('资源路径未重定位');
            }
            unset($row);
            $before = array_values(array_filter($before, fn($row) => !isset($paths[$row['key']])));
            $after = array_values(array_filter($after, fn($row) => !isset($paths[$row['key']])));
        }
        if ($before !== $after) throw new RuntimeException('恢复表内容不一致：' . $table);
    }
    $resourcePaths = ['uploads' => DATA_DIR . '/uploads'];
    foreach (['avatar.webp', 'cover.webp'] as $name) if (is_file(DATA_DIR . '/' . $name)) $resourcePaths[$name] = DATA_DIR . '/' . $name;
    foreach ($paths as $path) $resourcePaths[$path['relative']] = $path['source'];
    foreach ($resourcePaths as $relative => $path) {
        if (!file_exists($path)) continue;
        $entries = is_file($path) ? [$path] : new RecursiveIteratorIterator(new RecursiveDirectoryIterator($path, FilesystemIterator::SKIP_DOTS));
        foreach ($entries as $entry) {
            $file = (string)$entry;
            if (!is_file($file)) continue;
            $dest = $restoredDir . '/' . $relative . (is_dir($path) ? '/' . substr($file, strlen(rtrim($path, '/')) + 1) : '');
            if (!is_file($dest) || hash_file('sha256', $file) !== hash_file('sha256', $dest)) throw new RuntimeException('资源校验失败');
            $files++;
        }
    }
    $db = $restored;
    migrate_database($db);
    if ($db->query('PRAGMA integrity_check')->fetchColumn() !== 'ok') throw new RuntimeException('恢复后迁移校验失败');
    echo 'restore-rehearsal: ' . count($tables) . ' 张表逐行一致，' . $files . " 个文件 SHA-256 一致，账号/配置保留，插件路径重定位及迁移通过\n";
} finally {
    $db = null; $restored = null;
    foreach ([DATA_DIR, $restoredDir] as $directory) {
        if (!is_dir($directory)) continue;
        foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($directory, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::CHILD_FIRST) as $file) {
            if ($file->isDir() && !$file->isLink()) rmdir($file->getPathname());
            else unlink($file->getPathname());
        }
        rmdir($directory);
    }
    if (is_file($archive)) unlink($archive);
}
