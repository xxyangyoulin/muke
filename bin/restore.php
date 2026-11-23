<?php
require dirname(__DIR__) . '/src/Backup.php';
try {
    if (empty($argv[1]) || empty($argv[2])) throw new RuntimeException('用法：php bin/restore.php backup.zip /absolute/path/empty-data-directory');
    backup_restore($argv[1], $argv[2]);
    echo "恢复完成，请将 MKBLOG_DATA_DIR 指向恢复目录\n";
} catch (Throwable $e) { fwrite(STDERR, $e->getMessage() . "\n"); exit(1); }
