<?php
declare(strict_types=1);

function task_status_read(string $task): array {
    $path = DATA_DIR . '/' . $task . '.status.json';
    return is_file($path) ? (json_decode((string)file_get_contents($path), true) ?: []) : [];
}

function task_status_write(string $task, array $status): void {
    $path = DATA_DIR . '/' . $task . '.status.json';
    $temp = tempnam(DATA_DIR, $task . '-');
    if ($temp === false) throw new RuntimeException('无法创建任务状态文件');
    try {
        if (file_put_contents($temp, json_encode($status, JSON_THROW_ON_ERROR)) === false || !rename($temp, $path)) throw new RuntimeException('无法保存任务状态');
    } finally { if (is_file($temp)) unlink($temp); }
}

function task_running(string $task): bool {
    $lock = fopen(DATA_DIR . '/' . $task . '.lock', 'c');
    $busy = !flock($lock, LOCK_EX | LOCK_NB);
    fclose($lock);
    return $busy;
}

function scheduled_backup_files(): array {
    $directory = DATA_DIR . '/backups';
    if (is_link($directory)) throw new RuntimeException('自动备份目录不能是符号链接');
    $files = [];
    foreach (glob($directory . '/muke-auto-*.zip') as $file) {
        if (is_link($file) || !is_file($file) || !preg_match('/^muke-auto-\d{8}-\d{6}-[a-f0-9]{8}\.zip$/', basename($file))) continue;
        $files[] = ['name' => basename($file), 'bytes' => filesize($file), 'created_at' => filemtime($file)];
    }
    usort($files, fn($a, $b) => ($b['created_at'] <=> $a['created_at']) ?: strcmp($b['name'], $a['name']));
    return $files;
}

function maybe_scheduled_backup(PDO $db): void {
    if (opt('backup_enabled') !== '1') return;
    require_once __DIR__ . '/Backup.php';
    $lock = fopen(DATA_DIR . '/backup.lock', 'c');
    if (!flock($lock, LOCK_EX | LOCK_NB)) { fclose($lock); return; }
    try {
        $previous = task_status_read('backup');
        $interval = max(1, (int)opt('backup_interval_hours')) * 3600;
        if (time() < (int)($previous['started_at'] ?? 0) + $interval) return;
        $status = ['state' => 'running', 'started_at' => time(), 'last_success' => $previous['last_success'] ?? 0, 'error' => ''];
        task_status_write('backup', $status);
        try {
            $directory = DATA_DIR . '/backups';
            if (is_link($directory)) throw new RuntimeException('自动备份目录不能是符号链接');
            if (!is_dir($directory) && !mkdir($directory, 0700)) throw new RuntimeException('无法创建自动备份目录');
            $name = 'muke-auto-' . date('Ymd-His') . '-' . bin2hex(random_bytes(4)) . '.zip';
            $archive = $directory . '/' . $name;
            $partial = $archive . '.partial';
            backup_create($db, $partial);
            if (!chmod($partial, 0600) || !rename($partial, $archive)) throw new RuntimeException('无法保存自动备份文件');
            $status['last_success'] = time();
            $status['file'] = $name;
            // 只在新备份成功后轮换本功能生成的文件，不清理手工备份。
            $older = array_values(array_filter(scheduled_backup_files(), fn($file) => $file['name'] !== $name));
            foreach (array_slice($older, max(1, (int)opt('backup_keep')) - 1) as $file) {
                if (!unlink($directory . '/' . $file['name'])) throw new RuntimeException('备份成功，但旧备份清理失败');
            }
            $status['state'] = 'done';
        } catch (Throwable $e) {
            if (isset($partial) && is_file($partial)) unlink($partial);
            $status['state'] = 'error';
            $status['error'] = $e->getMessage();
        }
        $status['finished_at'] = time();
        task_status_write('backup', $status);
    } finally { fclose($lock); }
}
