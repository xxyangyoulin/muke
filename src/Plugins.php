<?php
declare(strict_types=1);

function plugin_list(): array {
    static $plugins = null;
    if ($plugins === null) {
        $plugins = [];
        $routes = ['/', '/articles', '/admin', '/api', '/image'];
        $protocols = ['up'];
        foreach (glob(dirname(__DIR__) . '/plugins/*/plugin.php') as $file) {
            $plugin = require $file;
            if (!is_array($plugin) || !isset($plugin['id']) || !preg_match('/^[a-z0-9_-]+$/', $plugin['id']) || $plugin['id'] !== basename(dirname($file))) throw new RuntimeException('插件 ID 必须与目录名一致：' . $file);
            if (isset($plugins[$plugin['id']])) throw new RuntimeException('重复插件 ID：' . $plugin['id']);
            if (isset($plugin['page'])) {
                $route = $plugin['page']['route'];
                if (!preg_match('#^/[a-z0-9_-]+$#', $route) || in_array($route, $routes, true) || $route === '/article') throw new RuntimeException('插件路由冲突或无效：' . $route);
                $routes[] = $route;
            }
            foreach ($plugin['ref_protocols'] ?? [] as $protocol) {
                if (!preg_match('/^[a-z0-9_-]{1,16}$/', $protocol) || in_array($protocol, $protocols, true)) throw new RuntimeException('图片协议冲突或无效：' . $protocol);
                $protocols[] = $protocol;
            }
            $plugins[$plugin['id']] = $plugin;
        }
    }
    return $plugins;
}

function plugin_enabled_key(array $plugin): string {
    return $plugin['setting_keys']['enabled'] ?? ('plugin:' . $plugin['id'] . ':enabled');
}

/** 插件来源徽章主题色（label_color）：仅放行 #rgb/#rrggbb/#rrggbbaa，进 state 前消毒 */
function plugin_badge_color(array $plugin): ?string {
    $color = trim((string)($plugin['label_color'] ?? ''));
    return preg_match('/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/', $color) ? strtolower($color) : null;
}

/** 声明了 timeline_field 且当前关闭的「已启用」插件来源：其动态整体退出首页时间线与站点 RSS。
 *  停用插件的动态回到普通时间线（与 weibo/douban 行为一致）；插件自己的页面 / 数据端点不受影响。 */
function plugin_feed_hidden_sources(PDO $db): array {
    $hidden = [];
    foreach (plugin_list() as $id => $plugin) {
        if (empty($plugin['timeline_field'])) continue;
        $config = plugin_config($db, $id);
        if (empty($config['_enabled'])) continue;
        if (!filter_var((string)($config[$plugin['timeline_field']] ?? '0'), FILTER_VALIDATE_BOOLEAN)) $hidden[] = $id;
    }
    return $hidden;
}

/** 时间线查询的 source 过滤片段（表别名 p；无可隐藏来源时返回空串，原查询计划不变） */
function feed_source_filter(PDO $db): string {
    $hidden = plugin_feed_hidden_sources($db);
    if (!$hidden) return '';
    return ' AND p.source NOT IN (' . implode(',', array_map(fn($s) => $db->quote($s), $hidden)) . ')';
}

function plugin_setting_key(array $plugin, string $field): string {
    return $plugin['setting_keys'][$field] ?? ('plugin:' . $plugin['id'] . ':' . $field);
}

function plugin_config(PDO $db, string $id): array {
    $plugin = plugin_list()[$id] ?? null;
    if (!$plugin) return [];
    $allSettings = settings($db);
    $enabled = $allSettings[plugin_enabled_key($plugin)] ?? config(plugin_enabled_key($plugin));
    $values = ['_enabled' => $enabled === '' ? ($plugin['default_enabled'] ?? false) : filter_var($enabled, FILTER_VALIDATE_BOOLEAN)];
    foreach ($plugin['fields'] ?? [] as $field) {
        $key = plugin_setting_key($plugin, $field['key']);
        $value = $allSettings[$key] ?? config($key);
        if ($value === '' && array_key_exists('default', $field)) $value = $field['default'];
        $values[$field['key']] = $value;
    }
    return $values;
}

