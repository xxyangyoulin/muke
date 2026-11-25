<?php
declare(strict_types=1);

function api_plugin_data(PDO $db, bool $admin): never {
        $id = (string)($_GET['id'] ?? '');
        $plugin = plugin_list()[$id] ?? null;
        if (!$plugin || empty($plugin['data']) || !is_callable($plugin['data'])) json_response(['error' => '插件数据不存在'], 404);
        if (empty(plugin_config($db, $id)['_enabled'])) json_response(['error' => '插件未启用'], 404);
        json_response(['ok' => true, 'data' => ($plugin['data'])($db, $admin)]);
}

function api_plugins(PDO $db, bool $admin): never {
        if (!$admin) json_response(['error' => '请先登录管理后台'], 401);
        $list = [];
        $allSettings = settings($db);
        foreach (plugin_list() as $id => $plugin) {
            $config = plugin_config($db, $id);
            $stmt = $db->prepare('SELECT COUNT(*) FROM posts WHERE source = ?');
            $stmt->execute([$id]);
            $fields = [];
            foreach ($plugin['fields'] ?? [] as $field) {
                $f = $field;
                $f['value'] = $config[$field['key']] ?? '';
                $fields[] = $f;
            }
            $list[] = ['id' => $id, 'name' => $plugin['name'] ?? $id, 'description' => $plugin['description'] ?? '', 'fields' => $fields, 'enabled' => (bool)$config['_enabled'], 'supports_history' => !empty($plugin['supports_history']), 'post_count' => (int)$stmt->fetchColumn(), 'last_sync' => (int)($allSettings["plugin:{$id}:last_sync"] ?? 0), 'last_error' => (string)($allSettings["plugin:{$id}:last_error"] ?? ''), 'removed' => isset($allSettings["plugin:{$id}:removed"])];
        }
        json_response(['plugins' => $list]);
}

