<?php
declare(strict_types=1);

function api_tasks(PDO $db, bool $admin): never {
    if (!$admin) json_response(['error' => '请先登录管理后台'], 401);
    $settings = settings($db);
    $plugins = [];
    foreach (plugin_list() as $id => $plugin) {
        if (!isset($plugin['sync'])) continue;
        $config = plugin_config($db, $id);
        $interval = isset($plugin['interval_key']) ? (int)opt($plugin['interval_key']) : max(1, (int)($config['sync_minutes'] ?? 60)) * 60;
        $last = (int)($settings["plugin:$id:last_sync"] ?? 0);
        $plugins[] = ['id' => $id, 'name' => $plugin['name'] ?? $id, 'enabled' => (bool)$config['_enabled'], 'last_sync' => $last, 'next_sync' => $config['_enabled'] && $interval > 0 ? ($last ? $last + $interval : time()) : null, 'error' => $settings["plugin:$id:last_error"] ?? ''];
    }
    $backup = task_status_read('backup');
    $backup['running'] = task_running('backup');
    if (($backup['state'] ?? '') === 'running' && !$backup['running']) $backup['state'] = 'interrupted';
    $backup['enabled'] = opt('backup_enabled') === '1';
    $backup['interval_hours'] = (int)opt('backup_interval_hours');
    $backup['keep'] = (int)opt('backup_keep');
    $backup['next_run'] = $backup['enabled'] ? ((int)($backup['started_at'] ?? 0) ? (int)$backup['started_at'] + $backup['interval_hours'] * 3600 : time()) : null;
    $backup['directory'] = DATA_DIR . '/backups';
    try { $backup['files'] = scheduled_backup_files(); }
    catch (RuntimeException $e) { $backup['files'] = []; $backup['error'] = $e->getMessage(); }
    json_response(['worker' => ['running' => task_running('worker'), 'status' => task_status_read('worker')], 'sync_running' => task_running('plugin.sync'), 'plugins' => $plugins, 'deploy' => ['cancelling' => is_file(DATA_DIR . '/deploy.cancel'), 'running' => netlify_deploy_running(), 'status' => netlify_status_read(), 'pending' => auto_deploy_pending_read()], 'backup' => $backup]);
}

function api_save_backup_settings(PDO $db, array $input, string $action, bool $admin): never {
    $enabled = $input['enabled'] ?? null;
    $hours = filter_var($input['interval_hours'] ?? null, FILTER_VALIDATE_INT);
    $keep = filter_var($input['keep'] ?? null, FILTER_VALIDATE_INT);
    if (!in_array($enabled, [0, 1], true) || $hours === false || $hours < 1 || $hours > 720 || $keep === false || $keep < 1 || $keep > 30) json_response(['error' => '备份间隔需为 1–720 小时，保留数量需为 1–30 份'], 422);
    $db->beginTransaction();
    try {
        save_setting($db, 'backup_enabled', (string)$enabled);
        save_setting($db, 'backup_interval_hours', (string)$hours);
        save_setting($db, 'backup_keep', (string)$keep);
        $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    json_response(['ok' => true]);
}
