<?php
require dirname(__DIR__) . '/app.php';
session_write_close();
try {
    $result = export_site($db);
    echo "已导出 {$result['posts']} 条动态及插件内容到 dist/\n";
} catch (Throwable $e) {
    fwrite(STDERR, '导出失败：' . $e->getMessage() . "\n");
    exit(1);
}