function api_save_plugin(PDO $db, array $input, string $action, bool $admin): never {
    $id = (string)($input['id'] ?? '');
    $plugin = plugin_list()[$id] ?? null;
    if (!$plugin) json_response(['error' => '插件不存在'], 404);
    $values = (array)($input['values'] ?? []);
    if (!empty($input['enabled'])) $values = array_replace(plugin_config($db, $id), $values);
    $changes = [];
    foreach ($plugin['fields'] ?? [] as $field) {
        // 启用时校验合并后的完整配置；其他请求只处理提交的字段。
        if (!array_key_exists($field['key'], $values)) continue;
        $key = plugin_setting_key($plugin, $field['key']);
        $raw = trim((string)($values[$field['key']] ?? ''));
        $type = $field['type'] ?? 'text';
        $label = $field['label'] ?? $field['key'];
        if ($type === 'number') {
            $num = filter_var($raw, FILTER_VALIDATE_INT);
            $min = (int)($field['min'] ?? 0);
            $max = (int)($field['max'] ?? 1000000);
            if (($raw !== '' && $num === false) || ($raw === '' && !empty($field['required'])) || ($num !== false && ($num < $min || $num > $max))) json_response(['error' => "{$label} 需在 {$min}–{$max} 之间"], 422);
            $changes[$key] = (string)($num === false ? ($field['default'] ?? '') : $num);
        } elseif ($type === 'select') {
            if (!array_key_exists($raw, (array)($field['options'] ?? []))) json_response(['error' => "{$label} 取值无效"], 422);
            $changes[$key] = $raw;
        } else {
            if (!empty($field['required']) && $raw === '') json_response(['error' => "{$label} 不能为空"], 422);
            if ($raw !== '' && mb_strlen($raw) > (int)($field['max'] ?? 200)) json_response(['error' => "{$label} 过长"], 422);
            if ($raw !== '' && isset($field['pattern']) && !preg_match($field['pattern'], $raw)) json_response(['error' => "{$label} 格式不正确"], 422);
            $changes[$key] = $raw;
        }
    }
    if (array_key_exists('enabled', $input)) $changes[plugin_enabled_key($plugin)] = !empty($input['enabled']) ? '1' : '0';
    $db->beginTransaction();
    try {
    foreach ($changes as $key => $value) save_setting($db, $key, $value);
    $db->prepare('DELETE FROM settings WHERE key = ?')->execute(["plugin:{$id}:removed"]);
    // 配置变更钩子：让开关类设置即时作用到已同步的数据
    if (isset($plugin['on_config_change']) && is_callable($plugin['on_config_change'])) ($plugin['on_config_change'])($db, plugin_config($db, $id));
    $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    json_response(['ok' => true]);
}

function api_remove_plugin(PDO $db, array $input, string $action, bool $admin): never {
    $id = (string)($input['id'] ?? '');
    $plugin = plugin_list()[$id] ?? null;
    if (!$plugin) json_response(['error' => '插件不存在'], 404);
    $purge = !empty($input['purge']);
    $db->beginTransaction();
    try {
        $images = $purge ? plugin_delete_posts($db, $id) : [];
        $db->prepare('DELETE FROM plugin_tombstones WHERE plugin = ?')->execute([$id]);
        foreach ($plugin['fields'] ?? [] as $field) $db->prepare('DELETE FROM settings WHERE key = ?')->execute([plugin_setting_key($plugin, $field['key'])]);
        $prefix = "plugin:{$id}:";
        $db->prepare('DELETE FROM settings WHERE substr(key, 1, ?) = ?')->execute([strlen($prefix), $prefix]);
        save_setting($db, plugin_enabled_key($plugin), '0');
        save_setting($db, "plugin:{$id}:removed", '1');
        $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    foreach ($images as $ref) delete_image_file($ref);
    json_response(['ok' => true, 'purged' => $purge]);
}

function api_sync_plugin(PDO $db, array $input, string $action, bool $admin): never {
    $id = (string)($input['id'] ?? '');
    $plugin = plugin_list()[$id] ?? null;
    if (!$plugin) json_response(['error' => '插件不存在'], 404);
    if (!isset($plugin['sync'])) json_response(['error' => '插件不支持同步'], 422);
    $config = plugin_config($db, $id);
    if (empty($config['_enabled'])) json_response(['error' => '插件未启用'], 422);
    session_write_close();
    set_time_limit(0);
    $lock = @fopen(DATA_DIR . '/plugin.sync.lock', 'c');
    if ($lock && !flock($lock, LOCK_EX | LOCK_NB)) { fclose($lock); json_response(['error' => '同步正在进行，请稍后再试'], 409); }
    try {
        $items = ($plugin['sync'])($db, $config, ['all' => !empty($input['all'])]);
        $added = plugin_sync_items($db, $id, $items);
        if ($added > 0) auto_deploy_touch($db);
        if (isset($plugin['after_sync']) && is_callable($plugin['after_sync'])) ($plugin['after_sync'])($db, $items);
        save_setting($db, "plugin:{$id}:last_sync", (string)time());
        save_setting($db, "plugin:{$id}:last_error", '');
        json_response(['ok' => true, 'added' => $added]);
    } catch (Throwable $e) {
        save_setting($db, "plugin:{$id}:last_error", mb_substr($e->getMessage(), 0, 200));
        error_log($e->getMessage());
        json_response(['error' => '同步失败：' . $e->getMessage()], 500);
    } finally {
        if ($lock) { flock($lock, LOCK_UN); fclose($lock); }
    }
}


/** 清空某插件已同步的全部动态（含其上传图片与墓碑；保留插件配置与启用状态，便于清测试数据后重新同步） */
function api_purge_plugin_posts(PDO $db, array $input, string $action, bool $admin): never {
    $id = (string)($input['id'] ?? '');
    if (!isset(plugin_list()[$id])) json_response(['error' => '插件不存在'], 404);
    $db->beginTransaction();
    try {
        $images = plugin_delete_posts($db, $id);
        $db->prepare('DELETE FROM plugin_tombstones WHERE plugin = ?')->execute([$id]);
        $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    $files = 0;
    foreach ($images as $ref) {
        if (!str_starts_with($ref, 'up:')) continue;
        $path = DATA_DIR . '/uploads/' . substr($ref, 3);
        if (is_file($path)) {
            delete_image_file($ref);
            if (!is_file($path)) $files++;
        }
    }
    plugin_fire('plugin.posts_purged', ['plugin' => $id]);
    json_response(['ok' => true, 'deleted_files' => $files]);
}


/** 插件管理操作（仅管理员）：转发给插件的 admin_op 闭包，op/参数由插件自行解释 */
function api_plugin_admin(PDO $db, array $input, string $action, bool $admin): never {
    $id = (string)($input['id'] ?? '');
    $plugin = plugin_list()[$id] ?? null;
    if (!$plugin) json_response(['error' => '插件不存在'], 404);
    if (empty($plugin['admin_op']) || !is_callable($plugin['admin_op'])) json_response(['error' => '插件不支持管理操作'], 422);
    if (empty(plugin_config($db, $id)['_enabled'])) json_response(['error' => '插件未启用'], 422);
    $op = (string)($input['op'] ?? '');
    try {
        json_response(['ok' => true] + ($plugin['admin_op'])($db, $op, $input, plugin_config($db, $id)));
    } catch (InvalidArgumentException $e) {
        json_response(['error' => $e->getMessage()], 422);
    }
}
