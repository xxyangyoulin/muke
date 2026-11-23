<?php
declare(strict_types=1);

// 插件同步 CLI：正常由页面访问到期时后台派发（见 src/Plugins.php 的 dispatch_plugin_sync），
// 也可加入 crontab 主动驱动，例如每分钟：* * * * * php /path/to/mkblog/bin/sync.php
require_once __DIR__ . '/../app.php';
maybe_sync_plugins($db);
echo "plugin sync done\n";
