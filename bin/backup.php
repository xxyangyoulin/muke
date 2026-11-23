<?php
require dirname(__DIR__) . '/app.php';
require_once dirname(__DIR__) . '/src/Backup.php';
session_write_close();
try {
    if (empty($argv[1])) throw new RuntimeException('用法：php bin/backup.php /absolute/path/backup.zip');
    backup_create($db, $argv[1]);
    echo "完整备份已保存\n";
} catch (Throwable $e) { fwrite(STDERR, $e->getMessage() . "\n"); exit(1); }