// 调用方在事务中清理记录，提交后再删除返回的上传图片引用。
function plugin_delete_posts(PDO $db, string $plugin): array {
    $stmt = $db->prepare('SELECT images FROM posts WHERE source = ?');
    $stmt->execute([$plugin]);
    $images = [];
    foreach ($stmt->fetchAll(PDO::FETCH_COLUMN) as $json) {
        foreach (json_decode($json, true) ?: [] as $ref) $images[] = $ref;
    }
    $db->prepare("DELETE FROM comments WHERE kind = 'post' AND target_id IN (SELECT id FROM posts WHERE source = ?)")->execute([$plugin]);
    $db->prepare('DELETE FROM post_likes WHERE post_id IN (SELECT id FROM posts WHERE source = ?)')->execute([$plugin]);
    $db->prepare('DELETE FROM posts WHERE source = ?')->execute([$plugin]);
    return array_values(array_unique($images));
}

function plugin_sync_items(PDO $db, string $plugin, array $items, ?array &$events = null): int {
    if (!$items) return 0;
    $stmt = $db->prepare('SELECT key FROM plugin_tombstones WHERE plugin = ?');
    $stmt->execute([$plugin]);
    $tombstoned = array_fill_keys($stmt->fetchAll(PDO::FETCH_COLUMN), true);
    $existing = [];
    $stmt = $db->prepare('SELECT source_key FROM posts WHERE source = ?');
    $stmt->execute([$plugin]);
    foreach ($stmt->fetchAll(PDO::FETCH_COLUMN) as $k) $existing[$k] = true;
    $insert = $db->prepare('INSERT INTO posts (content, images, meta, created_at, updated_at, source, source_key, link) VALUES (?, ?, ?, ?, ?, ?, ?, ?)');
    $added = 0;
    foreach ($items as $item) {
        $key = (string)($item['key'] ?? '');
        if ($key === '' || isset($tombstoned[$key]) || isset($existing[$key])) continue;
        $images = [];
        foreach ((array)($item['images'] ?? []) as $ref) {
            if (!is_string($ref)) continue;
            if (preg_match('#^[a-z0-9_-]{1,16}:[^:/]#', $ref)) $images[] = $ref; // up: 及插件自定义引用协议（wall: 等）
            elseif (preg_match('#^https?://#', $ref)) { $local = fetch_remote_image($ref); if ($local !== null) $images[] = $local; }
        }
        $time = (string)($item['created_at'] ?? date(DATE_ATOM));
        // link 仅接受 http(s)：防订阅源内容注入 javascript:/data: URL（存储型 XSS）
        $link = (string)($item['link'] ?? '');
        if ($link !== '' && !preg_match('#^https?://#i', $link)) $link = '';
        $meta = isset($item['meta']) && is_array($item['meta']) ? json_encode($item['meta'], JSON_UNESCAPED_UNICODE) : '';
        $insert->execute([(string)($item['content'] ?? ''), json_encode($images), $meta, $time, $time, $plugin, $key, $link]);
        $event = ['id' => (int)$db->lastInsertId(), 'content' => (string)($item['content'] ?? ''), 'source' => $plugin, 'source_key' => $key];
        if ($db->inTransaction()) $events[] = $event;
        else plugin_fire('post.saved', $event);
        $existing[$key] = true;
        $added++;
    }
    return $added;
}

/** 已启用插件的公开页面声明：[route => ['plugin' => id, 'page' => 声明]]（路由放行 / 导航 / 导出共用） */
function plugin_pages(PDO $db): array {
    $pages = [];
    foreach (plugin_list() as $id => $plugin) {
        if (empty($plugin['page']) || !is_array($plugin['page'])) continue;
        $config = plugin_config($db, $id);
        if (empty($config['_enabled'])) continue;
        $pages[$plugin['page']['route']] = ['plugin' => $id, 'page' => $plugin['page']];
    }
    return $pages;
}

