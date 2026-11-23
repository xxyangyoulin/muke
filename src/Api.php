<?php
declare(strict_types=1);

require_once __DIR__ . '/Api/Feed.php';
require_once __DIR__ . '/Api/Content.php';
require_once __DIR__ . '/Api/Plugins.php';
require_once __DIR__ . '/Api/Comments.php';
require_once __DIR__ . '/Api/System.php';

/** API 分发：路由表 → 处理器（处理器自行校验并以 json_response 终结） */
function api(PDO $db): never {
    $action = $_GET['action'] ?? 'state';
    $admin = isset($_SESSION['user']);
    if ($_SERVER['REQUEST_METHOD'] === 'GET') {
        $routes = [
            'state' => 'api_state',
            'posts' => 'api_posts',
            'plugin-data' => 'api_plugin_data',
            'articles' => 'api_articles',
            'plugins' => 'api_plugins',
            'comments' => 'api_comments', 'likes' => 'api_likes',
            'comments-admin' => 'api_comments_admin',
            'deploy-status' => 'api_deploy_status',
            'backup-export' => 'api_backup_export', 'download-static' => 'api_download_static',
        ];
        if (!isset($routes[$action])) json_response(['error' => '接口不存在'], 404);
        ($routes[$action])($db, $admin);
    }
    if ($_SERVER['REQUEST_METHOD'] !== 'POST') json_response(['error' => '不支持的请求方法'], 405);
    if (!hash_equals($_SESSION['csrf'], $_SERVER['HTTP_X_CSRF_TOKEN'] ?? '')) json_response(['error' => '页面已过期，请刷新后重试'], 403);
    $input = json_decode(file_get_contents('php://input'), true);
    if (!is_array($input)) $input = $_POST;
    $routes = [
        'setup' => 'api_auth', 'login' => 'api_auth', 'logout' => 'api_logout',
        'save-post' => 'api_save_post', 'delete-post' => 'api_delete_post',
        'render' => 'api_render', 'upload-image' => 'api_upload_image',
        'save-article' => 'api_save_article', 'delete-article' => 'api_delete_article',
        'save-plugin' => 'api_save_plugin', 'remove-plugin' => 'api_remove_plugin', 'sync-plugin' => 'api_sync_plugin', 'purge-plugin-posts' => 'api_purge_plugin_posts', 'plugin-admin' => 'api_plugin_admin',
        'comment' => 'api_comment', 'like' => 'api_like', 'moderate-comment' => 'api_moderate_comment',
        'profile' => 'api_profile', 'change-password' => 'api_change_password',
        'save-avatar' => 'api_save_avatar', 'remove-avatar' => 'api_remove_avatar',
        'save-cover' => 'api_save_cover', 'remove-cover' => 'api_remove_cover',
        'save-settings' => 'api_save_settings',
        'export' => 'api_export',
        'netlify-save' => 'api_netlify_save', 'netlify-deploy' => 'api_netlify_deploy',
        'backup-import' => 'api_backup_import',
    ];
    if (!isset($routes[$action])) json_response(['error' => '接口不存在'], 404);
    if (!in_array($action, ['setup', 'login', 'comment', 'like'], true) && !$admin) json_response(['error' => '请先登录管理后台'], 401);
    ($routes[$action])($db, $input, $action, $admin);
}
