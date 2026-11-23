<?php
declare(strict_types=1);

function backup_create(PDO $db, string $archive): void {
    if (file_exists($archive)) throw new RuntimeException('备份目标已存在');
    $temp = tempnam(sys_get_temp_dir(), 'muke-backup-');
    $zip = new ZipArchive();
    if ($zip->open($archive, ZipArchive::CREATE | ZipArchive::EXCL) !== true) throw new RuntimeException('无法创建备份');
    $db->beginTransaction();
    try {
        $db->exec('UPDATE settings SET value = value WHERE 0');
        $source = new SQLite3(DATA_DIR . '/blog.sqlite', SQLITE3_OPEN_READONLY);
        $snapshot = new SQLite3($temp);
        if (!$source->backup($snapshot)) throw new RuntimeException('数据库快照失败');
        $source->close(); $snapshot->close();
        $zip->addFile($temp, 'blog.sqlite');
        $paths = ['uploads' => DATA_DIR . '/uploads'];
        foreach (['avatar.webp', 'cover.webp'] as $name) if (is_file(DATA_DIR . '/' . $name)) $paths[$name] = DATA_DIR . '/' . $name;
        $manifest = ['app' => 'mkblog', 'version' => 1, 'paths' => []];
        foreach (plugin_list() as $id => $plugin) {
            foreach (($plugin['backup_paths'] ?? fn() => [])() as $key => $path) {
                if ($path === '') continue;
                if (!is_dir($path) || !is_readable($path)) throw new RuntimeException("插件 $id 的资源目录不可读取");
                $relative = 'plugins/' . $id . '/' . $key;
                $paths[$relative] = $path;
                $manifest['paths'][$key] = $relative;
            }
        }
        foreach ($paths as $relative => $path) {
            if (!file_exists($path)) continue;
            if (is_file($path)) { $zip->addFile($path, $relative); continue; }
            $zip->addEmptyDir($relative);
            foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($path, FilesystemIterator::SKIP_DOTS)) as $file) {
                if ($file->isLink()) throw new RuntimeException('备份资源不能包含符号链接');
                if ($file->isFile() && !$zip->addFile($file->getPathname(), $relative . '/' . substr($file->getPathname(), strlen(rtrim($path, '/')) + 1))) throw new RuntimeException('添加备份资源失败');
            }
        }
        $zip->addFromString('manifest.json', json_encode($manifest, JSON_THROW_ON_ERROR));
        if (!$zip->close()) throw new RuntimeException('备份写入失败');
        $db->commit();
    } catch (Throwable $e) {
        if ($db->inTransaction()) $db->rollBack();
        try { $zip->close(); } catch (ValueError $ignored) {}
        @unlink($archive);
        throw $e;
    } finally { unlink($temp); }
}

function backup_restore(string $archive, string $target): void {
    if (!is_dir($target) || count(scandir($target)) !== 2) throw new RuntimeException('恢复目标必须是已存在的空目录');
    $target = realpath($target);
    $zip = new ZipArchive();
    if ($zip->open($archive) !== true) throw new RuntimeException('无法打开备份');
    try {
        $manifest = json_decode($zip->getFromName('manifest.json') ?: '', true, 512, JSON_THROW_ON_ERROR);
        if (($manifest['app'] ?? '') !== 'mkblog' || ($manifest['version'] ?? 0) !== 1 || $zip->locateName('blog.sqlite') === false) throw new RuntimeException('备份格式不正确');
        for ($i = 0; $i < $zip->numFiles; $i++) {
            $name = $zip->getNameIndex($i);
            if (str_starts_with($name, '/') || str_contains($name, '\\') || str_contains($name, "\0") || in_array('..', explode('/', $name), true)) throw new RuntimeException('备份包含非法路径');
            $zip->getExternalAttributesIndex($i, $opsys, $attrs);
            if (($attrs >> 16 & 0170000) === 0120000) throw new RuntimeException('备份包含符号链接');
        }
        foreach ($manifest['paths'] as $key => $relative) {
            if (!preg_match('#^plugins/[a-z0-9_-]+/[a-z0-9_]+$#', $relative) || !preg_match('/^[a-z0-9_]+$/', $key)) throw new RuntimeException('插件资源声明无效');
        }
        if (!$zip->extractTo($target)) throw new RuntimeException('备份解压失败');
    } finally { $zip->close(); }
    $restored = new PDO('sqlite:' . $target . '/blog.sqlite');
    if ($restored->query('PRAGMA integrity_check')->fetchColumn() !== 'ok') throw new RuntimeException('备份数据库完整性检查失败');
    $stmt = $restored->prepare('INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value');
    foreach ($manifest['paths'] as $key => $relative) $stmt->execute([$key, $target . '/' . $relative]);
}
