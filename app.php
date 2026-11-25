<?php
declare(strict_types=1);

define('MKBLOG_VERSION', trim(file_get_contents(__DIR__ . '/VERSION')));

// mkblog 内核装配：src/ 分域模块 + 引导（会话、数据库、迁移）
require_once __DIR__ . '/src/Config.php';
require_once __DIR__ . '/src/Support.php';
require_once __DIR__ . '/src/Settings.php';
require_once __DIR__ . '/src/PublicData.php';
require_once __DIR__ . '/src/Http.php';
require_once __DIR__ . '/src/Images.php';
require_once __DIR__ . '/src/Articles.php';
require_once __DIR__ . '/src/Comments.php';
require_once __DIR__ . '/src/Mail.php';
require_once __DIR__ . '/src/Export.php';
require_once __DIR__ . '/src/Minify.php';
require_once __DIR__ . '/src/Plugins.php';
require_once __DIR__ . '/src/Netlify.php';
require_once __DIR__ . '/src/Tasks.php';

define('DATA_DIR', (string)config('data_dir'));
date_default_timezone_set((string)config('timezone'));
if (!is_dir(DATA_DIR)) mkdir(DATA_DIR, 0755, true);
session_set_cookie_params(['httponly' => true, 'samesite' => 'Lax', 'secure' => !empty($_SERVER['HTTPS']) && $_SERVER['HTTPS'] !== 'off']);
session_start();
$_SESSION['csrf'] ??= bin2hex(random_bytes(24));
$db = new PDO('sqlite:' . DATA_DIR . '/blog.sqlite');
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
$db->exec('PRAGMA busy_timeout = 5000');
require_once __DIR__ . '/src/Database.php';
migrate_database($db);

// 内核装配完成。各域逻辑见 src/ 模块（域说明在各自文件头），
// 插件的页面/表结构/引用解析/导出均由插件自带（见 plugins/*/plugin.php 契约）。

require_once __DIR__ . '/src/Api.php';