/** 插件自定义图片引用（如 wall:<id>）→ 描述符 {url,thumb,...}；核心 post_images 对非 up: 引用逐插件询问 */
function plugin_resolve_ref(PDO $db, string $ref): ?array {
    foreach (plugin_list() as $plugin) {
        if (!in_array(strtok($ref, ':'), $plugin['ref_protocols'] ?? [], true)) continue;
        if (!isset($plugin['resolve_ref']) || !is_callable($plugin['resolve_ref'])) continue;
        $hit = ($plugin['resolve_ref'])($db, $ref);
        if (is_array($hit)) return $hit;
    }
    return null;
}

function maybe_sync_plugins(PDO $db): void {
    $lock = @fopen(DATA_DIR . '/plugin.sync.lock', 'c');
    if ($lock && !flock($lock, LOCK_EX | LOCK_NB)) { fclose($lock); return; }
    try {
    foreach (plugin_list() as $id => $plugin) {
        if (!isset($plugin['sync'])) continue;
        $config = plugin_config($db, $id);
        if (empty($config['_enabled'])) continue;
        if (isset($plugin['interval_key'])) $interval = (int)opt($plugin['interval_key']);
        else $interval = max(1, (int)($config['sync_minutes'] ?? 60)) * 60;
        if ($interval <= 0) continue;
        $last = (int)(settings($db)["plugin:{$id}:last_sync"] ?? 0);
        if (time() - $last < $interval) continue;
        save_setting($db, "plugin:{$id}:last_sync", (string)time());
        try {
            $items = ($plugin['sync'])($db, $config);
            $added = plugin_sync_items($db, $id, $items);
            if ($added > 0) auto_deploy_touch($db);
            if (isset($plugin['after_sync']) && is_callable($plugin['after_sync'])) ($plugin['after_sync'])($db, $items);
            save_setting($db, "plugin:{$id}:last_error", '');
        } catch (Throwable $e) {
            save_setting($db, "plugin:{$id}:last_error", mb_substr($e->getMessage(), 0, 200));
            error_log("plugin {$id} sync failed: " . $e->getMessage());
        }
    }
    } finally {
        if ($lock) { flock($lock, LOCK_UN); fclose($lock); }
    }
}

/** 是否有插件到达同步时间（只读时间戳，不做网络请求、不加锁） */
function plugins_sync_due(PDO $db): bool {
    foreach (plugin_list() as $id => $plugin) {
        if (!isset($plugin['sync'])) continue;
        $config = plugin_config($db, $id);
        if (empty($config['_enabled'])) continue;
        $interval = isset($plugin['interval_key']) ? (int)opt($plugin['interval_key']) : max(1, (int)($config['sync_minutes'] ?? 60)) * 60;
        if ($interval <= 0) continue;
        $last = (int)(settings($db)["plugin:{$id}:last_sync"] ?? 0);
        if (time() - $last >= $interval) return true;
    }
    return false;
}

/** 到期则派发后台同步进程：访客请求不承担外呼耗时（flock 保证同步单飞，stamp 抑制进程churn） */
function dispatch_plugin_sync(PDO $db): void {
    if (!plugins_sync_due($db)) return;
    $stamp = DATA_DIR . '/plugin.dispatch.stamp';
    if (is_file($stamp) && time() - filemtime($stamp) < 30) return;
    @touch($stamp);
    @exec(sprintf('nohup %s %s >/dev/null 2>&1 &', escapeshellarg(PHP_SAPI === 'fpm-fcgi' ? PHP_BINDIR . '/php' : PHP_BINARY), escapeshellarg(dirname(__DIR__) . '/bin/sync.php')));
}

/** 事件总线：插件经 'hooks' => ['事件名' => fn(array $payload)] 订阅核心生命周期。
 *  现有事件：post.saved / post.deleted / article.saved / article.deleted /
 *  comment.created / comment.moderated（payload 见各触发点；无订阅者时零开销） */
function plugin_fire(string $event, array $payload = []): void {
    global $db;
    foreach (plugin_list() as $plugin) {
        if (empty(plugin_config($db, $plugin['id'])['_enabled'])) continue;
        $hooks = $plugin['hooks'] ?? null;
        if (!is_array($hooks) || !isset($hooks[$event]) || !is_callable($hooks[$event])) continue;
        try {
            ($hooks[$event])($payload);
        } catch (Throwable $e) {
            error_log("plugin hook {$plugin['id']} on {$event} failed: " . $e->getMessage());
        }
    }
}
